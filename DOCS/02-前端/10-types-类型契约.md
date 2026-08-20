# 10 · types —— 类型契约层

> 文件：`src/types/*.ts`（index.ts 聚合导出）

## 1. 职责与纪律

前端与后端 DTO 的**逐字段对齐契约**：每个文件的头部注释标明对应后端来源文件（如 `types/debug.ts` 列出 debug_cmds.rs / logging/buffer.rs / state/app_state.rs 等）。改动后端序列化结构时，本层是必改点。

## 2. 文件地图

| 文件 | 内容 | 对应后端 |
| --- | --- | --- |
| `anchor.ts` | `AnchorConfig` / `AnchorProfile` / `RecordingStatus` / `AnchorStatusUpdate` / `MockLiveData` | anchor_cmds / mock_cmds |
| `config.ts` | `GlobalConfig` / `CloseBehavior` / `PostRecordAction` / `ProxyType` / `RecordFormat` / `LogLevel` / `ImportSummary` | config/model.rs |
| `file.ts` | `RecordingFile` / `FileFolder` / `SystemTimeJson` / `RecordingFilesPayload` / `CleanupSummary` + `systemTimeToDate()` | services/file_cache.rs / cleanup.rs |
| `debug.ts` | `ToolStatus` / `DebugInfo` / `LogEntry` / `NetworkLog` / `DetectorStatsSnapshot` / `ActiveRecording` / `RecordingSummary` / `RecorderStateInfo` / `ScanLogEntry` / `FileCacheState` / `MockStatusChanged` | debug_cmds / logging / stats / app_state |
| `health.ts` | `DiagnosticReport` / `DiagnosticFullReport` / `DownloadFfmpegResult` / `DownloadProgress` | checker / wizard_cmds |
| `notification.ts` | `Notification` / `NotificationLevel` | notification/types.rs |
| `theme.ts` | `ThemeMode` | 前端自有（localStorage） |
| `update.ts` | `UpdateInfo` / `AppInfo` | update_cmds |

## 3. 关键细节

- **`SystemTimeJson`**：Rust `SystemTime` 经 serde 默认序列化为 `{ secs_since_epoch, nanos_since_epoch }`（非 ISO 字符串）——必须用 `systemTimeToDate()` 转换（`types/file.ts`）。
- **`GlobalConfig` 逐字对齐**：key 全 snake_case；字符串枚举值与后端 TOML 完全一致（`close_behavior: "tray"|"exit"`、`proxy_type: "none"|"http"|"socks5"`、`log_level` 小写）；含遗留字段 `anchor_ids`（与后端兼容语义一致）。
- **`AnchorStatusUpdate`**：`recording_status_changed` 事件载荷（anchor_id + is_live + is_recording），前端 store 增量更新。
- **`MockStatusChanged`**：`mock:status_changed` 事件载荷（enabled + count）。
- 类型注释即文档：新增字段时在注释中标注「与后端 xx.rs 对齐」。

## 4. 已知陷阱

- **漏改 types = 运行时静默错位**：TS 类型只约束前端，后端结构变化（如字段改名）前端不报编译错（invoke 返回 any 强转），运行时才暴露——**后端改 DTO 必须同步 types/**。
- serde 的 `Option<T>` → `T | null`；`#[serde(default)]` 字段 → 前端按「可能缺失」处理（或 DEFAULT_CONFIG 兜底）。
- 枚举字符串值前后端必须完全一致（大小写/连字符）；Tauri 不做枚举映射。
- `SystemTimeJson` 的纳秒字段在 JS 里是 number：`nanos / 1e6` 转毫秒（精度损失可忽略，用于展示）。
