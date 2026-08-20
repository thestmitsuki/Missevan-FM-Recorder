//! 磁盘空间保护与录制崩溃熔断（S2/S3 修复共用模块）
//!
//! - `check_disk_space`：录制启动前（engine.rs S2a）与运行路径定期（loop.rs
//!   每检测轮 / monitor.rs 每 5 分钟）共用的低开销磁盘阈值检查（fs2 单次
//!   statfs / GetDiskFreeSpaceEx），阈值 `disk_space_limit_gb` 语义与手动健康
//!   检查（checker/checks.rs DiskSpaceCheck）一致：**0 = 不限制**。
//! - `CrashBackoff`：同一主播连续崩溃的熔断退避（S2b）——连续异常退出达到
//!   阈值后暂停自动重启，指数退避至上限。
//! - `DiskNotifyThrottle`：DISK 通知节流（S2a/S3 共用，冷却期内不重复发送，
//!   避免磁盘不足期间通知刷屏）。

use std::time::{SystemTime, UNIX_EPOCH};

/// 1 GiB（与 checker/checks.rs 原实现一致，1024³）
pub const GB: u64 = 1024 * 1024 * 1024;

/// 磁盘空间检查结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskSpaceStatus {
    /// 空间充足（或阈值 0 = 不限制；此时 available_gb 无意义，恒为 0）
    Ok { available_gb: u64 },
    /// 剩余空间低于阈值（仅当 threshold_gb > 0 时出现）
    Low { available_gb: u64, threshold_gb: u64 },
    /// 查询失败（如路径所在卷不可达）；调用方按放行处理（与健康检查 Warning
    /// 同语义——探测失败不误伤录制）
    QueryFailed(String),
}

/// 查询目录所在卷的可用空间（字节）。失败返回 Err（调用方决定放行/告警）。
pub fn available_space_bytes(dir: &str) -> Result<u64, String> {
    fs2::available_space(std::path::Path::new(dir)).map_err(|e| e.to_string())
}

/// 阈值判断（纯函数，便于单测）：`threshold_gb == 0` = 不限制；否则按字节
/// 精确比较。与旧实现（GB 截断后比较）数学等价且更精确——截断单调，故
/// `floor(a/GB) < t ⇔ a < t·GB`（t ≥ 0），无行为变化。
pub fn below_threshold(available_bytes: u64, threshold_gb: u64) -> bool {
    threshold_gb > 0 && available_bytes < threshold_gb.saturating_mul(GB)
}

/// 低开销磁盘检查（S2a/S3 共用）：阈值 0 = 不限制（直接放行，无系统调用）；
/// 查询失败返回 QueryFailed（放行语义）。检查点是同步单次系统调用，可在异步
/// 上下文中直接调用（与 checker/checks.rs 既有用法一致）。
pub fn check_disk_space(output_dir: &str, threshold_gb: u64) -> DiskSpaceStatus {
    if threshold_gb == 0 {
        return DiskSpaceStatus::Ok { available_gb: 0 };
    }
    match available_space_bytes(output_dir) {
        Ok(bytes) => {
            let available_gb = bytes / GB;
            if below_threshold(bytes, threshold_gb) {
                DiskSpaceStatus::Low {
                    available_gb,
                    threshold_gb,
                }
            } else {
                DiskSpaceStatus::Ok { available_gb }
            }
        }
        Err(e) => DiskSpaceStatus::QueryFailed(e),
    }
}

// ── S2b：崩溃熔断退避 ──

/// 连续崩溃熔断阈值：连续 N 次异常退出后暂停自动重启
pub const CRASH_BACKOFF_THRESHOLD: u32 = 3;
/// 熔断退避基线（秒）：达到阈值起 60s 起，指数增长
pub const CRASH_BACKOFF_BASE_SECS: u64 = 60;
/// 熔断退避上限（秒）：5 分钟
pub const CRASH_BACKOFF_MAX_SECS: u64 = 300;

/// 崩溃熔断退避时长（毫秒，纯函数）：连续次数 < 3 不熔断；
/// 第 3 次起 60s / 120s / 240s 指数增长，第 6 次起封顶 5 分钟。
pub fn crash_cooldown_ms(consecutive: u32) -> u64 {
    if consecutive < CRASH_BACKOFF_THRESHOLD {
        return 0;
    }
    let shift = (consecutive - CRASH_BACKOFF_THRESHOLD).min(3);
    let secs = CRASH_BACKOFF_BASE_SECS
        .checked_shl(shift)
        .unwrap_or(u64::MAX)
        .min(CRASH_BACKOFF_MAX_SECS);
    secs * 1000
}

/// 单主播录制崩溃熔断状态（S2b；跨检测轮次/录制任务共享，存于 AppState）
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CrashBackoff {
    /// 连续异常退出次数（正常结束 / 稳定运行探活成功 / 手动操作清零）
    pub consecutive: u32,
    /// 熔断截止 epoch 毫秒；0 = 未熔断
    pub blocked_until_ms: i64,
}

impl CrashBackoff {
    /// 是否处于熔断退避期
    pub fn is_blocked(&self, now_ms: i64) -> bool {
        now_ms < self.blocked_until_ms
    }

    /// 记录一次崩溃：连续次数 +1；达到阈值后按指数退避设置熔断截止
    pub fn record_crash(&mut self, now_ms: i64) {
        self.consecutive = self.consecutive.saturating_add(1);
        if self.consecutive >= CRASH_BACKOFF_THRESHOLD {
            self.blocked_until_ms =
                now_ms.saturating_add(crash_cooldown_ms(self.consecutive) as i64);
        }
    }

    /// 恢复（正常结束 / 稳定运行探活成功 / 手动操作）：清零计数与熔断
    pub fn record_success(&mut self) {
        self.consecutive = 0;
        self.blocked_until_ms = 0;
    }
}

// ── DISK 通知节流 ──

/// DISK 通知冷却期（毫秒）：10 分钟——磁盘不足期间同类通知不刷屏
pub const DISK_NOTIFY_COOLDOWN_MS: i64 = 10 * 60 * 1000;

/// DISK 通知节流（S2a 启动前拒绝 / S3 定期预警共用，存于 AppState）
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiskNotifyThrottle {
    last_notify_ms: i64,
}

impl DiskNotifyThrottle {
    /// 冷却期内 → false（不重复发送）
    pub fn should_notify(&self, now_ms: i64) -> bool {
        now_ms.saturating_sub(self.last_notify_ms) >= DISK_NOTIFY_COOLDOWN_MS
    }

    /// 记录一次已发送（原子配合 should_notify 使用，见 AppState::disk_notify_allowed）
    pub fn mark_notified(&mut self, now_ms: i64) {
        self.last_notify_ms = now_ms;
    }
}

/// 当前 epoch 毫秒（熔断/节流判断的统一时钟入口）
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 阈值判断 ──

    #[test]
    fn below_threshold_disabled_when_zero() {
        // 0 = 不限制：无论剩余多少都放行
        assert!(!below_threshold(0, 0));
        assert!(!below_threshold(u64::MAX, 0));
    }

    #[test]
    fn below_threshold_byte_precise() {
        // 阈值 10 GB：剩余 9.99 GB → 不足；恰好 10 GB → 充足
        assert!(below_threshold(10 * GB - 1, 10));
        assert!(!below_threshold(10 * GB, 10));
        assert!(!below_threshold(50 * GB, 10));
        // 饱和乘法不溢出：阈值取 u64::MAX GB → 任何真实剩余都不足
        assert!(!below_threshold(u64::MAX, u64::MAX)); // 恰好等于（饱和后）→ 充足
        assert!(below_threshold(u64::MAX - 1, u64::MAX)); // 少 1 字节 → 不足
    }

    #[test]
    fn below_threshold_equivalent_to_legacy_gb_truncation() {
        // 与 checks.rs 旧实现（available/GB < threshold）数学等价性回归：
        // 截断单调，floor(a/GB) < t ⇔ a < t·GB（t ≥ 0）
        let gb = GB;
        for &(avail_gb, threshold) in &[
            (9u64, 10u64),
            (10, 10),
            (11, 10),
            (0, 10),
            (5, 5),
            (4, 5),
            (100, 1),
            (10, 0),
        ] {
            for offset in [0u64, 1, gb - 1, 123] {
                let bytes = avail_gb.saturating_mul(gb).saturating_add(offset);
                let legacy = avail_gb < threshold; // 旧实现：截断 GB 比较
                assert_eq!(
                    below_threshold(bytes, threshold),
                    legacy,
                    "avail_gb={} threshold={} offset={}",
                    avail_gb,
                    threshold,
                    offset
                );
            }
        }
    }

    #[test]
    fn check_disk_space_disabled_when_threshold_zero() {
        // 阈值 0 = 不限制：不发起系统调用，路径无效也放行（跨平台确定性）
        assert_eq!(
            check_disk_space("Z:/nonexistent_xyz_123", 0),
            DiskSpaceStatus::Ok { available_gb: 0 }
        );
    }

    #[test]
    fn available_space_bytes_real_dir_ok() {
        // 系统调用路径可用性：真实目录查询成功（跨平台确定性）
        let dir = std::env::temp_dir();
        assert!(available_space_bytes(&dir.to_string_lossy()).is_ok());
    }

    #[test]
    fn check_disk_space_real_dir_reports_low_with_huge_threshold() {
        // 真实目录 + 极大阈值 → 确定性判定 Low
        let dir = std::env::temp_dir();
        match check_disk_space(&dir.to_string_lossy(), u64::MAX) {
            DiskSpaceStatus::Low { threshold_gb, .. } => assert_eq!(threshold_gb, u64::MAX),
            other => panic!("应判定为 Low，实际: {:?}", other),
        }
    }

    // ── 崩溃熔断退避（S2b）──

    #[test]
    fn crash_cooldown_zero_below_threshold() {
        assert_eq!(crash_cooldown_ms(0), 0);
        assert_eq!(crash_cooldown_ms(1), 0);
        assert_eq!(crash_cooldown_ms(2), 0);
    }

    #[test]
    fn crash_cooldown_grows_exponentially_then_caps() {
        // 第 3 次起 60s / 120s / 240s，第 6 次起封顶 5 分钟
        assert_eq!(crash_cooldown_ms(3), 60_000);
        assert_eq!(crash_cooldown_ms(4), 120_000);
        assert_eq!(crash_cooldown_ms(5), 240_000);
        assert_eq!(crash_cooldown_ms(6), 300_000);
        assert_eq!(crash_cooldown_ms(10), 300_000);
    }

    #[test]
    fn crash_backoff_blocks_after_threshold_and_expires() {
        let mut b = CrashBackoff::default();
        let now = 1_000_000i64;
        assert!(!b.is_blocked(now));
        // 第 1、2 次崩溃：未达阈值，不熔断
        b.record_crash(now);
        b.record_crash(now);
        assert!(!b.is_blocked(now), "未达阈值不熔断");
        // 第 3 次崩溃 → 熔断 60s
        b.record_crash(now);
        assert!(b.is_blocked(now));
        assert!(b.is_blocked(now + 59_999), "熔断期内仍阻止");
        assert!(!b.is_blocked(now + 60_000), "退避到期后放行");
        // 第 4 次崩溃 → 退避翻倍 120s
        b.record_crash(now + 60_000);
        assert!(b.is_blocked(now + 60_000 + 119_999));
        assert!(!b.is_blocked(now + 60_000 + 120_000));
        assert_eq!(b.consecutive, 4);
    }

    #[test]
    fn crash_backoff_success_resets() {
        let mut b = CrashBackoff::default();
        let now = 5_000_000i64;
        b.record_crash(now);
        b.record_crash(now);
        b.record_crash(now);
        assert!(b.is_blocked(now));
        // 恢复：清零计数与熔断
        b.record_success();
        assert!(!b.is_blocked(now));
        assert_eq!(b.consecutive, 0);
        assert_eq!(b.blocked_until_ms, 0);
    }

    // ── DISK 通知节流 ──

    #[test]
    fn disk_throttle_blocks_within_cooldown() {
        let mut t = DiskNotifyThrottle::default();
        let now = 2_000_000i64;
        assert!(t.should_notify(now), "首次应放行");
        t.mark_notified(now);
        assert!(!t.should_notify(now + 1), "冷却期内阻止");
        assert!(!t.should_notify(now + DISK_NOTIFY_COOLDOWN_MS - 1));
        assert!(t.should_notify(now + DISK_NOTIFY_COOLDOWN_MS), "冷却到期放行");
    }
}
