//! 文件系统辅助命令（§11.2 托盘/系统：`open_output_dir`）
//!
//! Windows 上用资源管理器打开（选中）输出目录；目录不存在时先创建。
//! 打开实现（open_in_explorer）位于 `domain::tools`——录制后动作
//! （post_record_action=open_folder）与托盘「最近录制」菜单复用同一实现。

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
    crate::domain::tools::open_in_explorer(dir)
}
