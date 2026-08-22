//! 应用单实例锁（fs2 文件锁，全平台统一）。
//!
//! 背景：双录根因候选②——应用双开时，两个实例各自持有独立的检测循环、
//! 任务表与 FFmpeg 进程表，会为同一主播同时启动两个录制进程（双录）。
//!
//! 实现：`fs2::FileExt::try_lock_exclusive` 独占文件锁（Windows 为
//! `LockFileEx`，Linux/Unix 为 `flock`）。锁文件目录按平台区分：
//!
//! - Windows：`dirs::cache_dir()/missevan-recorder`（如
//!   `%LOCALAPPDATA%\...\cache\missevan-recorder`）；
//! - Linux/Unix：`$XDG_RUNTIME_DIR/missevan-recorder`（用户级运行时目录，
//!   系统重启自动清理）；未设置 XDG_RUNTIME_DIR 时回退
//!   `~/.cache/missevan-recorder`。
//!
//! `InstanceGuard` 持有打开的文件句柄直至进程退出（Drop 时句柄关闭 →
//! 自动解锁；进程崩溃同样由内核释放，无「死锁」残留）。
//!
//! 实现取舍（Task 7 依赖纪律）：未用 tauri-plugin-single-instance——其 2.4.3
//! rust-version=1.77.2 ≤ 项目 MSRV 1.89.0（MSRV 本身兼容），但加入后重解析
//! 依赖树会把 tauri-runtime 的 toml 从 0.9.12 升到 1.1.0（rust-version=1.85
//! ≤ MSRV 1.89.0，MSRV 可接受）——锁文件整体变动。改用依赖树中已有的 fs2
//! 0.4.3（零新增依赖）统一实现：Windows 与 Unix 语义一致——同进程第二个
//! fd 加锁同样被拒（LockFileEx / flock 均非重入），与旧命名互斥体语义相同，
//! 可在单测中验证（同进程第二次 acquire 返回 None——与真实双开同一语义）。

use crate::tr;

/// 单实例守卫：持有已加锁的文件句柄，进程存活期间保持打开
/// （Drop 时句柄关闭 → flock/LockFileEx 自动解锁；进程崩溃同样由内核
/// 释放，无「死锁」残留）。
pub struct InstanceGuard {
    _file: std::fs::File,
}

/// 锁文件目录：
/// - Windows：`dirs::cache_dir()/missevan-recorder`（如 `%LOCALAPPDATA%\...\cache`）；
/// - Linux/Unix：`$XDG_RUNTIME_DIR/missevan-recorder`（未设置时回退
///   `~/.cache/missevan-recorder`；HOME 也未设置时返回 None → acquire 视为
///   获取失败 → 调用方退出，fail-closed 与旧行为一致）。
fn lock_dir() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        return dirs::cache_dir().map(|p| p.join("missevan-recorder"));
    }
    #[cfg(not(windows))]
    {
        if let Some(xdg) = std::env::var_os("XDG_RUNTIME_DIR") {
            if !xdg.is_empty() {
                return Some(std::path::PathBuf::from(xdg).join("missevan-recorder"));
            }
        }
        std::env::var_os("HOME").map(|h| {
            std::path::PathBuf::from(h)
                .join(".cache")
                .join("missevan-recorder")
        })
    }
}

/// 尝试获取单实例锁。
///
/// - `Some(guard)`：本实例获得锁（首个实例）；guard 需存活到进程退出
/// - `None`：另一实例已在运行（文件锁被占用 WouldBlock），或无法创建/打开
///   锁文件——调用方应退出（fail-closed，与旧行为一致）
///
/// `name` 用作锁文件名（`{name}.lock`），便于同一目录内多用途锁隔离。
pub fn acquire(name: &str) -> Option<InstanceGuard> {
    use fs2::FileExt;
    let dir = lock_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{name}.lock"));
    // create + read/write：锁文件须真实存在（flock/LockFileEx 对只读打开的 fd
    // 也可加锁，但保持读写打开以兼容后续需要写内容的场景）
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
        .ok()?;
    match file.try_lock_exclusive() {
        Ok(()) => Some(InstanceGuard { _file: file }),
        Err(e) => {
            if e.kind() == std::io::ErrorKind::WouldBlock {
                tracing::info!("{}", tr!("app.single_instance_busy", path = path.display()));
            } else {
                tracing::warn!(
                    "{}",
                    tr!("app.single_instance_failed", path = path.display(), err = e)
                );
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// 进程内唯一锁名（并行测试隔离）：锁文件落在真实锁目录中但名字唯一，
    /// 与生产锁 `missevan-recorder-single-instance.lock` 互不干扰；
    /// Windows 无法经环境变量重定向 dirs::cache_dir（Known Folder API），
    /// 故不依赖目录隔离，仅依赖锁名唯一。
    fn unique_name(tag: &str) -> String {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        format!(
            "missevan-test-single-instance-{}-{}-{}",
            tag,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        )
    }

    /// 清理测试锁文件（与 acquire 内部路径计算一致；目录保留——应用启动
    /// 本就会创建它）
    fn cleanup(name: &str) {
        if let Some(dir) = lock_dir() {
            let _ = std::fs::remove_file(dir.join(format!("{name}.lock")));
        }
    }

    #[test]
    fn second_lock_rejected_while_first_guard_alive() {
        let name = unique_name("lock");
        // 首个实例：获取成功（锁文件被创建）
        let guard = acquire(&name).expect("首次获取单实例锁应成功");
        // 第二「实例」：同一文件再次加锁 → WouldBlock → None
        //（与真实双开同一语义；Windows LockFileEx / Unix flock 同进程第二个
        //  fd 均互斥，测试可全平台跑）
        assert!(
            acquire(&name).is_none(),
            "守卫存活期间再次获取必须被拒绝"
        );
        drop(guard);
        // 句柄释放后自动解锁：可再次获取
        let guard2 = acquire(&name).expect("释放后应可再次获取");
        drop(guard2);
        cleanup(&name);
    }

    #[test]
    fn distinct_names_do_not_conflict() {
        let a = unique_name("a");
        let b = unique_name("b");
        let guard_a = acquire(&a).expect("首次获取 a 应成功");
        // 不同名字的锁文件互不影响
        let guard_b = acquire(&b).expect("不同名字的锁文件不应冲突");
        drop(guard_a);
        drop(guard_b);
        cleanup(&a);
        cleanup(&b);
    }
}
