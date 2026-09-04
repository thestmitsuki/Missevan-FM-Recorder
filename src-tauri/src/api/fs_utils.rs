//! 文件系统辅助命令（§11.2 托盘/系统：`open_output_dir`）
//!
//! 打开输出目录（目录不存在时先创建），实现走 tauri-plugin-opener 的
//! `open_path`：Windows 以资源管理器打开目录本身，Linux 经 xdg-open 打开。

use std::sync::Arc;
use tauri::State;
use tauri_plugin_opener::OpenerExt;

use crate::domain::config::manager::ConfigManager;
use crate::domain::config::model::resolve_output_dir;
use crate::infrastructure::error::types::AppError;
use crate::tr;

/// 打开输出目录（opener 插件 `open_path`：打开目录本身）
#[tauri::command]
pub(crate) async fn open_output_dir(
    app: tauri::AppHandle,
    config_manager: State<'_, Arc<ConfigManager>>,
) -> Result<(), AppError> {
    let config = config_manager.load()?;
    // 打开/创建的是真实目录：磁盘原值（相对可能）先解析为物理位置
    //（model::resolve_output_dir），相对值会落到进程 CWD（开错目录不报错）
    let dir = resolve_output_dir(&config.global.output_dir);
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| {
            AppError::system(
                crate::infrastructure::error::types::IO_WRITE_FAIL,
                tr!("app.create_output_dir_failed"),
            )
            .with_technical(e.to_string())
        })?;
    }
    // tauri_plugin_opener::Opener::open_path(path: impl Into<String>, with: Option<impl Into<String>>)
    app.opener()
        .open_path(dir.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|e| {
            AppError::system(
                crate::infrastructure::error::types::INT_UNEXPECTED,
                tr!("app.open_output_dir_failed"),
            )
            .with_technical(e.to_string())
        })
}
