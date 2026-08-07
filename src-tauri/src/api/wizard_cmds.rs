//! 设置向导相关 Tauri 命令（Task 7）
//!
//! - `download_ffmpeg`：流式下载 FFmpeg 便携版到 `{exe_dir}/ffmpeg/`，emit
//!   `download:progress { percent, stage }` 事件，解压后更新配置并重新触发 FfmpegCheck
//! - `exit_app`：退出应用
//! - `finish_wizard`：向导完成（关闭向导窗 / 显示聚焦主窗 / 刷新文件缓存 / 唤醒检测循环）
//!
//! 注意：`#[tauri::command]` 会生成 `__cmd__xxx` 宏导入，其可见性跟随函数；
//! 命令须为 `pub(crate)` 才能被 lib.rs 根模块的 `generate_handler!` 通过全路径引用
//! （Task 6 在根模块用非 pub 骨架可行，是因为宏与调用方同模块；E0255/E0603 已知坑）。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::StreamExt;
use tauri::{Emitter, Manager, State};

use crate::domain::config::manager::ConfigManager;
use crate::domain::services::file_cache::{FileCacheHandle, FileCacheManager};
use crate::infrastructure::checker::checks::{DiskSpaceCheck, FfmpegCheck, HealthCheck};
use crate::infrastructure::checker::report::{CheckResult, CheckStatus, DiagnosticReport};
use crate::infrastructure::state::app_state::RecorderState;
use crate::infrastructure::error::types::{AppError, IO_WRITE_FAIL};

/// FFmpeg 下载源（gyan.dev 官方构建，内含 ffmpeg.exe / ffprobe.exe）
const FFMPEG_DOWNLOAD_URL: &str = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip";
/// 网络失败时提示手动下载
const MANUAL_DOWNLOAD_HINT: &str =
    "请前往 https://ffmpeg.org/download.html 手动下载 FFmpeg，并在配置中设置 ffmpeg_path";

/// 需要从 zip 中提取的可执行文件名（不含路径，匹配 zip 内任意子目录层级）
const FFMPEG_ZIP_TARGETS: [&str; 2] = ["ffmpeg.exe", "ffprobe.exe"];

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
                message: format!("{} 可用: {}", name, version),
                details: Some(format!("路径: {}", cand.display())),
                suggestion: None,
                duration_ms: start.elapsed().as_millis() as u64,
            };
        }
    }
    CheckResult {
        check_name: name.to_string(),
        status: CheckStatus::Failed,
        message: format!("未找到 {}", name),
        details: None,
        suggestion: Some("请点击“下载并安装”按钮自动安装，或手动下载后设置 ffmpeg_path".into()),
        duration_ms: start.elapsed().as_millis() as u64,
    }
}

/// 输出目录写入权限检查：幂等创建目录，写入并删除临时探测文件
async fn write_permission_check(output_dir: &str) -> CheckResult {
    let start = std::time::Instant::now();
    let dir = std::path::Path::new(output_dir);
    let probe = dir.join(format!(".missevan-write-test-{}", std::process::id()));

    let result: Result<(), String> = (|| {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建目录失败: {}", e))?;
        std::fs::write(&probe, b"ok").map_err(|e| format!("写入测试文件失败: {}", e))?;
        std::fs::remove_file(&probe).map_err(|e| format!("删除测试文件失败: {}", e))?;
        Ok(())
    })();

    match result {
        Ok(()) => CheckResult {
            check_name: "输出目录写入权限".to_string(),
            status: CheckStatus::Passed,
            message: "输出目录可写".to_string(),
            details: Some(format!("目录: {}", output_dir)),
            suggestion: None,
            duration_ms: start.elapsed().as_millis() as u64,
        },
        Err(msg) => CheckResult {
            check_name: "输出目录写入权限".to_string(),
            status: CheckStatus::Failed,
            message: format!("无写入权限：{}", msg),
            details: None,
            suggestion: Some("请更换输出目录，或检查目录权限".to_string()),
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

/// `download:progress` 事件载荷（Task 8 向导 UI 依赖）
#[derive(Debug, Clone, serde::Serialize)]
struct DownloadProgress {
    /// 下载进度百分比（0-100；解压/验证阶段沿用上一值）
    percent: u8,
    /// 阶段：connecting / downloading / extracting / verifying / done
    stage: &'static str,
}

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
struct TempFileGuard(PathBuf);

impl TempFileGuard {
    fn new(path: PathBuf) -> Self {
        Self(path)
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.0) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("清理临时文件失败 {:?}: {}", self.0, e);
            }
        }
    }
}

/// 将 zip 条目名映射为扁平化输出文件名。
///
/// 只提取 `ffmpeg.exe` / `ffprobe.exe`（忽略条目在 zip 内的子目录层级，
/// 如 `ffmpeg-7.1-essentials_build/bin/ffmpeg.exe`），其余条目返回 None。
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
/// 同步阻塞 IO，调用方应放入 `spawn_blocking`。
fn extract_ffmpeg_zip(zip_path: &Path, target_dir: &Path) -> Result<Vec<PathBuf>, AppError> {
    std::fs::create_dir_all(target_dir).map_err(|e| {
        AppError::system(IO_WRITE_FAIL, "创建 FFmpeg 目录失败").with_technical(e.to_string())
    })?;

    let file = std::fs::File::open(zip_path).map_err(|e| {
        AppError::system(IO_WRITE_FAIL, "打开下载的 zip 失败").with_technical(e.to_string())
    })?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| {
            AppError::system(IO_WRITE_FAIL, "zip 文件损坏或格式不支持").with_technical(e.to_string())
        })?;

    let mut extracted = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| {
            AppError::system(IO_WRITE_FAIL, "读取 zip 条目失败").with_technical(e.to_string())
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
                AppError::system(IO_WRITE_FAIL, "创建输出文件失败").with_technical(e.to_string())
            })?;
            std::io::copy(&mut entry, &mut out).map_err(|e| {
                AppError::system(IO_WRITE_FAIL, "解压写入失败").with_technical(e.to_string())
            })?;
        }
        // Windows 的 rename 不覆盖已存在文件：先移除旧文件再替换。
        // 此时 .tmp 已完整写入并关闭句柄，替换窗口极短，不会出现半写状态
        let _ = std::fs::remove_file(&out_path);
        std::fs::rename(&tmp_path, &out_path).map_err(|e| {
            AppError::system(IO_WRITE_FAIL, "替换输出文件失败").with_technical(e.to_string())
        })?;
        extracted.push(out_path);
    }

    if extracted.is_empty() {
        return Err(
            AppError::system(IO_WRITE_FAIL, "zip 中未找到 ffmpeg.exe / ffprobe.exe")
                .with_suggestion(MANUAL_DOWNLOAD_HINT),
        );
    }
    Ok(extracted)
}

/// 下载 FFmpeg 便携版并解压到 `{exe_dir}/ffmpeg/`，完成后重新触发 FfmpegCheck。
///
/// 流式下载（reqwest `bytes_stream`），按接收字节数计算百分比并 emit
/// `download:progress` 事件；成功后把 `ffmpeg_path` / `ffprobe_path` 写入配置
/// 并返回重检结果。失败返回 AppError（附带手动下载提示）。
#[tauri::command]
pub(crate) async fn download_ffmpeg(
    window: tauri::Window,
    config_manager: State<'_, Arc<ConfigManager>>,
) -> Result<CheckResult, AppError> {
    use tokio::io::AsyncWriteExt;

    tracing::info!("开始下载 FFmpeg: {}", FFMPEG_DOWNLOAD_URL);
    DownloadProgress::emit(&window, 0, "connecting");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| {
            AppError::network("创建下载客户端失败")
                .with_technical(e.to_string())
                .with_suggestion(MANUAL_DOWNLOAD_HINT)
        })?;

    let resp = client
        .get(FFMPEG_DOWNLOAD_URL)
        .send()
        .await
        .map_err(|e| {
            AppError::network("连接 FFmpeg 下载源失败")
                .with_technical(e.to_string())
                .with_suggestion(MANUAL_DOWNLOAD_HINT)
        })?;

    // 1. 流式下载到 {exe_dir}/ffmpeg/ 下的临时 zip。
    //    路径提前计算并挂 RAII guard：任何失败路径（非 2xx、下载中断、
    //    写入失败、解压失败）析构时都会删除临时 zip，避免磁盘残留
    let ffmpeg_dir = crate::domain::tools::exe_dir().join("ffmpeg");
    let zip_path = ffmpeg_dir.join("ffmpeg-release-essentials.zip");
    let _zip_guard = TempFileGuard::new(zip_path.clone());
    if !resp.status().is_success() {
        return Err(AppError::network(format!("下载源返回异常状态码: {}", resp.status()))
            .with_suggestion(MANUAL_DOWNLOAD_HINT));
    }
    let total = resp.content_length().unwrap_or(0);

    std::fs::create_dir_all(&ffmpeg_dir).map_err(|e| {
        AppError::system(IO_WRITE_FAIL, "创建 FFmpeg 目录失败").with_technical(e.to_string())
    })?;

    let mut file = tokio::fs::File::create(&zip_path).await.map_err(|e| {
        AppError::system(IO_WRITE_FAIL, "创建下载临时文件失败").with_technical(e.to_string())
    })?;

    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_percent: u8 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            AppError::network("下载中断")
                .with_technical(e.to_string())
                .with_suggestion(MANUAL_DOWNLOAD_HINT)
        })?;
        file.write_all(&chunk).await.map_err(|e| {
            AppError::system(IO_WRITE_FAIL, "写入下载文件失败").with_technical(e.to_string())
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
        AppError::system(IO_WRITE_FAIL, "刷新下载文件失败").with_technical(e.to_string())
    })?;
    file.sync_all().await.map_err(|e| {
        AppError::system(IO_WRITE_FAIL, "同步下载文件失败").with_technical(e.to_string())
    })?;
    drop(file);
    DownloadProgress::emit(&window, 100, "downloading");
    tracing::info!("FFmpeg 下载完成: {} bytes", downloaded);

    // 2. 解压 ffmpeg.exe / ffprobe.exe（阻塞 IO 放入 spawn_blocking）
    DownloadProgress::emit(&window, 100, "extracting");
    let extracted: Vec<PathBuf> = tauri::async_runtime::spawn_blocking({
        let zip_path = zip_path.clone();
        let ffmpeg_dir = ffmpeg_dir.clone();
        move || extract_ffmpeg_zip(&zip_path, &ffmpeg_dir)
    })
    .await
    .map_err(|e| AppError::internal(format!("解压任务失败: {}", e)))??;
    // 成功路径尽早删除临时 zip；失败路径由 _zip_guard 在析构时兜底清理
    let _ = std::fs::remove_file(&zip_path);
    tracing::info!("FFmpeg 解压完成: {:?}", extracted);

    // 3. 更新配置中的 ffmpeg_path / ffprobe_path（指向本次下载的可执行文件）
    let ffmpeg_exe = ffmpeg_dir.join("ffmpeg.exe");
    let ffprobe_exe = ffmpeg_dir.join("ffprobe.exe");
    let mut config = config_manager.load()?;
    config.global.ffmpeg_path = Some(ffmpeg_exe.to_string_lossy().into_owned());
    config.global.ffprobe_path = ffprobe_exe.to_string_lossy().into_owned();
    config_manager.save_global(&config.global)?;

    // 4. 重新触发 FfmpegCheck 并返回检测结果
    DownloadProgress::emit(&window, 100, "verifying");
    let check = FfmpegCheck {
        ffmpeg_path: Some(ffmpeg_exe.to_string_lossy().into_owned()),
    };
    let result = check.run().await;
    DownloadProgress::emit(&window, 100, "done");
    tracing::info!("FFmpeg 重检完成: {}", result.message);
    Ok(result)
}

/// 退出应用（向导窗的「退出」按钮；走统一优雅退出路径：保存配置 →
/// 停检测循环 → cancel 录制任务 → 等 JoinHandle ≤5s → exit）
#[tauri::command]
pub(crate) fn exit_app(app: tauri::AppHandle) {
    tracing::info!("用户请求退出应用");
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
    // 0. 标记引导完成（主 AGENT 引导逻辑修复）：config.toml 在第 3 步已写盘，
    //    is_first_run 依赖 wizard_completed——置 true 后再次启动不再进引导
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

#[cfg(test)]
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
        assert!(err.message.contains("未找到 ffmpeg.exe"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
