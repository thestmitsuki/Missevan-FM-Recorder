# 10 · infrastructure/notification —— 通知域

> 文件：`src-tauri/src/infrastructure/notification/{dispatcher,types,buffer,windows_toast}.rs`

## 1. 职责

统一通知分发：任意业务点调用 `notify(code, level, title, message, ...)`，按**设置过滤矩阵**决定是否 → 系统 toast / 提示音 / 前端推送（`app:notification`）/ 本地环形缓冲。

## 2. 类型（types.rs）

```rust
pub enum NotificationLevel { Info, Warning, Error, Critical }

pub struct Notification {
    id: String,                 // uuid
    code: String,               // 事件码（REC_START / DISK_LOW / ...）
    level: NotificationLevel,
    title: String, message: String,
    suggestion: Option<String>, // 修复建议（前端「通知中心」展示）
    source: String,             // 来源模块（.with_source()）
    timestamp: DateTime<Local>,
    actionable: bool,           // 是否可操作（.with_actionable()）
}
```

## 3. dispatcher.rs —— NotificationDispatcher

### NotifySettings（与 GlobalConfig 通知字段同步，Task 18）

```rust
pub struct NotifySettings {
    enabled: bool,        // notifications_enabled 总开关
    system: bool,         // notify_system
    sound: bool,          // notify_sound
    recording_start / recording_end / recording_error / live_start / live_end / disk_warning / update: bool,
}
```

### 分发流程

```
notify(code, level, title, message, source?, actionable?)
 ├─ from_settings(&config) → 过滤矩阵
 ├─ should_send_system / should_send_sound / should_send_frontend
 ├─ 系统 toast：Windows 走 windows_toast（AUMID 注册）；其他平台 notify-rust 兜底
 ├─ 提示音：系统默认提示音（notify_sound 开启时）
 ├─ 前端推送：emit "app:notification"（前端通知中心）
 └─ 本地环形缓冲（RingBuffer 500 条，get_notifications 供前端/调试页）
```

**事件码 → 通知类型映射**（过滤矩阵依据）：如 `REC_START→recording_start`、`REC_API_FAILED→recording_error`、`DISK_LOW→disk_warning`、`UPDATE_AVAILABLE→update`、`LIVE_START/END→live_start/live_end`。

## 4. windows_toast.rs —— Windows 原生 toast

- **背景**（源码级证据注释）：tauri-plugin-notification 2.3.3 的 Windows 实现在开发模式下**不设置** `System.AppUserModel.ID`，notify-rust 4.18.0 会回退到 `Toast::POWERSHELL_APP_ID`——结果通知**冒充 PowerShell** 弹窗（用户看到「Windows PowerShell 正在尝试发送通知」）。
- **修复**（组 C/3）：以应用身份注册 AUMID（`com.missevan-recorder.app`）发送 toast；非 Windows 分支保持 notify-rust 默认行为。
- 该文件为 `#[cfg(windows)]` 条件编译。

## 5. buffer.rs —— RingBuffer

固定容量环形缓冲（`VecDeque`），`push` 超限丢最旧，`all()` 最新在前，`filter_by_level` 过滤。容量 500（与 `RING_BUFFER_CAPACITY` 一致）。

## 6. 跨模块依赖

| 消费方 | 触发场景 |
| --- | --- |
| `recorder/engine.rs` / `monitor.rs` | REC_START / REC_END / REC_ERROR / REC_CRASH / REC_API_FAILED / REC_RETRY |
| `detector/loop.rs` | LIVE_START / LIVE_END / DISK_LOW / API 连续失败 |
| `api/config_cmds.rs` | CF_SAVE_FAILED 等 |
| `api/update_cmds.rs` | UPDATE_AVAILABLE |
| `infrastructure/checker` | 健康检查结果通知（向导） |
| `api/mock_cmds.rs` | mock 模式提示 |

## 7. 测试

- RingBuffer 容量/丢旧/过滤；
- 过滤矩阵：总开关关闭全禁；事件勾选映射（如 `REC_API_FAILED` 走 recording_error 开关）；system/sound 独立开关；
- `from_settings` 映射与 GlobalConfig 字段对齐。

## 8. 已知陷阱

- **新增事件码** = 4 处同步：`dispatcher.rs` 的类型映射（should_send_*）、`NotifySettings` 字段（如需用户可配）、`types.rs` 事件码常量、前端 `locales/*` 通知文案 + 通知中心展示。
- toast 是**异步平台调用**，不保证送达时序；测试环境（headless）toast 不可用属预期。
- `windows_toast.rs` 的 AUMID 必须与 `tauri.conf.json` 的 `identifier` 一致（`com.missevan-recorder.app`），改 identifier 需同步。
- DISK 类通知有独立节流（`DiskNotifyThrottle`，在 recorder/disk.rs）——`notify` 本身不过滤重复，节流在调用方（防刷屏）。
