//! 检测循环运行统计（Task 15）
//!
//! `DetectionLoop` 每轮/每次主播检测更新原子计数器；调试页「检测循环」模块
//! 通过 `get_detector_stats` 读取快照。计数口径：
//! - `total/success/failed`：按**单次主播检测**计（mock 模式同样计数）
//! - `unknown_checks`：状态「未知」次数（Server/Network/Format 错误、429 冷却跳过）；
//!   计入 `failed_checks`（规格：未知计入 stats 失败数），另单独计数便于观察
//! - `last_check_at`：每轮检测开始时间
//!
//! `enabled_anchors / live_anchors / recording_anchors` 由命令层从
//! 配置 / live_cache / AppState 实时聚合（snapshot 中默认为 0）。

use chrono::Utc;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};

/// 检测统计快照（`get_detector_stats` 返回值）
#[derive(Debug, Clone, Serialize)]
pub struct DetectorStatsSnapshot {
    /// 检测循环是否运行中
    pub running: bool,
    /// 上次检测开始时间（RFC3339）；从未检测 = None
    pub last_check_at: Option<String>,
    /// 主播检测总次数
    pub total_checks: u64,
    /// 成功次数
    pub success_checks: u64,
    /// 失败次数（含「未知」；明确失败 + 未知均计入）
    pub failed_checks: u64,
    /// 状态「未知」次数（Server/Network/Format 错误、429 冷却跳过；计入 failed_checks）
    pub unknown_checks: u64,
    /// 启用检测的主播数（命令层从配置聚合）
    pub enabled_anchors: usize,
    /// 直播中的主播数（命令层从 live_cache 聚合）
    pub live_anchors: usize,
    /// 录制中的主播数（命令层从 AppState 聚合）
    pub recording_anchors: usize,
}

/// 检测循环计数器（原子，无锁读写）
#[derive(Debug, Default)]
pub struct DetectorStats {
    running: AtomicBool,
    /// epoch 毫秒；0 = 从未检测
    last_check_at_ms: AtomicI64,
    total_checks: AtomicU64,
    success_checks: AtomicU64,
    failed_checks: AtomicU64,
    unknown_checks: AtomicU64,
}

impl DetectorStats {
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置循环运行标志
    pub fn set_running(&self, running: bool) {
        self.running.store(running, Ordering::Relaxed);
    }

    /// 一轮检测开始（记录上次检测时间）
    pub fn mark_round_started(&self) {
        self.last_check_at_ms
            .store(Utc::now().timestamp_millis(), Ordering::Relaxed);
    }

    /// 一次主播检测开始（总次数 +1）
    pub fn record_check_started(&self) {
        self.total_checks.fetch_add(1, Ordering::Relaxed);
    }

    /// 一次主播检测成功
    pub fn record_check_success(&self) {
        self.success_checks.fetch_add(1, Ordering::Relaxed);
    }

    /// 一次主播检测失败
    pub fn record_check_failed(&self) {
        self.failed_checks.fetch_add(1, Ordering::Relaxed);
    }

    /// 一次主播检测状态「未知」（Server/Network/Format 错误、429 冷却跳过）：
    /// 计入失败数（规格：未知计入 stats 失败数），并单独计数便于观察
    pub fn record_check_unknown(&self) {
        self.failed_checks.fetch_add(1, Ordering::Relaxed);
        self.unknown_checks.fetch_add(1, Ordering::Relaxed);
    }

    /// 清零计数（保留 running 标志）
    pub fn reset(&self) {
        self.total_checks.store(0, Ordering::Relaxed);
        self.success_checks.store(0, Ordering::Relaxed);
        self.failed_checks.store(0, Ordering::Relaxed);
        self.unknown_checks.store(0, Ordering::Relaxed);
        self.last_check_at_ms.store(0, Ordering::Relaxed);
    }

    /// 当前快照（聚合字段由调用方填充）
    pub fn snapshot(&self) -> DetectorStatsSnapshot {
        let ms = self.last_check_at_ms.load(Ordering::Relaxed);
        DetectorStatsSnapshot {
            running: self.running.load(Ordering::Relaxed),
            last_check_at: (ms > 0).then(|| {
                chrono::DateTime::from_timestamp_millis(ms)
                    .map(|d| d.to_rfc3339())
                    .unwrap_or_default()
            }),
            total_checks: self.total_checks.load(Ordering::Relaxed),
            success_checks: self.success_checks.load(Ordering::Relaxed),
            failed_checks: self.failed_checks.load(Ordering::Relaxed),
            unknown_checks: self.unknown_checks.load(Ordering::Relaxed),
            enabled_anchors: 0,
            live_anchors: 0,
            recording_anchors: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_zero_and_not_running() {
        let stats = DetectorStats::new();
        let snap = stats.snapshot();
        assert!(!snap.running);
        assert_eq!(snap.last_check_at, None);
        assert_eq!(snap.total_checks, 0);
        assert_eq!(snap.success_checks, 0);
        assert_eq!(snap.failed_checks, 0);
        assert_eq!(snap.unknown_checks, 0);
    }

    #[test]
    fn counts_success_and_failure() {
        let stats = DetectorStats::new();
        stats.set_running(true);
        stats.mark_round_started();
        stats.record_check_started();
        stats.record_check_success();
        stats.record_check_started();
        stats.record_check_success();
        stats.record_check_started();
        stats.record_check_failed();

        let snap = stats.snapshot();
        assert!(snap.running);
        assert!(snap.last_check_at.is_some());
        assert_eq!(snap.total_checks, 3);
        assert_eq!(snap.success_checks, 2);
        assert_eq!(snap.failed_checks, 1);
        assert_eq!(snap.unknown_checks, 0);
    }

    #[test]
    fn unknown_counts_into_failed_and_its_own_counter() {
        let stats = DetectorStats::new();
        stats.record_check_started();
        stats.record_check_unknown();
        stats.record_check_started();
        stats.record_check_failed();
        stats.record_check_started();
        stats.record_check_unknown();

        let snap = stats.snapshot();
        assert_eq!(snap.total_checks, 3);
        // 规格：未知计入失败数
        assert_eq!(snap.failed_checks, 3);
        assert_eq!(snap.unknown_checks, 2);
        assert_eq!(snap.success_checks, 0);
    }

    #[test]
    fn reset_clears_counts_keeps_running_flag() {
        let stats = DetectorStats::new();
        stats.set_running(true);
        stats.mark_round_started();
        stats.record_check_started();
        stats.record_check_success();
        stats.record_check_unknown();

        stats.reset();
        let snap = stats.snapshot();
        assert!(snap.running);
        assert_eq!(snap.last_check_at, None);
        assert_eq!(snap.total_checks, 0);
        assert_eq!(snap.success_checks, 0);
        assert_eq!(snap.failed_checks, 0);
        assert_eq!(snap.unknown_checks, 0);
    }
}
