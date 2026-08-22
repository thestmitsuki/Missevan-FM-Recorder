//! 录制文件清理（§11.2 `run_cleanup_now` 命令 / 录制结束自动清理共用）
//!
//! 策略（规格 文件分类）：先按 mtime 删除 N 天前的旧文件；若总量仍超
//! `max_total_gb`，按最旧优先继续删除直到达标或文件清空。
//! `retention_days == 0` 表示不按天数清理；`max_total_gb == 0` 表示不限制总大小
//! （与 GlobalConfig 字段注释一致）。
//!
//! 触发方式：`run_cleanup_now` 命令（手动）与每次录制任务结束（monitor.rs
//! 统一出口，见 `cleanup_on_recording_end`）共用同一实现；原 cleanup_time
//! 每日定时调度（cleanup_scheduler）已移除。

use crate::domain::config::manager::ConfigManager;
use crate::domain::config::model::GlobalConfig;
use crate::domain::services::file_cache::{
    FileCacheHandle, FileCacheManager, OutputScan, RecordingFile,
};
use crate::infrastructure::error::types::AppError;
use crate::infrastructure::state::app_state::AppStateHandle;
use crate::tr;
use crate::tr_plural;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use tauri::WebviewWindow;

/// 候选清理文件条目
#[derive(Debug, Clone)]
pub struct CleanupCandidate {
    pub path: PathBuf,
    pub modified: SystemTime,
    pub size: u64,
}

/// 清理结果摘要（run_cleanup_now 返回给前端）
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CleanupSummary {
    pub files_deleted: usize,
    pub bytes_freed: u64,
    pub files_remaining: usize,
    pub bytes_remaining: u64,
}

/// 规划清理：返回应删除的文件路径列表（纯函数，便于单测）
///
/// 1. `retention_days > 0`：删除修改时间早于 N 天前的文件；
/// 2. `max_total_gb > 0` 且剩余总量超限：按最旧优先删除，直到总量达标或清空。
pub fn plan_cleanup(
    candidates: &[CleanupCandidate],
    retention_days: u32,
    max_total_gb: u64,
    now: SystemTime,
) -> Vec<PathBuf> {
    let mut to_delete = Vec::new();
    let mut remaining: Vec<CleanupCandidate> = Vec::new();

    // 阶段 1：按保留天数清理
    if retention_days > 0 {
        let cutoff = now - std::time::Duration::from_secs(retention_days as u64 * 86400);
        for c in candidates {
            if c.modified < cutoff {
                to_delete.push(c.path.clone());
            } else {
                remaining.push(c.clone());
            }
        }
    } else {
        remaining = candidates.to_vec();
    }

    // 阶段 2：按总量上限清理（最旧优先）
    if max_total_gb > 0 {
        let limit = (max_total_gb as u64).saturating_mul(1024 * 1024 * 1024);
        let total: u64 = remaining.iter().map(|c| c.size).sum();
        if total > limit {
            remaining.sort_by_key(|c| c.modified);
            let mut acc = total;
            for c in remaining {
                if acc <= limit {
                    break;
                }
                acc = acc.saturating_sub(c.size);
                to_delete.push(c.path.clone());
            }
        }
    }

    to_delete
}

/// 执行一次录制文件清理（`run_cleanup_now` 命令与录制结束触发共用）：
/// 单次扫描输出目录（O1 去双扫：遍历一次同时产出清理候选与缓存条目）→
/// 过滤录制中的文件 → plan_cleanup → 删除 → 基于同一扫描产物刷新文件缓存
///（内部 emit `recording_files_changed`，前端文件列表即时更新）。
/// 扫描与删除为同步阻塞 IO，整体放入 spawn_blocking，避免阻塞 tokio worker。
pub async fn run_cleanup(
    window: WebviewWindow,
    cache: FileCacheHandle,
    config_manager: Arc<ConfigManager>,
    app_state: AppStateHandle,
) -> Result<CleanupSummary, AppError> {
    let config = config_manager.load()?;
    let output_dir = Path::new(&config.global.output_dir).to_path_buf();
    // 录制中的文件跳过清理（FFmpeg 正在写入，删除会损坏录制）
    //（闭包 move 用克隆；原值后续传给 refresh_from_files 标记缓存活跃态）
    let active_paths = app_state.lock().await.active_output_paths();
    let active_paths_for_scan = active_paths.clone();
    let retention_days = config.global.retention_days;
    let max_total_gb = config.global.max_total_gb as u64;

    // O1 去双扫 + 阻塞消除：同步阻塞 IO（递归扫描 + 删除）整体放入
    // spawn_blocking——大目录/机械盘扫描与文件删除不再阻塞 tokio worker
    //（录制结束自动清理不卡 UI/其他 async 任务）。单次遍历同时产出清理候选
    // 与文件缓存条目（scan_output_once），删除后基于同一份扫描产物刷新缓存
    //（refresh_from_files），不再二次全量扫描——旧实现此处 scan 一次、refresh
    // 内部又 scan 一次，同一目录连续扫两遍。
    let (remaining_files, summary) = tauri::async_runtime::spawn_blocking(move || {
        let scan = FileCacheManager::scan_output_once(&output_dir).map_err(|e| {
            AppError::system(
                crate::infrastructure::error::types::IO_WRITE_FAIL,
                tr!("cleanup.read_dir_failed", path = output_dir.display()),
            )
            .with_technical(e.to_string())
        })?;
        // 清理候选 = 扫描产物中非活跃录制、且修改时间可读的文件
        // （is_active_path：非分段精确匹配 + 分段段文件 `{前缀}_NNN.{ext}` 前缀匹配）
        let candidates: Vec<CleanupCandidate> = scan
            .files
            .iter()
            .zip(scan.modified.iter())
            .filter(|(f, _)| {
                !crate::domain::services::file_cache::is_active_path(
                    &f.path,
                    &active_paths_for_scan,
                )
            })
            .filter_map(|(f, m)| {
                m.as_ref().map(|modified| CleanupCandidate {
                    path: PathBuf::from(&f.path),
                    modified: *modified,
                    size: f.size,
                })
            })
            .collect();
        let planned = plan_cleanup(
            &candidates,
            retention_days,
            max_total_gb,
            SystemTime::now(),
        );
        let to_delete: HashSet<&PathBuf> = planned.iter().collect();

        let mut files_deleted = 0usize;
        let mut bytes_freed = 0u64;
        // R1：记录「计划删除但删除失败」的文件——文件（可能被 Windows 文件锁/
        // 权限拒绝删除）仍留在磁盘，必须保留在本轮缓存刷新结果中，保证文件
        // 列表与磁盘实际状态一致（不再短暂"消失"，下次 refresh/手动清理可重试）
        let mut delete_failed: HashSet<PathBuf> = HashSet::new();
        for path in &to_delete {
            // S1 纵深防御：删除前用 symlink_metadata（不跟随链接）复核——扫描侧
            // 已拒绝链接项，此处兜底（如扫描与删除之间条目被替换成链接），
            // 链接项一律跳过不删，杜绝触碰输出目录外的任何文件
            let Ok(meta) = std::fs::symlink_metadata(path) else {
                tracing::warn!(
                    "{}",
                    tr!("cleanup.meta_read_failed", path = format!("{:?}", path))
                );
                continue;
            };
            if meta.file_type().is_symlink() {
                tracing::warn!(
                    "{}",
                    tr!("cleanup.skip_symlink", path = format!("{:?}", path))
                );
                continue;
            }
            match std::fs::remove_file(path) {
                Ok(_) => {
                    files_deleted += 1;
                    bytes_freed += meta.len();
                }
                Err(e) => {
                    // Windows 文件锁（ffmpeg 仍持有句柄）/权限不足等：文件仍留在
                    // 磁盘。记 warn 并把文件保留在缓存中（partition_cleanup_remainder），
                    // 下次 refresh 或手动清理可再次尝试
                    tracing::warn!(
                        "{}",
                        tr!(
                            "cleanup.delete_failed_keep",
                            path = format!("{:?}", path),
                            err = e
                        )
                    );
                    delete_failed.insert((*path).clone());
                }
            }
        }
        // 剩余 = 未计划删除 + 计划删除但删除失败（R1）；同一份扫描产物派生出
        // 缓存剩余文件列表与剩余统计
        let (remaining_files, files_remaining, bytes_remaining) =
            partition_cleanup_remainder(scan, &candidates, &to_delete, &delete_failed);
        tracing::info!(
            "{}",
            tr_plural!(
                "cleanup.files_removed",
                files_deleted as u64,
                bytes = bytes_freed
            )
        );
        if !delete_failed.is_empty() {
            tracing::warn!(
                "{}",
                tr_plural!(
                    "cleanup.delete_failed_count",
                    delete_failed.len() as u64,
                    paths = format!("{:?}", delete_failed)
                )
            );
        }

        Ok::<_, AppError>((
            remaining_files,
            CleanupSummary {
                files_deleted,
                bytes_freed,
                files_remaining,
                bytes_remaining,
            },
        ))
    })
    .await
    .map_err(|e| AppError::internal(tr!("cleanup.task_failed", err = e)))??;

    // 基于同一扫描结果刷新文件缓存（内部 emit recording_files_changed）
    let manager = FileCacheManager::new(window, cache);
    manager.refresh_from_files(remaining_files, active_paths).await?;

    Ok(summary)
}

/// 计算清理后「剩余」集合（R1）：未计划删除的文件 + 计划删除但删除失败
/// （文件仍在磁盘）的文件，都保留在刷新后的缓存与剩余统计中——文件列表与
/// 磁盘实际状态一致，删除失败的文件不会从列表中"消失"（下次 refresh 或
/// 手动清理可再次尝试删除）。
///
/// 返回（刷新缓存的剩余文件列表, files_remaining, bytes_remaining）。
/// 注：remaining_files 基于扫描产物（全部录制扩展名文件）；files/bytes_remaining
/// 基于清理候选（修改时间可读的文件）——与既有语义一致。
fn partition_cleanup_remainder(
    scan: OutputScan,
    candidates: &[CleanupCandidate],
    to_delete: &HashSet<&PathBuf>,
    delete_failed: &HashSet<PathBuf>,
) -> (Vec<RecordingFile>, usize, u64) {
    let remaining_files: Vec<RecordingFile> = scan
        .files
        .into_iter()
        .filter(|f| {
            let p = PathBuf::from(&f.path);
            !to_delete.contains(&p) || delete_failed.contains(&p)
        })
        .collect();
    let mut files_remaining = 0usize;
    let mut bytes_remaining = 0u64;
    for c in candidates {
        if !to_delete.contains(&c.path) || delete_failed.contains(&c.path) {
            files_remaining += 1;
            bytes_remaining += c.size;
        }
    }
    (remaining_files, files_remaining, bytes_remaining)
}

/// 是否启用自动清理（纯函数）：仅 `auto_cleanup_enabled` 为 true 时在每次
/// 录制任务结束时执行清理。手动 `run_cleanup_now` 命令不受此开关限制。
pub fn should_cleanup(config: &GlobalConfig) -> bool {
    config.auto_cleanup_enabled
}

/// 录制结束自动清理入口（每次录制任务结束时触发一次，替代原 cleanup_time
/// 每日定时调度）：读**最新**配置（而非录制启动时的快照），`auto_cleanup_enabled`
/// 时执行与 `run_cleanup_now` 命令完全相同的清理（内部刷新文件缓存并 emit
/// `recording_files_changed`，前端文件列表即时更新）。失败仅记 warn，不阻断
/// 录制结束流程。
pub async fn cleanup_on_recording_end(
    window: WebviewWindow,
    cache: FileCacheHandle,
    config_manager: Arc<ConfigManager>,
    app_state: AppStateHandle,
) {
    let config = match config_manager.load() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("{}", tr!("cleanup.end_config_load_failed", err = e));
            return;
        }
    };
    if !should_cleanup(&config.global) {
        return;
    }
    match run_cleanup(window, cache, config_manager, app_state).await {
        Ok(summary) => tracing::info!(
            "{}",
            tr_plural!(
                "cleanup.end_auto_done",
                summary.files_deleted as u64,
                bytes = summary.bytes_freed
            )
        ),
        Err(e) => tracing::warn!("{}", tr!("cleanup.end_auto_failed", err = e)),
    }
}

/// 递归扫描输出目录下的录制文件（扩展名与文件缓存一致）
///
/// O1：候选基于 `FileCacheManager::scan_output_once` 单次遍历产物构建——
/// 清理与文件缓存共享同一份扫描结果，避免重复全量遍历（清理路径只扫一遍）。
/// S1：遍历统一走 `fs_walk::safe_walk_files`（条目经 symlink_metadata 不
/// 跟随链接判定，符号链接 / Windows junction 一律跳过），输出目录内的链接
/// 不会把扫描带出目录外——杜绝自动清理误删目录外音频文件（数据丢失）。
///
/// 生产路径（run_cleanup）为去双扫已内联单次扫描（scan_output_once），本函数
/// 保留作为「清理候选视图」的独立入口，主要被单测覆盖（含链接/junction 安全
/// 回归）；不参与生产调用故加 allow(dead_code) 避免误报。
#[cfg_attr(not(test), allow(dead_code))]
pub fn scan_recording_files(root: &Path) -> Result<Vec<CleanupCandidate>, AppError> {
    // 根目录读取失败透传为 AppError（与旧实现 read_dir 错误直接上抛一致）；
    // 子树读取失败由 safe_walk_files 跳过，不中断整次扫描；modified 获取
    // 失败的文件跳过（与旧实现 metadata().modified() 失败跳过一致）
    let scan = FileCacheManager::scan_output_once(root).map_err(|e| {
        AppError::system(
            crate::infrastructure::error::types::IO_WRITE_FAIL,
            tr!("cleanup.read_dir_failed", path = root.display()),
        )
        .with_technical(e.to_string())
    })?;
    Ok(scan
        .files
        .iter()
        .zip(scan.modified.iter())
        .filter_map(|(f, m)| {
            m.as_ref().map(|modified| CleanupCandidate {
                path: PathBuf::from(&f.path),
                modified: *modified,
                size: f.size,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn entry(name: &str, age_secs: u64, size: u64) -> CleanupCandidate {
        CleanupCandidate {
            path: PathBuf::from(format!("/recordings/{}", name)),
            modified: SystemTime::now() - Duration::from_secs(age_secs),
            size,
        }
    }

    fn paths(to_delete: &[PathBuf]) -> Vec<String> {
        let mut v: Vec<String> = to_delete
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn retention_deletes_only_files_older_than_days() {
        let now = SystemTime::now();
        let candidates = vec![
            entry("old1.m4a", 90 * 86400, 10),
            entry("old2.aac", 40 * 86400, 20),
            entry("new1.m4a", 5 * 86400, 30),
            entry("new2.mp3", 86400, 40),
        ];
        let to_delete = plan_cleanup(&candidates, 30, 0, now);
        assert_eq!(paths(&to_delete), vec!["old1.m4a", "old2.aac"]);
    }

    #[test]
    fn retention_zero_disables_time_based_cleanup() {
        let now = SystemTime::now();
        let candidates = vec![entry("ancient.m4a", 3650 * 86400, 10)];
        let to_delete = plan_cleanup(&candidates, 0, 0, now);
        assert!(to_delete.is_empty());
    }

    const GB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn max_total_deletes_oldest_until_under_limit() {
        let now = SystemTime::now();
        let candidates = vec![
            entry("old.m4a", 30 * 86400, 40 * GB),
            entry("mid.m4a", 20 * 86400, 40 * GB),
            entry("new.m4a", 10 * 86400, 40 * GB),
        ];
        // 总量 120GB > 100GB 上限：删除最旧 1 个（40GB）后总量 80GB 达标
        let to_delete = plan_cleanup(&candidates, 0, 100, now);
        assert_eq!(paths(&to_delete), vec!["old.m4a"]);
    }

    #[test]
    fn max_total_deletes_all_when_every_file_exceeds_limit() {
        let now = SystemTime::now();
        let candidates = vec![
            entry("huge.m4a", 10 * 86400, 5 * GB),
            entry("other.m4a", 5 * 86400, 2 * GB),
        ];
        // 两个文件都超 1GB 上限：删掉最旧后仍超限 → 继续删，直到文件清空
        let to_delete = plan_cleanup(&candidates, 0, 1, now);
        assert_eq!(paths(&to_delete), vec!["huge.m4a", "other.m4a"]);
    }

    #[test]
    fn combined_rules_apply_retention_then_size() {
        let now = SystemTime::now();
        let candidates = vec![
            entry("old.m4a", 90 * 86400, 5 * GB),   // 超期 → 删
            entry("old2.m4a", 31 * 86400, 5 * GB),  // 超期 → 删
            entry("recent.m4a", 3 * 86400, 60 * GB), // 未超期，但超总量 → 删（最旧）
            entry("new.m4a", 1 * 86400, 60 * GB),
        ];
        // retention 30 天删 2 个；剩余 120GB > 100GB → 再删最旧 recent
        let to_delete = plan_cleanup(&candidates, 30, 100, now);
        assert_eq!(paths(&to_delete), vec!["old.m4a", "old2.m4a", "recent.m4a"]);
    }

    #[test]
    fn both_rules_disabled_deletes_nothing() {
        let now = SystemTime::now();
        let candidates = vec![entry("x.m4a", 100 * 86400, 999)];
        assert!(plan_cleanup(&candidates, 0, 0, now).is_empty());
    }

    #[test]
    fn should_cleanup_reflects_toggle() {
        let mut cfg = GlobalConfig::default();
        cfg.auto_cleanup_enabled = false;
        assert!(!should_cleanup(&cfg), "关闭自动清理 → 录制结束不清理");
        cfg.auto_cleanup_enabled = true;
        assert!(should_cleanup(&cfg), "开启自动清理 → 录制结束执行清理");
    }

    #[test]
    fn scan_finds_recording_files_recursively_ignores_others() {
        let dir = std::env::temp_dir().join(format!("missevan-test-cleanup-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.m4a"), b"x").unwrap();
        std::fs::write(dir.join("b.MP3"), b"x").unwrap();
        std::fs::write(dir.join("sub/c.flac"), b"x").unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        std::fs::write(dir.join("noext"), b"x").unwrap();

        let found = scan_recording_files(&dir).unwrap();
        let mut names: Vec<String> = found
            .iter()
            .map(|c| c.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.m4a", "b.MP3", "c.flac"]); // 大小写不敏感，忽略 txt/无扩展名

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_missing_dir_returns_empty() {
        let dir = std::env::temp_dir().join("missevan-test-cleanup-missing");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(scan_recording_files(&dir).unwrap().is_empty());
    }

    // ── R1：删除失败的文件保留在缓存中（列表与磁盘一致）──

    /// 构造扫描产物中的一条文件缓存条目
    fn cache_file(name: &str, size: u64) -> RecordingFile {
        RecordingFile {
            id: name.to_string(),
            name: name.to_string(),
            path: format!("/recordings/{}", name),
            size,
            duration: 0.0,
            anchor_name: String::new(),
            created_at: SystemTime::now(),
            group_prefix: None,
            segment_index: None,
            is_active: false,
        }
    }

    #[test]
    fn delete_failed_files_remain_in_cache_and_summary() {
        // R1 回归：删除失败（Windows 文件锁/权限）的文件仍在磁盘，必须保留在
        // 刷新后的缓存与剩余统计中——文件列表与磁盘一致，不短暂"消失"
        let scan = OutputScan {
            files: vec![
                cache_file("a.m4a", 10),
                cache_file("b.m4a", 20),
                cache_file("c.m4a", 30),
            ],
            modified: vec![
                Some(SystemTime::now()),
                Some(SystemTime::now()),
                Some(SystemTime::now()),
            ],
        };
        let mk = |name: &str, size: u64| CleanupCandidate {
            path: PathBuf::from(format!("/recordings/{}", name)),
            modified: SystemTime::now(),
            size,
        };
        let candidates = vec![mk("a.m4a", 10), mk("b.m4a", 20), mk("c.m4a", 30)];
        // 计划删除 a、b；其中 b 删除失败（仍在磁盘）→ 保留；a 删除成功 → 移除
        let mut to_delete: HashSet<&PathBuf> = HashSet::new();
        to_delete.insert(&candidates[0].path);
        to_delete.insert(&candidates[1].path);
        let mut delete_failed: HashSet<PathBuf> = HashSet::new();
        delete_failed.insert(candidates[1].path.clone());

        let (remaining, files_remaining, bytes_remaining) =
            partition_cleanup_remainder(scan, &candidates, &to_delete, &delete_failed);

        let mut names: Vec<String> = remaining.iter().map(|f| f.name.clone()).collect();
        names.sort();
        assert_eq!(
            names,
            vec!["b.m4a", "c.m4a"],
            "删除失败的文件必须保留在缓存中（a 已删除、不保留）"
        );
        assert_eq!(files_remaining, 2, "剩余统计应含删除失败文件");
        assert_eq!(bytes_remaining, 20 + 30, "剩余字节应含删除失败文件");
    }

    #[test]
    fn all_planned_deleted_removes_all_from_cache() {
        // 全部删除成功：剩余集合与旧行为一致（无删除失败时无多余保留）
        let scan = OutputScan {
            files: vec![cache_file("a.m4a", 10), cache_file("b.m4a", 20)],
            modified: vec![Some(SystemTime::now()), Some(SystemTime::now())],
        };
        let mk = |name: &str, size: u64| CleanupCandidate {
            path: PathBuf::from(format!("/recordings/{}", name)),
            modified: SystemTime::now(),
            size,
        };
        let candidates = vec![mk("a.m4a", 10), mk("b.m4a", 20)];
        let mut to_delete: HashSet<&PathBuf> = HashSet::new();
        to_delete.insert(&candidates[0].path);
        to_delete.insert(&candidates[1].path);
        let delete_failed: HashSet<PathBuf> = HashSet::new();

        let (remaining, files_remaining, bytes_remaining) =
            partition_cleanup_remainder(scan, &candidates, &to_delete, &delete_failed);
        assert!(remaining.is_empty(), "全部删除成功 → 缓存无剩余");
        assert_eq!(files_remaining, 0);
        assert_eq!(bytes_remaining, 0);
    }

    #[test]
    fn scan_does_not_follow_symlink_or_junction_outside_root() {
        // M7 回归：输出目录内的链接（指向目录外位置）不得被扫描——否则清理
        // 服务会把目录外 .m4a 纳入删除候选
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "missevan-test-cleanup-link-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let outside = std::env::temp_dir().join(format!(
            "missevan-test-cleanup-outside-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.m4a"), b"x").unwrap();
        std::fs::write(root.join("real.m4a"), b"x").unwrap();
        // 创建目录链接；无权限（Windows 非开发者模式）时跳过本测试
        #[cfg(unix)]
        let link_ok = std::os::unix::fs::symlink(&outside, root.join("linked")).is_ok();
        #[cfg(windows)]
        let link_ok = std::os::windows::fs::symlink_dir(&outside, root.join("linked")).is_ok();
        if !link_ok {
            let _ = std::fs::remove_dir_all(&root);
            let _ = std::fs::remove_dir_all(&outside);
            return;
        }
        let found = scan_recording_files(&root).unwrap();
        let names: Vec<String> = found
            .iter()
            .map(|c| c.path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![root.join("real.m4a").to_string_lossy().into_owned()],
            "链接指向的目录外文件不得进入扫描结果");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// S1 回归（Windows）：junction（`mklink /J`，无需管理员/开发者模式）
    /// 指向输出目录外，`scan_recording_files` 不得把目录外 .m4a 纳入清理
    /// 候选——否则自动清理会删除输出目录外文件（数据丢失）。
    #[cfg(windows)]
    #[test]
    fn scan_skips_windows_junction_pointing_outside() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "missevan-test-cleanup-junc-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let outside = std::env::temp_dir().join(format!(
            "missevan-test-cleanup-junc-out-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.m4a"), b"x").unwrap();
        std::fs::write(root.join("real.m4a"), b"x").unwrap();
        let ok = std::process::Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                root.join("linked").to_str().unwrap(),
                outside.to_str().unwrap(),
            ])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            let _ = std::fs::remove_dir_all(&root);
            let _ = std::fs::remove_dir_all(&outside);
            return;
        }
        let found = scan_recording_files(&root).unwrap();
        let names: Vec<String> = found
            .iter()
            .map(|c| c.path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![root.join("real.m4a").to_string_lossy().into_owned()],
            "junction 指向的目录外 .m4a 不得进入清理候选（数据丢失风险）");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }
}
