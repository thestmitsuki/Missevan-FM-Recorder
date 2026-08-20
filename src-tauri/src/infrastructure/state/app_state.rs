use crate::domain::recorder::disk::{now_ms, CrashBackoff, DiskNotifyThrottle};
use crate::infrastructure::state::mock_store::MockStore;

use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// 录制历史上限（最近 N 条结束摘要）
const RECORDING_HISTORY_LIMIT: usize = 50;

/// 录制任务记录
pub struct Task {
    pub anchor_id: String,
    pub cancel_token: CancellationToken,
    pub handle: JoinHandle<()>,
    // —— 调试页「录制引擎」模块展示所需（Task 15 补充）——
    pub anchor_name: String,
    pub room_id: String,
    pub output_path: String,
    pub started_at: std::time::Instant,
    pub pid: Option<u32>,
}

/// 活跃录制任务（`get_recorder_state.active` 元素）
#[derive(Debug, Clone, Serialize)]
pub struct ActiveRecording {
    pub anchor_id: String,
    pub anchor_name: String,
    pub room_id: String,
    /// "recording"
    pub status: String,
    /// 已录时长（秒）
    pub duration_secs: u64,
    pub output_path: String,
    pub pid: Option<u32>,
}

/// 已结束录制摘要（`get_recorder_state.history` 元素）
#[derive(Debug, Clone, Serialize)]
pub struct RecordingSummary {
    pub anchor_id: String,
    pub anchor_name: String,
    pub room_id: String,
    pub output_path: String,
    /// RFC3339
    pub started_at: String,
    /// 录制时长（秒）
    pub duration_secs: u64,
    /// RFC3339
    pub ended_at: String,
}

/// 录制引擎状态快照（`get_recorder_state` 返回值）
#[derive(Debug, Clone, Serialize)]
pub struct RecorderStateInfo {
    pub active: Vec<ActiveRecording>,
    pub history: Vec<RecordingSummary>,
}

/// 应用运行时状态（可共享、可变）
pub struct AppState {
    pub tasks: HashMap<String, Task>,
    pub global_cancel: CancellationToken,
    /// 最近结束的录制摘要（最新在前，上限 50）
    pub history: VecDeque<RecordingSummary>,
    /// pre_record_delay 等待中的自动录制启动（anchor_id → 取消令牌）。
    /// 延迟窗口内任务尚未注册进 `tasks`，stop_recording / remove_anchor /
    /// 「关闭检测」必须先查此集合才能取消"即将启动"的录制（否则延迟结束后
    /// 仍会启动，只能等 monitor 兜底 ≤10s 才停——既有修复注释明确要杜绝的场景）。
    pub pending_starts: HashMap<String, CancellationToken>,
    /// 录制崩溃熔断状态（S2b；monitor 崩溃上报 → loop 自动重启门控查询）。
    /// 同一主播连续异常退出达阈值后暂停自动重启，指数退避至上限。
    pub crash_backoffs: HashMap<String, CrashBackoff>,
    /// DISK 通知节流（S2a 启动前拒绝 / S3 定期预警共用）：磁盘不足期间
    /// 同类通知冷却期内不重复发送，避免通知刷屏。
    pub disk_notify: DiskNotifyThrottle,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            global_cancel: CancellationToken::new(),
            history: VecDeque::new(),
            pending_starts: HashMap::new(),
            crash_backoffs: HashMap::new(),
            disk_notify: DiskNotifyThrottle::default(),
        }
    }

    pub fn is_recording(&self, anchor_id: &str) -> bool {
        self.tasks.contains_key(anchor_id)
    }

    /// 当前录制中的主播 id 集合（快照用途：get_recording_status 在锁内仅取此
    /// 快照即释放，遍历组装在锁外完成——避免长持锁阻塞同锁序操作）
    pub fn recording_anchor_ids(&self) -> HashSet<String> {
        self.tasks.keys().cloned().collect()
    }

    pub fn insert_task(&mut self, anchor_id: String, task: Task) {
        self.tasks.insert(anchor_id, task);
    }

    pub fn remove_task(&mut self, anchor_id: &str) -> Option<Task> {
        self.tasks.remove(anchor_id)
    }

    pub fn active_count(&self) -> usize {
        self.tasks.len()
    }

    /// 注册一个"延迟启动中"的主播（pre_record_delay 窗口；lib.rs 延迟开始前调用）。
    /// 已注册过（检测循环重入）时返回 false——调用方应放弃本次启动，避免同一
    /// 主播叠加多个延迟任务。
    pub fn register_pending_start(&mut self, anchor_id: &str, cancel: CancellationToken) -> bool {
        if self.pending_starts.contains_key(anchor_id) {
            return false;
        }
        self.pending_starts.insert(anchor_id.to_string(), cancel);
        true
    }

    /// 取消一个"延迟启动中"的主播（stop_recording / remove_anchor / 关闭检测
    /// 在延迟窗口内调用）：移除注册并触发取消令牌（lib.rs 的 select! 据此醒来
    /// 放弃启动）。返回是否确实存在待取消的启动。
    pub fn cancel_pending_start(&mut self, anchor_id: &str) -> bool {
        match self.pending_starts.remove(anchor_id) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    /// 移除"延迟启动中"的注册（不触发取消令牌）——lib.rs 延迟结束后清理用。
    pub fn remove_pending_start(&mut self, anchor_id: &str) -> bool {
        self.pending_starts.remove(anchor_id).is_some()
    }

    /// 该主播是否处于"延迟启动中"
    pub fn is_pending_start(&self, anchor_id: &str) -> bool {
        self.pending_starts.contains_key(anchor_id)
    }

    /// 记录一条结束的录制摘要（最新在前，超限丢最旧）
    pub fn record_history(&mut self, summary: RecordingSummary) {
        self.history.push_front(summary);
        if self.history.len() > RECORDING_HISTORY_LIMIT {
            self.history.pop_back();
        }
    }

    /// 活跃录制任务的输出路径集合（路径键归一化：`\` 与 `/` 视为相同）。
    /// 文件缓存据此标记「录制中」文件（is_active），前端禁删/禁重命名。
    pub fn active_output_paths(&self) -> std::collections::HashSet<String> {
        self.tasks
            .values()
            .map(|t| crate::domain::services::file_cache::path_key(&t.output_path))
            .collect()
    }

    // ── S2b：录制崩溃熔断（monitor 上报 / loop 门控 / monitor 恢复）──

    /// 上报一次录制崩溃：连续次数 +1，达到阈值后按指数退避设置熔断（loop 门控
    /// 据此暂停自动重启）。返回连续崩溃次数（调用方日志展示用）
    pub fn record_crash(&mut self, anchor_id: &str) -> u32 {
        let entry = self.crash_backoffs.entry(anchor_id.to_string()).or_default();
        entry.record_crash(now_ms());
        entry.consecutive
    }

    /// 恢复（正常结束 / 稳定运行探活成功 / 手动操作）：清零计数与熔断
    pub fn reset_crash(&mut self, anchor_id: &str) {
        if let Some(b) = self.crash_backoffs.get_mut(anchor_id) {
            b.record_success();
        }
    }

    /// 该主播是否处于崩溃熔断退避期（loop 自动重启门控查询）
    pub fn is_crash_blocked(&self, anchor_id: &str) -> bool {
        self.crash_backoffs
            .get(anchor_id)
            .is_some_and(|b| b.is_blocked(now_ms()))
    }

    /// 连续崩溃次数（调试/日志展示）
    pub fn crash_count(&self, anchor_id: &str) -> u32 {
        self.crash_backoffs.get(anchor_id).map(|b| b.consecutive).unwrap_or(0)
    }

    // ── R3/L10：按主播清理运行时状态（remove_anchor 路径调用）──

    /// 清理该主播的全部按主播维度运行时状态（主播删除时调用，避免长期运行
    /// 累积无主条目）：
    /// - `crash_backoffs` 崩溃熔断条目（此前仅复位计数，条目随删除回收）；
    /// - `pending_starts` 延迟启动注册（复用 `cancel_pending_start`：若删除时
    ///   仍在 pre_record_delay 窗口则一并取消并移除，防已删主播启动录制）。
    /// 全局维度状态（`tasks` / `history` / `disk_notify`）不在此清理；存于
    /// AppState 之外的按主播状态（429 限流 / 头像正负缓存）由各自模块的清理
    /// 方法处理（DetectionLoop::prune_anchor_state / anchor_cmds::remove_anchor）。
    /// 返回是否清理了任何条目（调用方按需记录日志）。
    pub fn prune_anchor(&mut self, anchor_id: &str) -> bool {
        let mut pruned = false;
        pruned |= self.crash_backoffs.remove(anchor_id).is_some();
        pruned |= self.cancel_pending_start(anchor_id);
        pruned
    }

    // ── DISK 通知节流（S2a/S3 共用）──

    /// 是否允许发送 DISK 通知：冷却期外放行并立即标记已发送（原子配合，避免
    /// 并发调用方重复通知）
    pub fn disk_notify_allowed(&mut self) -> bool {
        let now = now_ms();
        if self.disk_notify.should_notify(now) {
            self.disk_notify.mark_notified(now);
            true
        } else {
            false
        }
    }
}

/// 应用状态句柄（Arc<Mutex<AppState>>）
pub type AppStateHandle = Arc<Mutex<AppState>>;

pub fn create_app_state_handle() -> AppStateHandle {
    Arc::new(Mutex::new(AppState::new()))
}

/// Tauri State 包装（通过 app.manage() 注册）
#[derive(Clone)]
pub struct RecorderState {
    pub state: AppStateHandle,
    pub mock_mode: Arc<AtomicBool>,
    pub mock_store: Arc<MockStore>,
    pub app_handle: Arc<Mutex<Option<tauri::AppHandle>>>,
    pub shutdown_notify: Arc<Notify>,
}

impl RecorderState {
    pub fn new(mock_store: Arc<MockStore>) -> Self {
        Self {
            state: create_app_state_handle(),
            mock_mode: Arc::new(AtomicBool::new(false)),
            mock_store,
            app_handle: Arc::new(Mutex::new(None)),
            shutdown_notify: Arc::new(Notify::new()),
        }
    }

    /// 在 setup() 中调用，注入 AppHandle
    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        *self.app_handle.blocking_lock() = Some(handle);
    }
}

pub type AvatarCache = Arc<Mutex<HashMap<String, String>>>;

/// 头像拉取失败负缓存（O2）：主播 id → 禁止重试截止时刻（单调时钟 Instant）。
/// 拉取失败的主播在 TTL（见 anchor_cmds.rs `AVATAR_NEGATIVE_CACHE_TTL`）内
/// 不再发起网络请求，避免 API 抖动期每次 `get_anchors` 都重试全部失败项。
pub type AvatarNegativeCache = Arc<Mutex<HashMap<String, std::time::Instant>>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(anchor_id: &str, duration_secs: u64) -> RecordingSummary {
        RecordingSummary {
            anchor_id: anchor_id.to_string(),
            anchor_name: format!("主播{}", anchor_id),
            room_id: format!("room-{}", anchor_id),
            output_path: format!("/out/{}.m4a", anchor_id),
            started_at: "2026-08-01T00:00:00Z".to_string(),
            duration_secs,
            ended_at: "2026-08-01T01:00:00Z".to_string(),
        }
    }

    #[test]
    fn record_history_newest_first() {
        let mut state = AppState::new();
        state.record_history(summary("a", 10));
        state.record_history(summary("b", 20));
        assert_eq!(state.history.len(), 2);
        assert_eq!(state.history[0].anchor_id, "b");
        assert_eq!(state.history[1].anchor_id, "a");
        assert_eq!(state.history[0].duration_secs, 20);
    }

    #[tokio::test]
    async fn recording_anchor_ids_snapshots_tasks() {
        let mut state = AppState::new();
        assert!(state.recording_anchor_ids().is_empty(), "无任务时返回空集");
        let token1 = CancellationToken::new();
        let handle1 = tokio::spawn(async {});
        state.insert_task(
            "a1".to_string(),
            Task {
                anchor_id: "a1".to_string(),
                cancel_token: token1,
                handle: handle1,
                anchor_name: "A1".to_string(),
                room_id: "r1".to_string(),
                output_path: "/out/a1.m4a".to_string(),
                started_at: std::time::Instant::now(),
                pid: None,
            },
        );
        let ids = state.recording_anchor_ids();
        assert_eq!(ids.len(), 1, "仅返回任务表内的主播 id");
        assert!(ids.contains("a1"));
        assert!(!ids.contains("ghost"));
    }

    // ── 延迟启动注册（pre_record_delay 窗口可取消）──

    #[test]
    fn pending_start_register_cancel_flow() {
        let mut state = AppState::new();
        assert!(!state.is_pending_start("a"), "初始无待启动");
        let token = CancellationToken::new();
        assert!(
            state.register_pending_start("a", token.clone()),
            "首次注册应成功"
        );
        assert!(state.is_pending_start("a"));
        // 重复注册（检测循环重入）→ 拒绝，不覆盖已有令牌
        let token2 = CancellationToken::new();
        assert!(!state.register_pending_start("a", token2.clone()));
        assert!(!token2.is_cancelled(), "被拒的注册不得持有令牌");
        // 取消：移除注册并触发令牌（lib.rs 的 select! 据此放弃启动）
        assert!(state.cancel_pending_start("a"), "应找到待取消的启动");
        assert!(!state.is_pending_start("a"));
        assert!(token.is_cancelled(), "取消令牌必须被触发");
        // 再次取消/移除：无待启动 → false，不误报
        assert!(!state.cancel_pending_start("a"));
        assert!(!state.remove_pending_start("a"));
    }

    #[test]
    fn pending_start_remove_without_cancel_keeps_token_alive() {
        let mut state = AppState::new();
        let token = CancellationToken::new();
        state.register_pending_start("a", token.clone());
        // 延迟正常结束：仅移除注册，不触发取消（录制照常启动）
        assert!(state.remove_pending_start("a"));
        assert!(!state.is_pending_start("a"));
        assert!(!token.is_cancelled(), "正常结束路径不得取消令牌");
    }

    #[test]
    fn pending_starts_are_per_anchor_and_do_not_count_as_tasks() {
        let mut state = AppState::new();
        state.register_pending_start("a", CancellationToken::new());
        state.register_pending_start("b", CancellationToken::new());
        // 互不影响：取消 a 不影响 b
        assert!(state.cancel_pending_start("a"));
        assert!(state.is_pending_start("b"));
        // 未进 tasks 表：并发上限 / 已录制判断不受 pending 影响
        assert_eq!(state.active_count(), 0);
        assert!(!state.is_recording("a"));
    }

    #[test]
    fn record_history_caps_at_limit() {
        let mut state = AppState::new();
        for i in 0..(RECORDING_HISTORY_LIMIT + 10) {
            state.record_history(summary(&format!("id{}", i), i as u64));
        }
        assert_eq!(state.history.len(), RECORDING_HISTORY_LIMIT);
        // 最新在前，最旧的被丢弃
        assert_eq!(state.history[0].anchor_id, "id59");
        assert_eq!(
            state.history[RECORDING_HISTORY_LIMIT - 1].anchor_id,
            "id10"
        );
    }

    // ── S2b：崩溃熔断状态（AppState 集成）──

    #[test]
    fn crash_backoff_blocks_after_three_and_resets_on_success() {
        let mut state = AppState::new();
        assert!(!state.is_crash_blocked("a"));
        assert_eq!(state.crash_count("a"), 0);
        // 1、2 次崩溃：未熔断
        state.record_crash("a");
        state.record_crash("a");
        assert!(!state.is_crash_blocked("a"));
        assert_eq!(state.crash_count("a"), 2);
        // 第 3 次 → 熔断（60s）
        state.record_crash("a");
        assert!(state.is_crash_blocked("a"), "连续 3 次崩溃必须熔断");
        // 不同主播互不影响
        assert!(!state.is_crash_blocked("b"));
        // 恢复：正常结束清零
        state.reset_crash("a");
        assert!(!state.is_crash_blocked("a"));
        assert_eq!(state.crash_count("a"), 0);
    }

    #[test]
    fn disk_notify_throttle_allows_once_per_cooldown() {
        let mut state = AppState::new();
        // 首次放行并标记
        assert!(state.disk_notify_allowed());
        // 冷却期内拒绝（即使再次调用）
        assert!(!state.disk_notify_allowed());
        assert!(!state.disk_notify_allowed());
    }

    // ── R3/L10：按主播清理运行时状态（主播删除路径）──

    #[test]
    fn prune_anchor_removes_per_anchor_state_only() {
        let mut state = AppState::new();
        // 主播 a：崩溃熔断（连续 3 次 → 熔断中）+ 延迟启动注册
        state.record_crash("a");
        state.record_crash("a");
        state.record_crash("a");
        assert!(state.is_crash_blocked("a"));
        let token = CancellationToken::new();
        state.register_pending_start("a", token.clone());
        // 主播 b：同样有状态，但不受 a 的清理影响
        state.record_crash("b");

        assert!(state.prune_anchor("a"), "应清理到 a 的状态条目");
        assert_eq!(state.crash_count("a"), 0, "熔断条目应被移除");
        assert!(!state.is_pending_start("a"), "延迟启动注册应被移除");
        assert!(token.is_cancelled(), "清理时应取消延迟启动令牌（防已删主播启动录制）");
        // b 不受影响
        assert_eq!(state.crash_count("b"), 1);
        // 幂等：无 a 的状态条目时返回 false，不误报
        assert!(!state.prune_anchor("a"));
    }

    #[test]
    fn prune_anchor_without_state_is_noop() {
        let mut state = AppState::new();
        assert!(!state.prune_anchor("ghost"), "无条目时不应误报已清理");
        // 清理后状态查询全部回到默认
        assert_eq!(state.crash_count("ghost"), 0);
        assert!(!state.is_crash_blocked("ghost"));
    }
}
