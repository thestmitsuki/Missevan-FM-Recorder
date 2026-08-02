//! 应用单实例锁（Windows 命名互斥体）。
//!
//! 背景：双录根因候选②——应用双开时，两个实例各自持有独立的检测循环、
//! 任务表与 FFmpeg 进程表，会为同一主播同时启动两个录制进程（双录）。
//! 本模块用命名互斥体（`CreateMutexW` + `ERROR_ALREADY_EXISTS`）实现单实例：
//! 首个实例创建互斥体并持有句柄直至进程退出；后续实例在 `run()` 开头检测到
//! 互斥体已存在即退出（日志说明，不弹窗）。
//!
//! 实现取舍（Task 7 依赖纪律）：未用 tauri-plugin-single-instance——其 2.4.3
//! rust-version=1.77.2 ≤ 项目 MSRV 1.89.0（MSRV 本身兼容），但加入后重解析
//! 依赖树会把 tauri-runtime 的 toml 从 0.9.12 升到 1.1.0（rust-version=1.85
//! ≤ MSRV 1.89.0，MSRV 可接受）——锁文件整体变动，自实现方案已满足需求
//!（Cargo.toml 注释详见 windows feature）。
//! 改为使用依赖树中已有的 windows 0.61（仅新增 Win32_System_Threading /
//! Win32_Security feature，零新增编译单元），自实现 ~60 行命名互斥体：
//! 行为确定（第二实例立即退出，无隐藏窗口残留），且可在单测中验证
//! （同进程第二次 acquire 返回 None——与真实双开同一语义）。
//! 非 Windows 平台为空实现（本项目实际仅面向 Windows：winreg / Windows
//! toast / windows crate；macOS/Linux 构建不受影响、不做单实例约束）。

/// 单实例守卫：持有命名互斥体句柄，进程存活期间保持打开（Drop 时关闭；
/// 句柄全部关闭后内核对象销毁——实例崩溃也不留「死锁」）
#[cfg(windows)]
pub struct InstanceGuard {
    handle: windows::Win32::Foundation::HANDLE,
}

/// 尝试获取单实例锁。
///
/// - `Some(guard)`：本实例获得锁（首个实例）；guard 需存活到进程退出
/// - `None`：另一实例已在运行（或互斥体创建失败）——调用方应退出
#[cfg(windows)]
pub fn acquire(name: &str) -> Option<InstanceGuard> {
    use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::PCWSTR;

    // 非 owned 互斥体（binitialowner=false）：无需 ReleaseMutex，
    // 句柄关闭即释放；ERROR_ALREADY_EXISTS 语义与所有权无关
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let handle: HANDLE = unsafe { CreateMutexW(None, false, PCWSTR(wide.as_ptr())).ok()? };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // 另一实例已持有该互斥体：关闭本句柄（不释放对方的锁）
        unsafe {
            let _ = CloseHandle(handle);
        }
        return None;
    }
    Some(InstanceGuard { handle })
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

/// 非 Windows 平台空实现（无单实例约束）
#[cfg(not(windows))]
pub struct InstanceGuard;

#[cfg(not(windows))]
pub fn acquire(_name: &str) -> Option<InstanceGuard> {
    Some(InstanceGuard)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// 互斥体名：进程内唯一（含 pid），避免与其他测试/并行运行冲突
    fn test_mutex_name() -> String {
        format!("missevan-recorder-test-single-instance-{}", std::process::id())
    }

    #[test]
    fn second_acquire_rejected_while_first_guard_alive() {
        let name = test_mutex_name();
        // 首个实例：获取成功
        let guard = acquire(&name).expect("首次获取单实例锁应成功");
        // 第二「实例」：互斥体已存在 → 拒绝（与真实双开同一语义）
        assert!(
            acquire(&name).is_none(),
            "守卫存活期间再次获取必须被拒绝"
        );
        drop(guard);
        // 句柄释放后互斥体销毁：可再次获取
        let guard2 = acquire(&name).expect("释放后应可再次获取");
        drop(guard2);
    }

    #[test]
    fn distinct_names_do_not_conflict() {
        let a = format!("{}-a", test_mutex_name());
        let b = format!("{}-b", test_mutex_name());
        let guard_a = acquire(&a).expect("首次获取 a 应成功");
        // 不同名字的互斥体互不影响
        let guard_b = acquire(&b).expect("不同名字的互斥体不应冲突");
        drop(guard_a);
        drop(guard_b);
    }
}
