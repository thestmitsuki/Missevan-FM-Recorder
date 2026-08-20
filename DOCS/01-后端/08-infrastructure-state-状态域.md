# 08 · infrastructure/state —— 状态域

> 文件：`src-tauri/src/infrastructure/state/{app_state,mock_store}.rs`

## 1. 职责

应用级运行时状态的权威持有者：录制任务表、延迟启动注册、崩溃计数、录制历史、头像缓存、Mock 数据源。

## 2. app_state.rs —— AppState / RecorderState

### 结构

```rust
pub struct AppState {
    pub tasks: HashMap<String, Task>,              // anchor_id -> 活跃录制任务
    pub pending_starts: HashMap<String, CancellationToken>, // 延迟窗口中的启动（可取消）
    pub history: VecDeque<RecordingSummary>,       // 最近 50 条录制结束摘要
    pub crash_counts: HashMap<String, u32>,        // 每主播崩溃计数（CrashBackoff 数据）
    pub avatar_cache: AvatarCache,                 // room_id -> avatar URL
    pub avatar_negative_cache: AvatarNegativeCache,// 失败负缓存 + 冷却
    pub mock_store: MockStore,                     // 内嵌 Mock 数据源
    // + 原子标志（running / shutdown 等）
}

pub type AppStateHandle = Arc<Mutex<AppState>>;

// RecorderState 是 Tauri 托管包装：state + app_handle（供 emit/退出）
pub struct RecorderState {
    pub state: AppStateHandle,
    pub app_handle: OnceLock<AppHandle>,
}
```

### Task（活跃录制任务）

```rust
pub struct Task {
    pub anchor_id: String,
    pub cancel_token: CancellationToken,   // 停止录制 = 发令牌
    pub handle: JoinHandle<()>,            // monitor 任务句柄（等待 ≤5s）
    pub anchor_name: String, pub room_id: String,
    pub output_path: String, pub started_at: Instant, pub pid: Option<u32>,
}
```

### 关键方法

| 方法 | 用途 |
| --- | --- |
| `register_pending_start(anchor_id, token)` | 注册延迟启动（**去重**：已有则 false）；`cancel_pending_start` 供停止 |
| `insert_task / remove_task / get_task / stop_task` | 任务表操作（stop_task = cancel + 等待 JoinHandle） |
| `active_count / active_output_paths / active_recording_ids` | 并发上限 / 活跃文件标记 / 状态查询 |
| `push_history`（上限 50）/ `recent_summaries` | 托盘「最近录制」数据源 |
| `crash_count / record_crash / clear_crash / is_crash_blocked` | 崩溃熔断（数据在 state，算法在 `recorder/disk.rs` CrashBackoff） |
| `avatar_cache` 系列（含 `in_negative_cooldown`） | 头像 O2 优化：失败负缓存 + 冷却期不重试 |
| `prune_anchor` | 删除主播时清理关联状态 |

### 序列化视图（调试页）

`ActiveRecording` / `RecordingSummary` / `RecorderStateInfo` 为调试页「录制引擎」模块的 DTO（`get_recorder_state` 返回）。

## 3. mock_store.rs —— MockStore

```rust
pub struct MockStore {
    entries: Arc<RwLock<HashMap<String, MockLiveData>>>, // room_id -> 模拟条目
    mock_mode: AtomicBool,                               // 模式开关
}
pub struct MockLiveData { room_id, name, is_live, stream_url, local_file }
```

- `is_mock_mode()`：开启后 DetectionLoop **不再发真实请求**（调试/演示）。
- 方法：`set / get / list / remove / clear / reset`；`set_all_live` 批量。
- `mock://` 占位流地址；`stream_url=""` = 故意无效地址（测试 FFmpeg 失败处理）；`local_file` 可选真实音视频供 FFmpeg 录制 mock 流。

## 4. 跨模块依赖

| 依赖 | 用途 |
| --- | --- |
| `domain/recorder/disk` | 复用 `CrashBackoff` / `DiskNotifyThrottle` / `now_ms`（state 只存数据，算法在 domain） |
| 消费者 | 全部 api 命令 / DetectionLoop / engine / monitor / TrayManager |

## 5. 测试

- 任务表增删查、pending_starts 去重与取消；
- 历史上限（50 条滚动）；
- 崩溃计数 / 熔断状态 / prune_anchor 清理（无条目不误报）；
- MockStore 增删改查 / 重置 / 模式开关。

## 6. 已知陷阱

- **AppState 与 FfmpegRecorder 是两个独立「录制真相」**：tasks（应用层任务）vs processes（进程表）。`is_recording` 的权威判定在 **FfmpegRecorder.processes**（双录防御）；tasks 用于取消/等待/展示。改动时注意区分。
- `pending_starts` 的存在意义：延迟窗口内任务尚未进 tasks 表，stop/remove 找不到 → 用 pending 表支持取消（回归修复）。删除该表会让「停止录制」在延迟窗口失效。
- 头像负缓存冷却：失败 URL 在冷却期内不重试（防刷 API）；测试用 `in_negative_cooldown` 纯函数。
- `RecorderState.app_handle` 是 `OnceLock`：`set_app_handle` 只在 setup 期调用一次；命令里在 `get` 之前确保已 set（否则 None 分支要兜底）。
