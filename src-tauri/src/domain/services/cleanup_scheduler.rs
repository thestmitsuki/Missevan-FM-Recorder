//! 自动清理定时调度（§11.1 `auto_cleanup_enabled` / `cleanup_time` 接线）。
//!
//! `run_cleanup_now` 命令的清理逻辑早已完整；本模块补上「定时」：
//! 后端启动（setup）与配置保存（save_config）时调用 `reschedule`——
//! 启用 → spawn 每日任务，等到 `cleanup_time`（"HH:MM"）执行一次清理
//!（与命令同逻辑：跳过录制中的文件、完成后刷新文件缓存并 emit
//! `recording_files_changed`）；禁用/配置变更 → 取消旧任务按新配置重建。
//!
//! 实现取舍：
//! - 每次醒来（等待结束）重新读取最新配置，保留天数/总量上限/时间即使
//!   未触发 reschedule 也按最新值执行（reschedule 只负责开关与时间窗）；
//! - `cleanup_time` 非法（非 "HH:MM"）→ 记 warn 并退出循环（下次保存配置
//!   或应用重启时按新值重试），不影响应用其余功能；
//! - 时间计算纯函数（`parse_cleanup_time` / `seconds_until_occurrence`）可单测；
//!   跨天由「今天未到 → 今天；已过 → 明天同一时刻」保证。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::Manager;
use tokio_util::sync::CancellationToken;

use crate::domain::config::manager::ConfigManager;
use crate::domain::services::cleanup::run_cleanup;
use crate::domain::services::file_cache::FileCacheHandle;
use crate::infrastructure::state::app_state::RecorderState;

/// 每日定时清理调度器（tauri manage 注册；save_config / 启动时 reschedule 重建）。
#[derive(Default)]
pub struct CleanupScheduler {
    /// 当前任务取消令牌（None = 未在调度）
    cancel: Mutex<Option<CancellationToken>>,
}

impl CleanupScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    /// 按配置重建调度：先取消旧任务；启用时 spawn 新每日任务。
    ///
    /// 任务持有 AppHandle，运行时经 tauri state 取配置/缓存/任务表，
    /// 与 `run_cleanup_now` 命令共用同一清理实现。
    pub fn reschedule(&self, app: tauri::AppHandle) {
        if let Some(old) = self.cancel.lock().unwrap_or_else(|e| e.into_inner()).take() {
            old.cancel();
        }
        let token = CancellationToken::new();
        *self.cancel.lock().unwrap_or_else(|e| e.into_inner()) = Some(token.clone());
        tracing::info!("自动清理调度已重建（是否启用由任务内读配置决定）");
        tauri::async_runtime::spawn(async move {
            cleanup_loop(app, token).await;
        });
    }
}

/// 每日清理循环：等待到下一个 `cleanup_time` → 执行清理 → 继续等下一轮。
async fn cleanup_loop(app: tauri::AppHandle, token: CancellationToken) {
    loop {
        // 每轮读最新配置（reschedule 已处理开关与时间窗变化，此处兜底 + 取参数）
        let config = match app.state::<Arc<ConfigManager>>().inner().load() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("自动清理：读取配置失败，调度退出: {}", e);
                return;
            }
        };
        if !config.global.auto_cleanup_enabled {
            tracing::info!("自动清理已禁用，调度退出");
            return;
        }
        let Some(delay_secs) =
            seconds_until_occurrence(&config.global.cleanup_time, chrono::Local::now())
        else {
            tracing::warn!(
                "自动清理：cleanup_time 格式无效（应为 HH:MM）: {}，调度退出",
                config.global.cleanup_time
            );
            return;
        };
        tracing::info!("自动清理：{} 秒后执行（{}）", delay_secs, config.global.cleanup_time);
        tokio::select! {
            _ = token.cancelled() => return,
            _ = tokio::time::sleep(Duration::from_secs(delay_secs)) => {}
        }
        // 到点：执行清理（内部刷新文件缓存并 emit recording_files_changed）
        let Some(window) = app.get_webview_window("main") else {
            tracing::warn!("自动清理：找不到主窗口，调度退出");
            return;
        };
        let cache: FileCacheHandle = app.state::<FileCacheHandle>().inner().clone();
        let app_state = app.state::<RecorderState>().state.clone();
        let config_manager: Arc<ConfigManager> = app.state::<Arc<ConfigManager>>().inner().clone();
        match run_cleanup(window, cache, config_manager, app_state).await {
            Ok(summary) => tracing::info!(
                "自动清理完成: 删除 {} 个文件 / 释放 {} 字节",
                summary.files_deleted,
                summary.bytes_freed
            ),
            Err(e) => tracing::warn!("自动清理执行失败（下轮重试）: {}", e),
        }
        // 回到循环顶部：重新读配置并等待下一个（今天已过 → 明天同一时刻）
    }
}

/// 解析 "HH:MM" → (小时, 分钟)；非法（非数字/越界/多余段）返回 None
pub fn parse_cleanup_time(s: &str) -> Option<(u32, u32)> {
    let mut it = s.trim().split(':');
    let h: u32 = it.next()?.trim().parse().ok()?;
    let m: u32 = it.next()?.trim().parse().ok()?;
    if it.next().is_some() || h > 23 || m > 59 {
        return None;
    }
    Some((h, m))
}

/// 距下一次 `HH:MM` 的秒数：今天该时刻未到 → 今天；已到/已过 → 明天同一时刻。
/// `cleanup_time` 非法 → None。跨天与 DST 差异由 chrono Local 处理。
pub fn seconds_until_occurrence(
    hhmm: &str,
    now: chrono::DateTime<chrono::Local>,
) -> Option<u64> {
    let (h, m) = parse_cleanup_time(hhmm)?;
    let today = now
        .date_naive()
        .and_hms_opt(h, m, 0)?
        .and_local_timezone(chrono::Local)
        .single()?;
    let next = if today > now {
        today
    } else {
        today + chrono::Duration::days(1)
    };
    Some((next - now).num_seconds().max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn parse_cleanup_time_accepts_valid_hhmm() {
        assert_eq!(parse_cleanup_time("03:00"), Some((3, 0)));
        assert_eq!(parse_cleanup_time("00:00"), Some((0, 0)));
        assert_eq!(parse_cleanup_time("23:59"), Some((23, 59)));
        assert_eq!(parse_cleanup_time(" 3:5 "), Some((3, 5)));
    }

    #[test]
    fn parse_cleanup_time_rejects_invalid() {
        for bad in ["", "03", "03:00:00", "25:00", "03:60", "a:b", "03:0x", "03:00:01"] {
            assert_eq!(parse_cleanup_time(bad), None, "应拒绝: {}", bad);
        }
    }

    #[test]
    fn seconds_until_today_if_not_passed() {
        let now = chrono::Local
            .with_ymd_and_hms(2026, 8, 7, 2, 0, 0)
            .unwrap();
        // 今天 03:00 未到 → 1 小时后
        assert_eq!(seconds_until_occurrence("03:00", now), Some(3600));
        // 恰在时刻前 1 秒
        let now2 = chrono::Local
            .with_ymd_and_hms(2026, 8, 7, 2, 59, 59)
            .unwrap();
        assert_eq!(seconds_until_occurrence("03:00", now2), Some(1));
    }

    #[test]
    fn seconds_until_tomorrow_if_passed() {
        let now = chrono::Local
            .with_ymd_and_hms(2026, 8, 7, 4, 0, 0)
            .unwrap();
        // 今天 03:00 已过 → 明天 03:00（23 小时后）
        assert_eq!(seconds_until_occurrence("03:00", now), Some(23 * 3600));
        // 恰在时刻（00 分 00 秒）→ 视作已过，明天同一时刻
        let now2 = chrono::Local
            .with_ymd_and_hms(2026, 8, 7, 3, 0, 0)
            .unwrap();
        assert_eq!(seconds_until_occurrence("03:00", now2), Some(24 * 3600));
    }

    #[test]
    fn seconds_until_invalid_time_returns_none() {
        let now = chrono::Local.with_ymd_and_hms(2026, 8, 7, 12, 0, 0).unwrap();
        assert_eq!(seconds_until_occurrence("99:99", now), None);
        assert_eq!(seconds_until_occurrence("", now), None);
    }
}
