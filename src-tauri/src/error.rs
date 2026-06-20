use serde::Serialize;
use thiserror::Error;

#[derive(Error, Debug, Serialize)]
pub enum AppError {
    #[error("配置错误: {0}")]
    Config(String),
    #[error("FFmpeg 未找到: {0}")]
    FfmpegNotFound(String),
    #[error("网络请求失败: {0}")]
    Network(String),
    #[error("IO 错误: {0}")]
    Io(String),
    #[error("录制进程异常: {0}")]
    Recorder(String),
    #[error("主播不存在: {0}")]
    AnchorNotFound(String),
    #[error("其他错误: {0}")]
    Other(String),
}

// 转换各种错误类型
impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        AppError::Other(e.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e.to_string())
    }
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        AppError::Network(e.to_string())
    }
}

impl From<toml::ser::Error> for AppError {
    fn from(e: toml::ser::Error) -> Self {
        AppError::Config(e.to_string())
    }
}

impl From<toml::de::Error> for AppError {
    fn from(e: toml::de::Error) -> Self {
        AppError::Config(e.to_string())
    }
}

// 用于将 AppError 转换为 String 返回给前端（tauri 命令默认需要 String）
impl From<AppError> for String {
    fn from(e: AppError) -> Self {
        e.to_string()
    }
}
