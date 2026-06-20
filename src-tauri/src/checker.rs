use serde::Serialize;
use std::path::PathBuf;
use tauri::Emitter;

use crate::config::model::Config;

#[derive(Debug, Clone, Serialize)]
pub struct CheckNotification {
    pub level: String, // "error", "warning", "info"
    pub message: String,
    pub check_name: String,
}

/// 检查 FFmpeg 是否存在（返回路径或错误）
pub async fn check_ffmpeg(ffmpeg_path: Option<&str>) -> Result<PathBuf, String> {
    crate::utils::check_ffmpeg(ffmpeg_path)
        .await
        .map_err(|e| e.to_string())
}

/// 仅检查 FFmpeg 并发送通知
pub async fn run_ffmpeg_check(app_handle: tauri::AppHandle, config: &Config) {
    match check_ffmpeg(config.ffmpeg_path.as_deref()).await {
        Ok(path) => {
            let notif = CheckNotification {
                level: "info".to_string(),
                message: format!("✅ FFmpeg 已就绪: {}", path.display()),
                check_name: "ffmpeg".to_string(),
            };
            let _ = app_handle.emit("check-notification", notif);
        }
        Err(e) => {
            let notif = CheckNotification {
                level: "error".to_string(),
                message: format!("❌ FFmpeg 未找到，录制功能无法使用。\n{}", e),
                check_name: "ffmpeg".to_string(),
            };
            let _ = app_handle.emit("check-notification", notif);
        }
    }
}

/// 执行所有检查（磁盘空间等）
pub async fn run_all_checks(app_handle: tauri::AppHandle, config: &Config) {
    // 1. FFmpeg 检查
    run_ffmpeg_check(app_handle.clone(), config).await;

    // 2. 磁盘空间检查（使用全局 output_dir）
    if let Err(e) = crate::utils::check_disk_space(&config.output_dir, config.disk_space_limit_gb) {
        let notif = CheckNotification {
            level: "warning".to_string(),
            message: format!("⚠️ 磁盘空间不足: {}", e),
            check_name: "disk_space".to_string(),
        };
        let _ = app_handle.emit("check-notification", notif);
    }

    // 未来可添加其他检查
}
