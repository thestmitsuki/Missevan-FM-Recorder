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
}

impl AppState {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            global_cancel: CancellationToken::new(),
            history: VecDeque::new(),
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
