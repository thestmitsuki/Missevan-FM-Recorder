# 06 · views/debug —— 调试页

> 文件：`src/views/debug/DebugView.vue` + `sections/*`（10 个分节 + SectionCard + shared.ts）

## 1. 职责

开发/排障面板（规格：调试页面仅用于开发版本或用户主动开启的调试模式）——默认关闭，入口在「设置 → 关于」；路由守卫仅在 `debugStore.enabled` 时放行 `/debug`。

## 2. 分节清单

| 分节 | 文件 | 数据源 |
| --- | --- | --- |
| 概览 Overview | `OverviewSection.vue` | `get_debug_info`（环境 / 工具状态 / 版本 / 统计汇总） |
| 检测循环 Detector | `DetectorSection.vue` | `get_detector_stats`（运行中 / 上次检测 / total/success/failed/unknown） |
| 录制引擎 Recorder | `RecorderSection.vue` | `get_recorder_state`（活跃任务 ActiveRecording[] + 历史 RecordingSummary[]） |
| 文件缓存 FileCache | `FileCacheSection.vue` | `get_file_cache_state`（文件数 / 扫描日志 ScanLogEntry[] ≤20） |
| 实时日志 Logs | `LogsSection.vue` | `get_logs`（LogEntry[]，级别/来源/文本过滤 + 分页）+ `debug:log` 实时推送（节流 100/s） |
| 网络请求 Network | `NetworkSection.vue` | `get_network_logs`（NetworkLog[]，过滤 + 分页） |
| Mock 模拟 | `MockSection.vue` | `get_mock_state` + `mock:status_changed`（条目编辑：room_id/name/is_live/stream_url/local_file） |
| 通知中心 Notifications | `NotificationsSection.vue` | 前端 notificationStore（或后端 get_notifications） |
| 性能 Performance | `PerformanceSection.vue` | 占位（轻量采样，当前为空实现） |
| 诊断导出 | Overview 内 | `export_diagnostic_report`（脱敏报告导出） |

## 3. shared.ts / SectionCard.vue

- `shared.ts`：分节共用的格式化/刷新组合（轮询开关、加载态、错误态）。
- `SectionCard.vue`：统一卡片外壳（标题 + 刷新按钮 + 内容插槽）。

## 4. 跨模块依赖

| 依赖 | 用途 |
| --- | --- |
| `services/api` | debug 命令组 |
| `services/events` | debug:log / mock:status_changed |
| `stores/debugStore` | 开关（路由守卫） |
| `types/debug.ts` | 全部 DTO（注释标明后端来源文件） |

## 5. 已知陷阱

- **调试页是只读观测 + Mock 编辑**：不要在这里加「改配置」入口（配置走设置页）；导出诊断报告已脱敏（Cookie/代理密码 → `***`），可安全外发。
- `PerformanceSection` 是空实现占位：不要误以为有性能数据。
- Mock 面板的 `stream_url` 空串 = 故意无效地址（测试 FFmpeg 失败路径）；`mock://` 占位流；`local_file` 可用真实本地音视频——这些语义与后端 MockStore 注释一致。
- 日志级别过滤在前端做（数据全量下发，`get_logs` 支持后端分页过滤——大日志量时用后端分页参数，别拉全量）。
- 实时日志节流（100/s）是后端丢事件式节流：极端高并发下前端看到抽样日志属预期。
