//! 设置向导相关 Tauri 命令（Task 7）
//!
//! - `download_ffmpeg`：流式下载 FFmpeg 便携版到 `{exe_dir}/ffmpeg/`（仅 Windows，
//!   gyan.dev zip），emit `download:progress { percent, stage }` 事件，解压后重新触发
//!   FfmpegCheck 并返回下载路径（修复子代理 B：**不写配置**——路径由前端暂存，
//!   向导最后一步「完成」按钮时随 save_config 统一落盘）；Linux 不自动下载
//!   （移植决策 #3），直接返回错误提示用系统包安装（Arch Linux：`sudo pacman -S ffmpeg`）
//! - `exit_app`：退出应用
//! - `finish_wizard`：向导完成（关闭向导窗 / 显示聚焦主窗 / 刷新文件缓存 / 唤醒检测循环）
//!
//! 注意：`#[tauri::command]` 会生成 `__cmd__xxx` 宏导入，其可见性跟随函数；
//! 命令须为 `pub(crate)` 才能被 lib.rs 根模块的 `generate_handler!` 通过全路径引用
//! （Task 6 在根模块用非 pub 骨架可行，是因为宏与调用方同模块；E0255/E0603 已知坑）。

#[cfg(windows)]
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(windows)]
use futures_util::StreamExt;
// Emitter（window.emit）仅 Windows 下载进度事件使用；Manager / State 跨平台
#[cfg(windows)]
use tauri::Emitter;
use tauri::{Manager, State};

use crate::domain::config::manager::ConfigManager;
use crate::domain::services::file_cache::{FileCacheHandle, FileCacheManager};
use crate::infrastructure::checker::checks::{DiskSpaceCheck, FfmpegCheck, HealthCheck};
use crate::infrastructure::checker::report::{CheckResult, CheckStatus, DiagnosticReport};
use crate::infrastructure::state::app_state::RecorderState;
// IO_WRITE_FAIL 仅 Windows 下载/解压路径使用（Linux 分支直接返回错误）
#[cfg(windows)]
use crate::infrastructure::error::types::IO_WRITE_FAIL;
use crate::infrastructure::error::types::AppError;
use crate::tr;

/// FFmpeg 下载源（gyan.dev 官方构建，内含 ffmpeg.exe / ffprobe.exe；仅 Windows：
/// 该源只有 Windows 构建，Linux 由用户安装系统包）
#[cfg(windows)]
const FFMPEG_DOWNLOAD_URL: &str = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip";
/// 需要从 zip 中提取的可执行文件名（不含路径，匹配 zip 内任意子目录层级；
/// 仅 Windows：gyan.dev zip 只含 .exe 构建）
#[cfg(windows)]
const FFMPEG_ZIP_TARGETS: [&str; 2] = ["ffmpeg.exe", "ffprobe.exe"];

/// FFmpeg zip 的期望 SHA256（小写十六进制；当前为**占位 None**）。
///
/// 尚未确定如何获取 gyan.dev 官方构建的 SHA256 清单，先跳过校验保持原有
/// 下载逻辑；确定来源后填入哈希即自动启用强校验——下载损坏 / 被篡改的
/// zip 在解压前被拦截（校验失败返回错误并清理临时文件）。
#[cfg(windows)]
const FFMPEG_ZIP_SHA256: Option<&str> = None;

/// 校验下载的 FFmpeg zip 的 SHA256（若已配置期望值）。
/// 期望值未配置（占位）时直接通过，不改变原有下载/解压流程。
#[cfg(windows)]
fn verify_ffmpeg_zip_sha256(zip_path: &std::path::Path) -> Result<(), AppError> {
    let Some(expected) = FFMPEG_ZIP_SHA256 else {
        tracing::debug!("{}", tr!("wizard.sha256_not_configured"));
        return Ok(());
    };
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(zip_path).map_err(|e| {
        AppError::system(IO_WRITE_FAIL, tr!("wizard.open_zip_failed")).with_technical(e.to_string())
    })?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| {
        AppError::system(IO_WRITE_FAIL, tr!("wizard.compute_sha256_failed")).with_technical(e.to_string())
    })?;
    let actual = hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(AppError::system(IO_WRITE_FAIL, tr!("wizard.sha256_mismatch"))
            .with_technical(format!("期望 {}，实际 {}", expected, actual))
            .with_suggestion(tr!("wizard.manual_download_hint")));
    }
    tracing::info!("{}", tr!("wizard.sha256_passed", actual = actual));
    Ok(())
}

/// 执行 `{exe} -version`，成功时返回首行版本信息
async fn probe_tool_version(exe: &std::path::Path) -> Option<String> {
    // 隐藏控制台（tools.rs::apply_create_no_window）：首启向导环境检查 / 下载后
    // 验证会 spawn ffmpeg/ffprobe（控制台子系统），发布构建无控制台时会弹黑窗口
    let mut probe = tokio::process::Command::new(exe);
    probe.arg("-version");
    #[cfg(windows)]
    crate::domain::tools::apply_create_no_window(probe.as_std_mut());
    match probe.output().await {
        Ok(out) if out.status.success() => {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .next()
                .map(|s| s.trim().to_string())
        }
        _ => None,
    }
}

/// 工具存在性检查：依次尝试候选可执行文件，命中即返回"可用 + 版本号"，
/// 全部失败返回"未找到"。
async fn tool_check(name: &str, candidates: &[std::path::PathBuf]) -> CheckResult {
    let start = std::time::Instant::now();
    for cand in candidates {
        if let Some(version) = probe_tool_version(cand).await {
            return CheckResult {
                check_name: name.to_string(),
                status: CheckStatus::Passed,
                message: tr!("wizard.tool_available", name = name, version = version),
                details: Some(tr!("wizard.tool_path", path = cand.display())),
                suggestion: None,
                duration_ms: start.elapsed().as_millis() as u64,
            };
        }
    }
    CheckResult {
        check_name: name.to_string(),
        status: CheckStatus::Failed,
        message: tr!("wizard.tool_not_found", name = name),
        details: None,
        // Linux 不自动下载（决策 #3）：提示系统包安装命令；Windows 走「下载并安装」按钮
        suggestion: Some(if cfg!(target_os = "linux") {
            tr!("wizard.linux_install_hint").to_string()
        } else {
            tr!("wizard.download_install_hint").to_string()
        }),
        duration_ms: start.elapsed().as_millis() as u64,
    }
}

/// 输出目录写入权限检查：幂等创建目录，写入并删除临时探测文件
async fn write_permission_check(output_dir: &str) -> CheckResult {
    let start = std::time::Instant::now();
    let dir = std::path::Path::new(output_dir);
    let probe = dir.join(format!(".missevan-write-test-{}", std::process::id()));

    let result: Result<(), String> = (|| {
        std::fs::create_dir_all(dir).map_err(|e| tr!("wizard.create_dir_failed", err = e))?;
        std::fs::write(&probe, b"ok").map_err(|e| tr!("wizard.write_test_failed", err = e))?;
        std::fs::remove_file(&probe).map_err(|e| tr!("wizard.delete_test_failed", err = e))?;
        Ok(())
    })();

    match result {
        Ok(()) => CheckResult {
            check_name: tr!("wizard.output_dir_write_permission").to_string(),
            status: CheckStatus::Passed,
            message: tr!("wizard.output_dir_writable").to_string(),
            details: Some(tr!("wizard.output_dir_path", path = output_dir)),
            suggestion: None,
            duration_ms: start.elapsed().as_millis() as u64,
        },
        Err(msg) => CheckResult {
            check_name: tr!("wizard.output_dir_write_permission").to_string(),
            status: CheckStatus::Failed,
            message: tr!("wizard.no_write_permission", err = msg),
            details: None,
            suggestion: Some(tr!("wizard.change_output_dir_hint").to_string()),
            duration_ms: start.elapsed().as_millis() as u64,
        },
    }
}

/// 向导环境检查：基于「第二页暂存的配置」并发检测 FFmpeg / ffprobe / 磁盘空间 / 写入权限。
///
/// 与 `run_health_check`（读取已落盘的全局配置）不同，本命令直接接收暂存的
/// 输出目录与磁盘阈值，向导流程结束前不会写盘（规格：第三页通过后才持久化）。
/// FFmpeg/ffprobe 候选顺序：配置指定路径 → `{exe_dir}/ffmpeg/`（上次下载结果）→ PATH。
#[tauri::command]
pub(crate) async fn run_wizard_health_check(
    output_dir: String,
    disk_threshold_gb: u64,
    config_manager: State<'_, Arc<ConfigManager>>,
) -> Result<DiagnosticReport, AppError> {
    // 1. 收集候选可执行文件路径（候选顺序：配置指定（若非空）→ `{exe_dir}/ffmpeg/`（上次下载结果）→ PATH，
    //    与 domain::tools::resolve_ffmpeg_executable 的语义一致）
    let (ffmpeg_candidates, ffprobe_candidates) = match config_manager.load() {
        Ok(cfg) => (
            crate::domain::tools::ffmpeg_candidates(cfg.global.ffmpeg_path.as_deref()),
            crate::domain::tools::ffprobe_candidates(&cfg.global.ffprobe_path),
        ),
        Err(_) => (
            crate::domain::tools::ffmpeg_candidates(None),
            crate::domain::tools::ffprobe_candidates(""),
        ),
    };

    // 2. 并发执行四项检查（规格：并发缩短等待）
    let disk_check = DiskSpaceCheck {
        output_dir: output_dir.clone(),
        threshold_gb: disk_threshold_gb,
    };
    let (ffmpeg, ffprobe, disk, write) = tokio::join!(
        tool_check("FFmpeg", &ffmpeg_candidates),
        tool_check("ffprobe", &ffprobe_candidates),
        disk_check.run(),
        write_permission_check(&output_dir),
    );

    Ok(DiagnosticReport::new(vec![ffmpeg, ffprobe, disk, write]))
}

/// `download:progress` 事件载荷（Task 8 向导 UI 依赖；仅 Windows 下载路径使用）
#[cfg(windows)]
#[derive(Debug, Clone, serde::Serialize)]
struct DownloadProgress {
    /// 下载进度百分比（0-100；解压/验证阶段沿用上一值）
    percent: u8,
    /// 阶段：connecting / downloading / extracting / verifying / done
    stage: &'static str,
}

/// `download_ffmpeg` 命令返回值（修复子代理 B）。
///
/// 下载成功**不再写配置**（配置文件唯一写入点在向导最后一步「完成」按钮）——
/// ffmpeg/ffprobe 绝对路径随返回值交给前端暂存（wizardStore.staged），完成时
/// 经 stagedToConfigPatch 一并写入 config.toml。
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct DownloadFfmpegResult {
    /// 下载完成后重新触发的 FFmpeg 检查结果
    pub check: CheckResult,
    /// 下载的 ffmpeg.exe 绝对路径（null = 未下载/自动探测）
    pub ffmpeg_path: Option<String>,
    /// 下载的 ffprobe.exe 绝对路径（空串 = 自动探测）
    pub ffprobe_path: String,
}

#[cfg(windows)]
impl DownloadProgress {
    fn emit(window: &tauri::Window, percent: u8, stage: &'static str) {
        let _ = window.emit(
            "download:progress",
            DownloadProgress {
                percent,
                stage,
            },
        );
    }
}

/// RAII 临时文件清理 guard：析构时删除持有的路径。
///
/// 覆盖所有失败路径（下载中断、写入失败、解压失败等）的临时文件残留；
/// 删除失败只记 warning 日志，绝不掩盖调用方的主错误（文件不存在视为成功）。
/// 仅 Windows 下载/解压路径使用。
#[cfg(windows)]
struct TempFileGuard(PathBuf);

#[cfg(windows)]
impl TempFileGuard {
    fn new(path: PathBuf) -> Self {
        Self(path)
    }
}

#[cfg(windows)]
impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.0) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("{}", tr!("wizard.cleanup_temp_failed", path = format!("{:?}", self.0), err = e));
            }
        }
    }
}

/// 将 zip 条目名映射为扁平化输出文件名。
///
/// 只提取 `ffmpeg.exe` / `ffprobe.exe`（忽略条目在 zip 内的子目录层级，
/// 如 `ffmpeg-7.1-essentials_build/bin/ffmpeg.exe`），其余条目返回 None。
/// 仅 Windows：gyan.dev zip 内的可执行文件均带 .exe 扩展名。
#[cfg(windows)]
fn flat_output_name(entry_name: &str) -> Option<&str> {
    let file_name = entry_name.rsplit(['/', '\\']).next().unwrap_or(entry_name);
    if FFMPEG_ZIP_TARGETS.contains(&file_name) {
        Some(file_name)
    } else {
        None
    }
}

/// 解压 zip 中需要的 ffmpeg/ffprobe 可执行文件到目标目录（扁平化）。
///
/// 同步阻塞 IO，调用方应放入 `spawn_blocking`。仅 Windows。
#[cfg(windows)]
fn extract_ffmpeg_zip(zip_path: &Path, target_dir: &Path) -> Result<Vec<PathBuf>, AppError> {
    std::fs::create_dir_all(target_dir).map_err(|e| {
        AppError::system(IO_WRITE_FAIL, tr!("wizard.create_ffmpeg_dir_failed")).with_technical(e.to_string())
    })?;

    let file = std::fs::File::open(zip_path).map_err(|e| {
        AppError::system(IO_WRITE_FAIL, tr!("wizard.open_zip_failed")).with_technical(e.to_string())
    })?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| {
            AppError::system(IO_WRITE_FAIL, tr!("wizard.zip_corrupt")).with_technical(e.to_string())
        })?;

    let mut extracted = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| {
            AppError::system(IO_WRITE_FAIL, tr!("wizard.read_zip_entry_failed")).with_technical(e.to_string())
        })?;
        if !entry.is_file() {
            continue;
        }
        let Some(out_name) = flat_output_name(entry.name()) else {
            continue;
        };
        let out_path = target_dir.join(out_name);
        // 原子替换：先完整写入 `<name>.tmp`，写完再 rename 覆盖目标。
        // 中途失败（拷贝中断/磁盘满）不会毁掉旧的 ffmpeg.exe；残留的
        // `.tmp` 由 RAII guard 在析构时清理
        let tmp_path = out_path.with_extension("tmp");
        let _tmp_guard = TempFileGuard::new(tmp_path.clone());
        {
            let mut out = std::fs::File::create(&tmp_path).map_err(|e| {
                AppError::system(IO_WRITE_FAIL, tr!("wizard.create_output_file_failed")).with_technical(e.to_string())
            })?;
            std::io::copy(&mut entry, &mut out).map_err(|e| {
                AppError::system(IO_WRITE_FAIL, tr!("wizard.extract_write_failed")).with_technical(e.to_string())
            })?;
        }
        // Windows 的 rename 不覆盖已存在文件：先移除旧文件再替换。
        // 此时 .tmp 已完整写入并关闭句柄，替换窗口极短，不会出现半写状态
        let _ = std::fs::remove_file(&out_path);
        std::fs::rename(&tmp_path, &out_path).map_err(|e| {
            AppError::system(IO_WRITE_FAIL, tr!("wizard.replace_output_failed")).with_technical(e.to_string())
        })?;
        extracted.push(out_path);
    }

    if extracted.is_empty() {
        return Err(
            AppError::system(IO_WRITE_FAIL, tr!("wizard.zip_no_ffmpeg"))
                .with_suggestion(tr!("wizard.manual_download_hint")),
        );
    }
    Ok(extracted)
}

/// 下载 FFmpeg 便携版并解压到 `{exe_dir}/ffmpeg/`，完成后重新触发 FfmpegCheck。
///
/// Windows：流式下载（reqwest `bytes_stream`），按接收字节数计算百分比并 emit
/// `download:progress` 事件；成功后返回重检结果与下载路径（修复子代理 B：
/// **不再写配置**——ffmpeg_path/ffprobe_path 由前端暂存，向导最后一步「完成」
/// 按钮点击时随 save_config 统一落盘）。失败返回 AppError（附带手动下载提示）。
/// Linux：不自动下载（移植决策 #3：gyan.dev 源只有 Windows 构建），直接返回
/// 错误并提示用系统包安装（Arch Linux：`sudo pacman -S ffmpeg`）；其余非
/// Windows 平台同理返回错误提示自行安装。
#[tauri::command]
pub(crate) async fn download_ffmpeg(
    window: tauri::Window,
) -> Result<DownloadFfmpegResult, AppError> {
    #[cfg(windows)]
    {
        return download_ffmpeg_windows(window).await;
    }
    #[cfg(not(windows))]
    {
        // 非 Windows 不使用参数（消除未使用警告）
        let _ = window;
        let message = if cfg!(target_os = "linux") {
            tr!("wizard.linux_auto_install_unavailable")
        } else {
            tr!("wizard.auto_download_unsupported")
        };
        return Err(AppError::system(
            crate::infrastructure::error::types::INT_UNEXPECTED,
            message,
        ));
    }
}

/// Windows 下载实现（gyan.dev zip）：流式下载 → 解压 → 重检 → 返回路径。
///
/// 与 `download_ffmpeg` 命令分离：非 Windows 平台不编译本函数（含 zip 解压与
/// reqwest stream 逻辑），命令签名保持跨平台一致。`#[tauri::command]` 宏只加在
/// 命令入口上，本函数不重复生成 `__cmd__` 宏。
#[cfg(windows)]
async fn download_ffmpeg_windows(
    window: tauri::Window,
) -> Result<DownloadFfmpegResult, AppError> {
    use tokio::io::AsyncWriteExt;

    tracing::info!("{}", tr!("wizard.download_start", url = FFMPEG_DOWNLOAD_URL));
    DownloadProgress::emit(&window, 0, "connecting");

    // G9 例外说明：FFmpeg 下载**不**复用共享 HTTP client（spider.rs
    // `MissevanClient::from_config` 缓存实例）——共享池按 api_timeout_secs
    //（默认 10s）与全局代理配置构建，而本下载是单次长时流式传输（600s 超时、
    // 大文件、一次性的连接），复用一个 10s 超时的客户端会直接中断下载；且
    // 走共享池会引入用户代理配置（可能干扰 gyan.dev 下载源）。保持独立
    // 客户端不构成「命令层每次调用新建」的抖动模式（下载是向导中的一次性操作）。
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| {
            AppError::network(tr!("wizard.create_client_failed"))
                .with_technical(e.to_string())
                .with_suggestion(tr!("wizard.manual_download_hint"))
        })?;

    let resp = client
        .get(FFMPEG_DOWNLOAD_URL)
        .send()
        .await
        .map_err(|e| {
            AppError::network(tr!("wizard.connect_download_source_failed"))
                .with_technical(e.to_string())
                .with_suggestion(tr!("wizard.manual_download_hint"))
        })?;

    // 1. 流式下载到 {exe_dir}/ffmpeg/ 下的临时 zip。
    //    路径提前计算并挂 RAII guard：任何失败路径（非 2xx、下载中断、
    //    写入失败、解压失败）析构时都会删除临时 zip，避免磁盘残留
    let ffmpeg_dir = crate::domain::tools::exe_dir().join("ffmpeg");
    let zip_path = ffmpeg_dir.join("ffmpeg-release-essentials.zip");
    let _zip_guard = TempFileGuard::new(zip_path.clone());
    if !resp.status().is_success() {
        return Err(AppError::network(tr!("wizard.download_source_bad_status", status = resp.status()))
            .with_suggestion(tr!("wizard.manual_download_hint")));
    }
    let total = resp.content_length().unwrap_or(0);

    std::fs::create_dir_all(&ffmpeg_dir).map_err(|e| {
        AppError::system(IO_WRITE_FAIL, tr!("wizard.create_ffmpeg_dir_failed")).with_technical(e.to_string())
    })?;

    let mut file = tokio::fs::File::create(&zip_path).await.map_err(|e| {
        AppError::system(IO_WRITE_FAIL, tr!("wizard.create_download_temp_failed")).with_technical(e.to_string())
    })?;

    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_percent: u8 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            AppError::network(tr!("wizard.download_interrupted"))
                .with_technical(e.to_string())
                .with_suggestion(tr!("wizard.manual_download_hint"))
        })?;
        file.write_all(&chunk).await.map_err(|e| {
            AppError::system(IO_WRITE_FAIL, tr!("wizard.write_download_failed")).with_technical(e.to_string())
        })?;
        downloaded += chunk.len() as u64;
        let percent = if total > 0 {
            ((downloaded.saturating_mul(100)) / total).min(99) as u8
        } else {
            last_percent
        };
        if percent != last_percent {
            DownloadProgress::emit(&window, percent, "downloading");
            last_percent = percent;
        }
    }
    file.flush().await.map_err(|e| {
        AppError::system(IO_WRITE_FAIL, tr!("wizard.flush_download_failed")).with_technical(e.to_string())
    })?;
    file.sync_all().await.map_err(|e| {
        AppError::system(IO_WRITE_FAIL, tr!("wizard.sync_download_failed")).with_technical(e.to_string())
    })?;
    drop(file);
    DownloadProgress::emit(&window, 100, "downloading");
    tracing::info!("{}", tr!("wizard.download_complete", bytes = downloaded));

    // 2. SHA256 完整性校验（占位启用）：期望哈希未配置时跳过，保持原逻辑；
    //    配置后在此拦截损坏/被篡改的 zip（失败时临时文件由 _zip_guard 清理）
    verify_ffmpeg_zip_sha256(&zip_path)?;

    // 3. 解压 ffmpeg.exe / ffprobe.exe（阻塞 IO 放入 spawn_blocking）
    DownloadProgress::emit(&window, 100, "extracting");
    let extracted: Vec<PathBuf> = tauri::async_runtime::spawn_blocking({
        let zip_path = zip_path.clone();
        let ffmpeg_dir = ffmpeg_dir.clone();
        move || extract_ffmpeg_zip(&zip_path, &ffmpeg_dir)
    })
    .await
    .map_err(|e| AppError::internal(tr!("wizard.extract_task_failed", err = e)))??;
    // 成功路径尽早删除临时 zip；失败路径由 _zip_guard 在析构时兜底清理
    let _ = std::fs::remove_file(&zip_path);
    tracing::info!("{}", tr!("wizard.extract_complete", paths = format!("{:?}", extracted)));

    // 4. 不再写配置（修复子代理 B 根因修复：配置文件的唯一写入点在向导最后一步
    //    「完成」按钮；旧实现在此 save_global 会把 ffmpeg 路径提前落盘，配置在
    //    向导第 3 步就已产生）。下载路径随返回值交给前端暂存
    //    （wizardStore.staged.ffmpegPath / ffprobePath），完成时随 stagedToConfigPatch
    //    一并写入 config.toml
    let ffmpeg_exe = ffmpeg_dir.join("ffmpeg.exe");
    let ffprobe_exe = ffmpeg_dir.join("ffprobe.exe");

    // 5. 重新触发 FfmpegCheck 并返回检测结果 + 下载路径
    DownloadProgress::emit(&window, 100, "verifying");
    let check = FfmpegCheck {
        ffmpeg_path: Some(ffmpeg_exe.to_string_lossy().into_owned()),
    };
    let result = check.run().await;
    DownloadProgress::emit(&window, 100, "done");
    tracing::info!("{}", tr!("wizard.recheck_complete", message = result.message));
    Ok(DownloadFfmpegResult {
        check: result,
        ffmpeg_path: Some(ffmpeg_exe.to_string_lossy().into_owned()),
        ffprobe_path: ffprobe_exe.to_string_lossy().into_owned(),
    })
}

/// 退出应用（向导窗的「退出」按钮；走统一优雅退出路径：保存配置 →
/// 停检测循环 → cancel 录制任务 → 等 JoinHandle ≤5s → exit）
#[tauri::command]
pub(crate) fn exit_app(app: tauri::AppHandle) {
    tracing::info!("{}", tr!("wizard.exit_requested"));
    crate::infrastructure::tray::request_shutdown(&app);
}

/// 设置向导完成：关闭向导窗口，显示并聚焦主窗口，刷新文件缓存并触发一次立即检测
#[tauri::command]
pub(crate) async fn finish_wizard(
    app: tauri::AppHandle,
    cache: State<'_, FileCacheHandle>,
    config_manager: State<'_, Arc<ConfigManager>>,
    detection_wake: State<'_, Arc<tokio::sync::Notify>>,
) -> Result<(), AppError> {
    // 0. 标记引导完成（修复子代理 B：配置已由前端在「完成」按钮 save_config 时
    //    全量落盘并显式写入 wizard_completed=true——stagedToConfigPatch 的
    //    wizard_completed 与 finish_wizard 语义对齐；此处仅为幂等兜底：若配置
    //    恰好未完成（理论不可达），置 true 后再次启动不再进引导）
    if let Ok(mut config) = config_manager.load() {
        if !config.global.wizard_completed {
            config.global.wizard_completed = true;
            let _ = config_manager.save_global(&config.global);
        }
    }
    // 1. 销毁向导窗口（向导流程结束）
    //    必须用 destroy() 而非 close()：前端 WizardView 注册了 onCloseRequested 并
    //    prevent_default()，Tauri 语义下只要存在 close-requested 监听器，close() 就被
    //    无条件取消——第 4 步「进入应用」会死锁，向导窗永远关不掉。
    //    destroy() 直接销毁窗口，不触发 CloseRequested 事件
    if let Some(wizard) = app.get_webview_window("wizard") {
        let _ = wizard.destroy();
    }
    // 2. 显示并聚焦主窗口
    let main_window = app.get_webview_window("main");
    if let Some(main_win) = &main_window {
        let _ = main_win.show();
        let _ = main_win.set_focus();
    }
    // 3. 刷新录制文件缓存（向主窗口推送最新文件列表）
    if let Some(main_win) = main_window {
        let recorder_state = app.state::<RecorderState>();
        let manager = FileCacheManager::new(main_win, cache.inner().clone());
        manager
            .refresh(&config_manager, &recorder_state.state)
            .await?;
    }
    // 4. 唤醒检测循环，触发一次立即直播检测
    detection_wake.notify_one();
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::io::Write;

    /// 构造测试 zip：包含子目录层级，以及应被忽略的文档/DLL/无扩展名条目
    fn make_test_zip(zip_path: &Path) {
        let file = std::fs::File::create(zip_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();

        writer
            .start_file("ffmpeg-7.1-essentials_build/bin/ffmpeg.exe", options)
            .unwrap();
        writer.write_all(b"ffmpeg-binary").unwrap();
        writer
            .start_file("ffmpeg-7.1-essentials_build/bin/ffprobe.exe", options)
            .unwrap();
        writer.write_all(b"ffprobe-binary").unwrap();
        // 不应提取：文档、DLL、无扩展名文件
        writer
            .start_file("ffmpeg-7.1-essentials_build/doc/readme.txt", options)
            .unwrap();
        writer.write_all(b"docs").unwrap();
        writer
            .start_file("ffmpeg-7.1-essentials_build/bin/ffmpeg-extra.dll", options)
            .unwrap();
        writer.write_all(b"dll").unwrap();
        writer
            .start_file("ffmpeg-7.1-essentials_build/bin/ffmpeg", options)
            .unwrap();
        writer.write_all(b"linux-binary").unwrap();
        writer.finish().unwrap();
    }

    #[test]
    fn flat_output_name_keeps_target_exe_and_rejects_others() {
        // zip 内任意子目录层级的 ffmpeg.exe / ffprobe.exe 都命中
        assert_eq!(
            flat_output_name("ffmpeg-7.1-essentials_build/bin/ffmpeg.exe"),
            Some("ffmpeg.exe")
        );
        assert_eq!(flat_output_name("ffmpeg.exe"), Some("ffmpeg.exe"));
        assert_eq!(flat_output_name("bin\\ffprobe.exe"), Some("ffprobe.exe"));
        // 其余一律拒绝
        assert_eq!(flat_output_name("ffmpeg-7.1-essentials_build/bin/ffmpeg"), None);
        assert_eq!(flat_output_name("ffmpeg-7.1-essentials_build/bin/ffmpeg-extra.dll"), None);
        assert_eq!(flat_output_name("ffmpeg-7.1-essentials_build/doc/readme.txt"), None);
        assert_eq!(flat_output_name(""), None);
        assert_eq!(flat_output_name("bin/"), None);
    }

    #[test]
    fn flat_output_name_sanitizes_path_traversal_prefixes() {
        // 路径穿越条目名只取 basename，配合 target_dir.join(basename) 不会逃逸出目标目录
        assert_eq!(flat_output_name("../../evil/ffmpeg.exe"), Some("ffmpeg.exe"));
        assert_eq!(flat_output_name("..\\..\\evil\\ffprobe.exe"), Some("ffprobe.exe"));
        // 穿越前缀 + 非目标文件名仍被拒绝
        assert_eq!(flat_output_name("../../evil/evil.exe"), None);
    }

    #[test]
    fn extract_zip_extracts_only_exe_files_flattened() {
        let dir = std::env::temp_dir().join("missevan-test-ffmpeg-zip");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let zip_path = dir.join("ffmpeg.zip");
        make_test_zip(&zip_path);

        let out_dir = dir.join("out");
        let extracted = extract_ffmpeg_zip(&zip_path, &out_dir).unwrap();

        assert_eq!(extracted.len(), 2);
        assert_eq!(
            std::fs::read(out_dir.join("ffmpeg.exe")).unwrap(),
            b"ffmpeg-binary"
        );
        assert_eq!(
            std::fs::read(out_dir.join("ffprobe.exe")).unwrap(),
            b"ffprobe-binary"
        );
        // 子目录层级被扁平化，无残留文件
        assert!(!out_dir.join("readme.txt").exists());
        assert!(!out_dir.join("ffmpeg-extra.dll").exists());
        assert!(!out_dir.join("ffmpeg").exists());
        assert!(!out_dir.join("ffmpeg-7.1-essentials_build").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_zip_without_targets_returns_error() {
        let dir = std::env::temp_dir().join("missevan-test-ffmpeg-zip-empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let zip_path = dir.join("empty.zip");
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file(
                "ffmpeg-7.1-essentials_build/doc/readme.txt",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(b"docs").unwrap();
        writer.finish().unwrap();

        let out_dir = dir.join("out");
        let err = extract_ffmpeg_zip(&zip_path, &out_dir).unwrap_err();
        assert!(err.message.contains(&tr!("wizard.zip_no_ffmpeg")));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
