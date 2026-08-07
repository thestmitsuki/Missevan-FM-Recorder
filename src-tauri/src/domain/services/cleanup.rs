//! 录制文件清理（§11.2 `run_cleanup_now` / 后续定时清理共用）
//!
//! 策略（规格 文件分类）：先按 mtime 删除 N 天前的旧文件；若总量仍超
//! `max_total_gb`，按最旧优先继续删除直到达标或文件清空。
//! `retention_days == 0` 表示不按天数清理；`max_total_gb == 0` 表示不限制总大小
//! （与 GlobalConfig 字段注释一致）。

use crate::domain::config::manager::ConfigManager;
use crate::domain::services::file_cache::{FileCacheHandle, FileCacheManager};
use crate::infrastructure::error::types::AppError;
use crate::infrastructure::state::app_state::AppStateHandle;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use tauri::WebviewWindow;

/// 与文件缓存一致的录制文件扩展名
const RECORDING_EXTENSIONS: [&str; 4] = ["m4a", "aac", "mp3", "flac"];

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

/// 执行一次录制文件清理（`run_cleanup_now` 命令与定时调度共用）：
/// 扫描输出目录 → 过滤录制中的文件 → plan_cleanup → 删除 → 刷新文件缓存
///（内部 emit `recording_files_changed`，前端文件列表即时更新）。
pub async fn run_cleanup(
    window: WebviewWindow,
    cache: FileCacheHandle,
    config_manager: Arc<ConfigManager>,
    app_state: AppStateHandle,
) -> Result<CleanupSummary, AppError> {
    let config = config_manager.load()?;
    let output_dir = Path::new(&config.global.output_dir);
    // 录制中的文件跳过清理（FFmpeg 正在写入，删除会损坏录制）
    let active_paths = app_state.lock().await.active_output_paths();
    let candidates: Vec<CleanupCandidate> = scan_recording_files(output_dir)?
        .into_iter()
        .filter(|c| {
            !active_paths.contains(&crate::domain::services::file_cache::path_key(
                &c.path.to_string_lossy(),
            ))
        })
        .collect();
    let planned = plan_cleanup(
        &candidates,
        config.global.retention_days,
        config.global.max_total_gb as u64,
        SystemTime::now(),
    );
    let to_delete: std::collections::HashSet<&PathBuf> = planned.iter().collect();

    let mut files_deleted = 0usize;
    let mut bytes_freed = 0u64;
    for path in &to_delete {
        if let Ok(meta) = std::fs::metadata(path) {
            bytes_freed += meta.len();
        }
        match std::fs::remove_file(path) {
            Ok(_) => files_deleted += 1,
            Err(e) => tracing::warn!("清理失败 {:?}: {}", path, e),
        }
    }
    let mut files_remaining = 0usize;
    let mut bytes_remaining = 0u64;
    for c in &candidates {
        if !to_delete.contains(&c.path) {
            files_remaining += 1;
            bytes_remaining += c.size;
        }
    }
    tracing::info!(
        "录制文件清理完成: 删除 {} 个文件 / 释放 {} 字节",
        files_deleted,
        bytes_freed
    );

    // 刷新文件缓存（内部 emit recording_files_changed）
    let manager = FileCacheManager::new(window, cache);
    manager.refresh(&config_manager, &app_state).await?;

    Ok(CleanupSummary {
        files_deleted,
        bytes_freed,
        files_remaining,
        bytes_remaining,
    })
}

/// 递归扫描输出目录下的录制文件（扩展名与文件缓存一致）
pub fn scan_recording_files(root: &Path) -> Result<Vec<CleanupCandidate>, AppError> {
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = std::fs::read_dir(&dir).map_err(|e| {
            AppError::system(
                crate::infrastructure::error::types::IO_WRITE_FAIL,
                format!("读取目录失败: {}", dir.display()),
            )
            .with_technical(e.to_string())
        })?;
        for entry in rd.flatten() {
            let path = entry.path();
            // M7：用 DirEntry::file_type()（不跟随链接）判定，拒绝 junction/
            // 符号链接项——避免扫描/清理越出输出目录（指向目录外位置的链接
            // 会暴露并删除目录外 .m4a 文件）
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() {
                let is_recording = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| RECORDING_EXTENSIONS.contains(&e.to_lowercase().as_str()))
                    .unwrap_or(false);
                if is_recording {
                    if let Ok(meta) = path.metadata() {
                        if let Ok(modified) = meta.modified() {
                            out.push(CleanupCandidate {
                                path,
                                modified,
                                size: meta.len(),
                            });
                        }
                    }
                }
            }
        }
    }
    Ok(out)
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
}
