//! 文件系统辅助命令（§11.2 托盘/系统：`open_output_dir`）
//!
//! Windows 上用资源管理器打开（选中）输出目录；目录不存在时先创建。

use std::sync::Arc;
use tauri::State;

use crate::domain::config::manager::ConfigManager;
use crate::infrastructure::error::types::AppError;

/// 打开输出目录（Windows：`explorer /select,` 并选中该目录）
#[tauri::command]
pub(crate) async fn open_output_dir(
    config_manager: State<'_, Arc<ConfigManager>>,
) -> Result<(), AppError> {
    let config = config_manager.load()?;
    let dir = std::path::Path::new(&config.global.output_dir);
    if !dir.exists() {
        std::fs::create_dir_all(dir).map_err(|e| {
            AppError::system(
                crate::infrastructure::error::types::IO_WRITE_FAIL,
                "创建输出目录失败",
            )
            .with_technical(e.to_string())
        })?;
    }
    open_in_explorer(dir)
}

/// Windows 实现：`explorer /select,{path}`（explorer 是 GUI 程序，spawn 后立即返回）
/// pub(crate)：托盘「最近录制」菜单复用（Task 17）
#[cfg(windows)]
pub(crate) fn open_in_explorer(dir: &std::path::Path) -> Result<(), AppError> {
    std::process::Command::new("explorer")
        .arg(format!("/select,{}", dir.display()))
        .spawn()
        .map_err(|e| {
            AppError::system(
                crate::infrastructure::error::types::INT_UNEXPECTED,
                "打开资源管理器失败",
            )
            .with_technical(e.to_string())
        })?;
    Ok(())
}

/// 非 Windows 平台：暂不支持
#[cfg(not(windows))]
pub(crate) fn open_in_explorer(_dir: &std::path::Path) -> Result<(), AppError> {
    Err(AppError::internal("当前平台不支持打开资源管理器"))
}
