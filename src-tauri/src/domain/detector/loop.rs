use rand::Rng;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::Emitter;
use tokio::sync::{Mutex, Notify, Semaphore};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use crate::domain::config::model::AnchorStatusUpdate;
use crate::domain::config::model::{AnchorConfig, Config};
use crate::domain::detector::merge_live_state;
use crate::domain::detector::stats::DetectorStats;
use crate::domain::spider::{CheckErrorKind, LiveCheckResult, MissevanClient};
use crate::infrastructure::state::app_state::AppState;
use crate::infrastructure::state::mock_store::MockStore;
use tauri::WebviewWindow;

/// 每轮单主播最大请求次数（含首次；Server/Network 类错误指数退避重试，
/// 规格「直播状态异常修复」：5XX/429 与本地网络错误均不判离线）
const MAX_ATTEMPTS: u32 = 3;

/// 重试退避（毫秒）：attempt 从 1 起指数增长（2s / 4s / 8s），上限 8s
fn retry_delay_ms(attempt: u32) -> u64 {
    let shift = attempt.saturating_sub(1).min(2);
    (2000u64 << shift).min(8000)
}

/// 429 冷却时长（毫秒）：指数退避 60s × 2^(n-1)（60s/120s/240s），上限 5 分钟
fn cooldown_ms(consecutive: u32) -> u64 {
    if consecutive == 0 {
        return 0;
    }
    let secs = 60u64
        .checked_mul(1u64 << (consecutive - 1).min(4))
        .unwrap_or(u64::MAX);
    secs.min(300) * 1000
}

/// 单主播 429 限流状态（跨检测轮次共享；连续 429 指数退避冷却）
#[derive(Debug, Clone, Default)]
pub struct RateLimit {
    /// 下次允许请求的 epoch 毫秒
    next_allowed_at_ms: i64,
    /// 连续 429 次数（指数退避基数）
    consecutive: u32,
}

impl RateLimit {
    pub fn is_allowed(&self, now_ms: i64) -> bool {
        now_ms >= self.next_allowed_at_ms
    }

    /// 记录一次 429：连续次数 +1，并延长冷却（指数退避）
    pub fn on_429(&mut self, now_ms: i64) {
        self.consecutive = self.consecutive.saturating_add(1);
        self.next_allowed_at_ms = now_ms + cooldown_ms(self.consecutive) as i64;
    }

    /// 请求成功：清零连续次数与冷却
    pub fn on_success(&mut self) {
        self.consecutive = 0;
        self.next_allowed_at_ms = 0;
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 状态归并 + 推送（规格「直播状态异常修复」双重验证）：
///
/// - 缓存存 **API 判定**（单一事实来源，供调试统计聚合）；
/// - 推送事件的 `is_live` 为 **归并后** 的直播展示状态 = API || 录制中
///   （API 判离线但录制进行中 → 保持「直播中」，避免风控误报翻转显示）；
/// - 仅当 API 判定变化时才推送（「未知」状态不改变缓存、不推送，避免前端闪烁）。
async fn push_merged_status(
    window: &WebviewWindow,
    live_cache: &Arc<Mutex<HashMap<String, bool>>>,
    app_state: &Arc<Mutex<AppState>>,
    anchor_id: &str,
    api_live: bool,
) {
    let is_recording = {
        let state = app_state.lock().await;
        state.tasks.contains_key(anchor_id)
    };
    let merged_live = merge_live_state(api_live, is_recording);

    let mut cache = live_cache.lock().await;
    let old = cache.get(anchor_id).copied().unwrap_or(false);
    cache.insert(anchor_id.to_string(), api_live);
    drop(cache);

    if api_live != old {
        let update = AnchorStatusUpdate {
            anchor_id: anchor_id.to_string(),
            is_live: merged_live,
            is_recording,
        };
        let _ = window.emit("recording_status_changed", &update);
    }
}

pub struct DetectionLoop {
    client: MissevanClient,
    window: WebviewWindow,
    live_cache: Arc<Mutex<HashMap<String, bool>>>,
    app_state: Arc<Mutex<AppState>>,
    avatar_cache: Arc<Mutex<HashMap<String, String>>>,
    mock_store: Arc<MockStore>,
    /// 手动唤醒信号（向导完成、手动刷新等场景触发一次立即检测）
    wake: Arc<Notify>,
    /// 退出信号（Task 17：托盘「退出」/ 主窗关闭时 notify_waiters，循环立即停止）
    shutdown: Arc<Notify>,
    /// 检测统计（调试页「检测循环」模块读取；`trigger_detection_now` 经命令触发）
    pub stats: Arc<DetectorStats>,
    /// 429 限流冷却（anchor_id -> RateLimit；避免频繁请求被风控）
    rate_limits: Arc<Mutex<HashMap<String, RateLimit>>>,
}

impl DetectionLoop {
    pub fn new(
        client: MissevanClient,
        window: WebviewWindow,
        live_cache: Arc<Mutex<HashMap<String, bool>>>,
        app_state: Arc<Mutex<AppState>>,
        avatar_cache: Arc<Mutex<HashMap<String, String>>>,
        mock_store: Arc<MockStore>,
        wake: Arc<Notify>,
        shutdown: Arc<Notify>,
        stats: Arc<DetectorStats>,
    ) -> Self {
        Self {
            client,
            window,
            live_cache,
            app_state,
            avatar_cache,
            mock_store,
            wake,
            shutdown,
            stats,
            rate_limits: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 手动触发一轮立即检测（唤醒等待中的循环；debug 页「立即检测」按钮）
    pub fn trigger_now(&self) {
        self.wake.notify_one();
    }

    pub async fn start(
        &self,
        get_config: impl Fn() -> Config + Send + Sync + 'static,
        start_recording: Arc<
            impl Fn(AnchorConfig, String, CancellationToken) + Send + Sync + 'static,
        >,
    ) {
        self.stats.set_running(true);
        loop {
            let config = get_config();
            // 检测间隔下限 5s（与 model.rs is_valid / 前端校验一致；规格默认 120s）
            let base_interval = config.global.check_interval_secs.max(5);
            // 随机抖动上限来自配置（Task 14：detector_jitter_secs，0 = 不抖动）
            let jitter: u64 =
                rand::thread_rng().gen_range(0..=config.global.detector_jitter_secs as u64);
            let delay = Duration::from_secs(base_interval + jitter);
            // 并发信号量每轮从配置重建（Task 20 收尾：detector_concurrency 运行时接线，
            // 修改后下一轮立即生效，无需重启；max(1) 兜底避免 0 死锁）
            let semaphore =
                Arc::new(Semaphore::new(config.global.detector_concurrency.max(1) as usize));

            // 等待下轮检测；finish_wizard 等场景可通过 wake 信号立即唤醒；
            // 退出信号（Task 17：shutdown_notify.notify_waiters()）到达则立即停止循环
            tokio::select! {
                _ = sleep(delay) => {}
                _ = self.wake.notified() => {}
                _ = self.shutdown.notified() => {
                    tracing::info!("收到退出信号，检测循环停止");
                    self.stats.set_running(false);
                    break;
                }
            }

            let anchors = config.anchors.clone();
            if anchors.is_empty() {
                continue;
            }

            // 本轮检测开始（记录上次检测时间）
            self.stats.mark_round_started();

            let mut handles = Vec::new();
            for anchor in &anchors {
                // 现在对**所有主播**都进行检测（不再受 enable_check 限制）
                let client = self.client.clone();
                let semaphore = semaphore.clone();
                let anchor_clone = anchor.clone();
                let start_recording = Arc::clone(&start_recording);
                let live_cache = self.live_cache.clone();
                let app_state = self.app_state.clone();
                //let app_handle = self.app_handle.clone();
                let avatar_cache = self.avatar_cache.clone();
                let window = self.window.clone();
                let mock_store = self.mock_store.clone();
                let stats = self.stats.clone();
                let rate_limits = self.rate_limits.clone();

                let handle = tokio::spawn(async move {
                    // 每次主播检测计数
                    stats.record_check_started();
                    let _permit = semaphore.acquire().await.unwrap();

                    // —— 429 限流冷却：冷却期内不发起请求，状态视为「未知」——
                    //（不改变缓存、不推送事件，避免前端闪烁；计入统计失败数）
                    let now = now_ms();
                    let rate_limited = {
                        let rl = rate_limits.lock().await;
                        rl.get(&anchor_clone.id)
                            .map(|r| !r.is_allowed(now))
                            .unwrap_or(false)
                    };
                    if rate_limited {
                        stats.record_check_unknown();
                        tracing::warn!(
                            "[检测] 429 冷却中，本轮跳过（保持上一状态）: {} (room_id={})",
                            anchor_clone.name,
                            anchor_clone.room_id
                        );
                        return;
                    }

                    // Mock 模式：不发起真实请求，直接从 MockStore 取模拟结果
                    let result = if mock_store.is_mock_mode() {
                        tracing::debug!(
                            "[Mock] 检测主播 {} (room_id={})",
                            anchor_clone.name,
                            anchor_clone.room_id
                        );
                        match mock_store.get(&anchor_clone.room_id) {
                            Some(mock) => Ok(LiveCheckResult {
                                is_live: mock.is_live,
                                anchor_name: Some(mock.name.clone()),
                                title: None,
                                stream_url: mock.is_live.then_some(mock.stream_url.clone()),
                                avatar: None,
                            }),
                            // 无条目 → 视为离线
                            None => Ok(LiveCheckResult {
                                is_live: false,
                                anchor_name: None,
                                title: None,
                                stream_url: None,
                                avatar: None,
                            }),
                        }
                    } else {
                        // —— 真实检测：错误分类处理（规格「直播状态异常修复」）——
                        // Server(5XX)/Network 类：指数退避重试（最多 MAX_ATTEMPTS 次）；
                        // 429：记录冷却（指数退避 60s×2^(n-1)，上限 5min），本轮放弃重试；
                        // Format：不重试（格式变化重试无意义）；Other：不重试（视为离线）。
                        let mut attempt = 1u32;
                        loop {
                            match client
                                .check_live(&anchor_clone.room_id, anchor_clone.cookie.as_deref())
                                .await
                            {
                                Ok(r) => {
                                    rate_limits
                                        .lock()
                                        .await
                                        .entry(anchor_clone.id.clone())
                                        .or_default()
                                        .on_success();
                                    break Ok(r);
                                }
                                Err(e) => {
                                    if e.status == Some(429) {
                                        let cool_s = {
                                            let mut rl = rate_limits.lock().await;
                                            let entry = rl
                                                .entry(anchor_clone.id.clone())
                                                .or_default();
                                            entry.on_429(now_ms());
                                            cooldown_ms(entry.consecutive) / 1000
                                        };
                                        tracing::warn!(
                                            "[检测] 429 限流，冷却 {}s: {} (room_id={})",
                                            cool_s,
                                            anchor_clone.name,
                                            anchor_clone.room_id
                                        );
                                        break Err(e);
                                    }
                                    let retryable = matches!(
                                        e.kind,
                                        CheckErrorKind::Server | CheckErrorKind::Network
                                    );
                                    if retryable && attempt < MAX_ATTEMPTS {
                                        let delay = retry_delay_ms(attempt);
                                        tracing::warn!(
                                            "[检测] 检测失败({:?})，{}s 后重试 {}/{}: {} (room_id={})",
                                            e.kind,
                                            delay / 1000,
                                            attempt,
                                            MAX_ATTEMPTS - 1,
                                            anchor_clone.name,
                                            anchor_clone.room_id
                                        );
                                        sleep(Duration::from_millis(delay)).await;
                                        attempt += 1;
                                        continue;
                                    }
                                    break Err(e);
                                }
                            }
                        }
                    };

                    match result {
                        Ok(result) => {
                            stats.record_check_success();
                            tracing::info!(
                                "[检测] anchor={} room_id={} is_live={} enable_check={} has_stream={}",
                                anchor_clone.name,
                                anchor_clone.room_id,
                                result.is_live,
                                anchor_clone.enable_check,
                                result.stream_url.is_some()
                            );
                            // 1. 更新头像缓存
                            if let Some(avatar) = &result.avatar {
                                avatar_cache
                                    .lock()
                                    .await
                                    .insert(anchor_clone.id.clone(), avatar.clone());
                            }

                            // 2. 状态归并 + 推送变化（缓存存 API 判定，事件推归并后直播状态）
                            push_merged_status(
                                &window,
                                &live_cache,
                                &app_state,
                                &anchor_clone.id,
                                result.is_live,
                            )
                            .await;

                            // 3. 录制门控：仅 API 判直播时启动（归并仅影响展示，不影响门控）
                            if anchor_clone.enable_check && result.is_live {
                                let already_recording = {
                                    let state = app_state.lock().await;
                                    state.tasks.contains_key(&anchor_clone.id)
                                };
                                if !already_recording {
                                    let cancel = CancellationToken::new();
                                    tracing::info!(
                                        "[检测] 触发自动录制: {} (room_id={})",
                                        anchor_clone.name,
                                        anchor_clone.room_id
                                    );
                                    start_recording(
                                        anchor_clone,
                                        result.stream_url.unwrap_or_default(),
                                        cancel,
                                    );
                                } else {
                                    tracing::debug!(
                                        "[检测] 已在录制中，跳过: {}",
                                        anchor_clone.name
                                    );
                                }
                            }
                        }
                        Err(e) => match e.kind {
                            // 状态「未知」：不改变缓存、不推送事件（避免前端闪烁），
                            // 计入统计失败数，下轮重试
                            CheckErrorKind::Server
                            | CheckErrorKind::Network
                            | CheckErrorKind::Format => {
                                stats.record_check_unknown();
                                tracing::warn!(
                                    "[检测] 状态未知（保持上一状态）: {} (room_id={}): {}",
                                    anchor_clone.name,
                                    anchor_clone.room_id,
                                    e
                                );
                            }
                            // 明确不可用（如 404 房间不存在）：视为离线，走归并推送
                            CheckErrorKind::Other => {
                                stats.record_check_failed();
                                tracing::error!("[检测] API 明确失败，视为离线: {}", e);
                                push_merged_status(
                                    &window,
                                    &live_cache,
                                    &app_state,
                                    &anchor_clone.id,
                                    false,
                                )
                                .await;
                            }
                        },
                    }
                });
                handles.push(handle);
            }

            for handle in handles {
                let _ = handle.await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 重试退避（Server/Network 类错误）──

    #[test]
    fn retry_delay_grows_exponentially_then_caps() {
        assert_eq!(retry_delay_ms(1), 2000);
        assert_eq!(retry_delay_ms(2), 4000);
        assert_eq!(retry_delay_ms(3), 8000);
        // 上限 8s，不再增长
        assert_eq!(retry_delay_ms(4), 8000);
        assert_eq!(retry_delay_ms(10), 8000);
    }

    // ── 429 冷却（指数退避）──

    #[test]
    fn cooldown_grows_exponentially_then_caps_at_5min() {
        assert_eq!(cooldown_ms(0), 0);
        assert_eq!(cooldown_ms(1), 60_000);
        assert_eq!(cooldown_ms(2), 120_000);
        assert_eq!(cooldown_ms(3), 240_000);
        // 上限 5 分钟
        assert_eq!(cooldown_ms(4), 300_000);
        assert_eq!(cooldown_ms(10), 300_000);
    }

    #[test]
    fn rate_limit_blocks_until_cooldown_expires() {
        let mut rl = RateLimit::default();
        let now = 1_000_000i64;
        assert!(rl.is_allowed(now));

        // 第一次 429 → 冷却 60s：now+60s 内禁止，到期后放行
        rl.on_429(now);
        assert!(!rl.is_allowed(now));
        assert!(!rl.is_allowed(now + 59_999));
        assert!(rl.is_allowed(now + 60_000));

        // 连续 429 → 冷却翻倍（120s）
        rl.on_429(now + 60_000);
        assert!(!rl.is_allowed(now + 60_000 + 119_999));
        assert!(rl.is_allowed(now + 60_000 + 120_000));
    }

    #[test]
    fn rate_limit_success_resets_cooldown() {
        let mut rl = RateLimit::default();
        let now = 5_000_000i64;
        rl.on_429(now);
        assert!(!rl.is_allowed(now));

        // 请求成功 → 立即放行
        rl.on_success();
        assert!(rl.is_allowed(now));
        assert_eq!(rl.consecutive, 0);

        // 冷却归零后再次 429 从 60s 重新开始（而非累积）
        rl.on_429(now);
        assert!(rl.is_allowed(now + 60_000));
    }
}
