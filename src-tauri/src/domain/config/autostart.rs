//! 开机自启（§11.2 `set_autostart`）
//!
//! Windows 实现写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 键
//! `MissevanRecorder`（值 = `"{exe_path}" --minimized`）；非 Windows 平台为空实现
//! （仅记录日志，保持命令可用）。`AutostartStore` trait 抽象注册表读写，便于
//! 单测注入内存 mock。

use crate::infrastructure::error::types::AppError;

/// Run 键路径（HKCU，无需管理员权限）
pub const RUN_KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// 注册表值名
pub const AUTOSTART_VALUE_NAME: &str = "MissevanRecorder";

/// 开机自启注册表读写（抽象层，可 mock）
pub trait AutostartStore: Send + Sync {
    /// 写入/更新 Run 条目；`value` 为 None 时删除条目（目标状态达成即成功）
    fn set_run_entry(&self, name: &str, value: Option<&str>) -> Result<(), AppError>;
    /// 读取 Run 条目；不存在返回 Ok(None)
    /// 测试辅助：仅单测断言使用，生产路径不调用（set_autostart 只写不读）
    #[allow(dead_code)]
    fn get_run_entry(&self, name: &str) -> Result<Option<String>, AppError>;
}

/// Windows 生产实现（winreg 0.55，rust-version 1.60 ≤ MSRV 1.89.0）
#[cfg(windows)]
#[derive(Default)]
pub struct WinregAutostart;

#[cfg(windows)]
impl AutostartStore for WinregAutostart {
    fn set_run_entry(&self, name: &str, value: Option<&str>) -> Result<(), AppError> {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        // winreg 0.55 的 create_subkey 返回 (RegKey, RegDisposition) 元组
        let (run, _disposition) = hkcu.create_subkey(RUN_KEY_PATH).map_err(|e| {
            AppError::system(
                crate::infrastructure::error::types::IO_WRITE_FAIL,
                "打开注册表 Run 键失败",
            )
            .with_technical(e.to_string())
        })?;
        match value {
            Some(v) => run.set_value(name, &v).map_err(|e| {
                AppError::system(
                    crate::infrastructure::error::types::IO_WRITE_FAIL,
                    "写入开机自启注册表失败",
                )
                .with_technical(e.to_string())
            })?,
            None => {
                // 条目不存在时删除会报错——目标状态已达成，忽略
                let _ = run.delete_value(name);
            }
        }
        Ok(())
    }

    fn get_run_entry(&self, name: &str) -> Result<Option<String>, AppError> {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        match hkcu.open_subkey(RUN_KEY_PATH) {
            Ok(run) => match run.get_value::<String, _>(name) {
                Ok(v) => Ok(Some(v)),
                Err(_) => Ok(None),
            },
            Err(_) => Ok(None),
        }
    }
}

/// 非 Windows 平台：空实现
#[cfg(not(windows))]
#[derive(Default)]
pub struct NoopAutostart;

#[cfg(not(windows))]
impl AutostartStore for NoopAutostart {
    fn set_run_entry(&self, _name: &str, value: Option<&str>) -> Result<(), AppError> {
        tracing::warn!(
            "非 Windows 平台不支持开机自启注册表写入（请求: {:?}）",
            value.map(|_| "enabled")
        );
        Ok(())
    }

    fn get_run_entry(&self, _name: &str) -> Result<Option<String>, AppError> {
        Ok(None)
    }
}

/// 构造自启命令字符串：`"{exe_path}" --minimized`
pub fn autostart_command(exe_path: &str) -> String {
    format!("\"{}\" --minimized", exe_path)
}

/// 应用自启设置（命令层核心逻辑；Store 注入便于单测 mock）
pub fn apply_autostart(
    store: &dyn AutostartStore,
    enabled: bool,
    exe_path: &str,
) -> Result<(), AppError> {
    if enabled {
        store.set_run_entry(AUTOSTART_VALUE_NAME, Some(&autostart_command(exe_path)))
    } else {
        store.set_run_entry(AUTOSTART_VALUE_NAME, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// 内存 mock（测试 AutostartStore 读写契约）
    #[derive(Default)]
    struct MockAutostartStore(Mutex<HashMap<String, String>>);

    impl AutostartStore for MockAutostartStore {
        fn set_run_entry(&self, name: &str, value: Option<&str>) -> Result<(), AppError> {
            let mut map = self.0.lock().unwrap();
            match value {
                Some(v) => {
                    map.insert(name.to_string(), v.to_string());
                }
                None => {
                    map.remove(name);
                }
            }
            Ok(())
        }

        fn get_run_entry(&self, name: &str) -> Result<Option<String>, AppError> {
            Ok(self.0.lock().unwrap().get(name).cloned())
        }
    }

    #[test]
    fn autostart_command_quotes_exe_and_appends_flag() {
        assert_eq!(
            autostart_command(r"C:\Program Files\missevan-recorder\app.exe"),
            r#""C:\Program Files\missevan-recorder\app.exe" --minimized"#
        );
    }

    #[test]
    fn apply_autostart_enabled_writes_run_entry() {
        let store = MockAutostartStore::default();
        apply_autostart(&store, true, r"C:\app\app.exe").unwrap();
        let got = store.get_run_entry(AUTOSTART_VALUE_NAME).unwrap().unwrap();
        assert_eq!(got, r#""C:\app\app.exe" --minimized"#);
    }

    #[test]
    fn apply_autostart_disabled_removes_run_entry() {
        let store = MockAutostartStore::default();
        apply_autostart(&store, true, r"C:\app\app.exe").unwrap();
        apply_autostart(&store, false, r"C:\app\app.exe").unwrap();
        assert!(store.get_run_entry(AUTOSTART_VALUE_NAME).unwrap().is_none());
        // 幂等：未启用状态下再次禁用也成功
        apply_autostart(&store, false, r"C:\app\app.exe").unwrap();
    }

    #[test]
    fn apply_autostart_toggle_updates_value() {
        let store = MockAutostartStore::default();
        apply_autostart(&store, true, r"C:\app\a.exe").unwrap();
        apply_autostart(&store, true, r"C:\app\b.exe").unwrap();
        let got = store.get_run_entry(AUTOSTART_VALUE_NAME).unwrap().unwrap();
        assert_eq!(got, r#""C:\app\b.exe" --minimized"#);
    }
}
