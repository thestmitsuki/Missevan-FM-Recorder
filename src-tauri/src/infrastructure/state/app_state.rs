use crate::infrastructure::state::mock_store::MockStore;

use serde::Serialize;
use std::collections::{HashMap, VecDeque};
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
    /// 每主播录制序号（filename_template `{index}` 数据源；单调递增不回收）
    pub recording_seq: HashMap<String, u32>,
    /// pre_record_delay 等待中的自动录制启动（anchor_id → 取消令牌）。
    /// 延迟窗口内任务尚未注册进 `tasks`，stop_recording / remove_anchor /
    /// 「关闭检测」必须先查此集合才能取消"即将启动"的录制（否则延迟结束后
    /// 仍会启动，只能等 monitor 兜底 ≤10s 才停——既有修复注释明确要杜绝的场景）。
    pub pending_starts: HashMap<String, CancellationToken>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            global_cancel: CancellationToken::new(),
            history: VecDeque::new(),
            recording_seq: HashMap::new(),
            pending_starts: HashMap::new(),
        }
    }

    pub fn is_recording(&self, anchor_id: &str) -> bool {
        self.tasks.contains_key(anchor_id)
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

    /// 分配并记录主播的下一个录制序号（1 起，单调递增；`{index}` 模板变量用）
    pub fn next_recording_seq(&mut self, anchor_id: &str) -> u32 {
        let next = self.recording_seq.get(anchor_id).copied().unwrap_or(0) + 1;
        self.recording_seq.insert(anchor_id.to_string(), next);
        next
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

    #[test]
    fn recording_seq_increments_per_anchor() {
        let mut state = AppState::new();
        assert_eq!(state.next_recording_seq("a"), 1);
        assert_eq!(state.next_recording_seq("a"), 2);
        assert_eq!(state.next_recording_seq("a"), 3);
        // 不同主播独立计数
        assert_eq!(state.next_recording_seq("b"), 1);
        assert_eq!(state.recording_seq.get("a"), Some(&3));
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
}
