#![allow(dead_code)]

pub mod buffer;
pub mod dispatcher;
pub mod types;
// Windows 原生 toast（组 C/3：应用注册通知，杜绝 PowerShell 兜底）
#[cfg(windows)]
pub mod windows_toast;
