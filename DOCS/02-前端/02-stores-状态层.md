# 02 · stores —— 状态层（Pinia）

> 文件：`src/stores/*.ts`（10 个 store，setup 风格）

## 1. 总览

| Store | 职责 | 持久化 | 事件订阅 |
| --- | --- | --- | --- |
| `anchorStore` | 主播列表 + 状态 Map + 筛选 + 视图模式 | localStorage（view_mode/filters） | `recording_status_changed` |
| `configStore` | 全局配置读写（DEFAULT_CONFIG 与后端对齐） | 后端 config.toml | — |
| `fileStore` | 录制文件 + 筛选 + 操作 | 无（后端 file_cache） | `recording_files_changed` |
| `notificationStore` | 通知中心（≤50 条） | 无 | `app:notification` |
| `playerStore` | 全局音频播放器（单例 audio） | 无（音量可选） | — |
| `themeStore` | 主题模式 light/dark/system | localStorage `theme_mode` | — |
| `appearanceStore` | 强调色/字号/密度/卡片显示项 | localStorage `appearance`(JSON) | — |
| `debugStore` | 调试模式开关 | localStorage `debug_enabled` | — |
| `wizardStore` | 向导暂存配置（内存） | 无（完成时落盘） | — |
| `mockStore` | Mock 数据编辑（后端 MockStore 镜像） | 无 | `mock:status_changed` |

## 2. anchorStore（直播页核心）

- 状态：`anchors` / `statusMap`（anchor_id → {is_live, is_recording, ...}）/ `filters`（searchQuery / tagFilter[] / recordFilter / liveFilter）/ `viewMode`（card/list）。
- 关键逻辑：
  - `fetchAnchors()`：首次全量拉取（get_anchors + get_recording_status 归并状态）；
  - `updateStatusFromEvent(update)`：**增量单条更新**（事件驱动，页面不重复 listen）；
  - 派生状态：`filteredAnchors`（搜索/标签/录制/直播四维过滤）、`liveCount` / `recordingCount`、`isAnchorRecording(id)`；
  - `liveSinceOf` / `recordingSinceOf`：基于时间戳的状态时长（直播页「开播于」「录制于」展示）；
  - `clearFilters()` 重置筛选。
- 持久化：`live_view_mode` / `live_filters`（localStorage JSON，与 `LiveFilters` 结构一致）。

## 3. configStore

- 状态：`config: GlobalConfig | null` / `loading` / `dirty` / `hasOutputDir` / `segmentMinutes`（秒→分钟换算）。
- 动作：`fetchConfig()`（get_config，后端无配置时返回默认）/ `saveConfig()`（save_config + dirty 标记）/ `updateConfig(patch)`（本地合并）/ `pickOutputDir()`（dialog 选目录）/ `resetConfig()`。
- `DEFAULT_CONFIG`：与后端 `GlobalConfig::default()` 逐字对齐（Task 3 核对），含遗留字段 `anchor_ids: []`（前端不读写）。

## 4. fileStore（文件页核心）

- 状态：`files` / `folders`（文件夹树）/ `searchQuery` / `typeFilter`（all/m4a/mp3）/ `dateRange`（YYYY-MM-DD 起止）/ `loading`。
- 派生：`filteredFiles`（扩展名 + 日期区间 + 搜索）、`groupedByDate`（今天/昨天/本周/本月/年月——`dateGroupOf` 纯函数）。
- 动作：`fetchFiles()` / `refreshFiles()` / `renameFile(old, new)` / `deleteFile(path)` / `getPlayUrl(file)`（convertFileSrc）；`startListener()` 订阅 `recording_files_changed`（自动刷新）。
- 辅助：`extOf`（小写扩展名）、`dateIsoOf`（SystemTime → YYYY-MM-DD，本地时区）。

## 5. playerStore（全局播放器）

- **单例 audio**：首次播放时创建挂 `document.body`，页面切换不销毁（跨页面续播）；
- 状态：`playing` / `progress` / `duration` / `volume` / `queue` / `currentFile` / `isQueuePlay`；
- 动作：`playFiles(files)`（队列顺序播放，ended 自动下一个）/ `togglePlay` / `seek` / `setVolume` / `stopPlayback` / `syncState`；
- **错误语义**：从不绑定空 src（无 src 的 Audio 不触发 error）——修复「切到文件页误报音频加载失败」根因；`@error` 仅在真实错误（文件缺失/被移动/asset scope 拦截）时提示。

## 6. themeStore / appearanceStore（UI 偏好）

- themeStore：`mode`（light/dark/system）+ `resolvedTheme`；`matchMedia` 监听系统主题；应用 `document.documentElement.classList`（dark 类）。
- appearanceStore：`prefs`（accent 强调色 / fontSize / density / cardShowAvatar / cardShowTags / cardShowRoomId / cardShowStatusIcon）；`applyPrefs` 改写 shadcn.css 的 `--primary/--ring` 等 CSS 变量（inline style 覆盖，亮暗均生效）+ `html[data-density]`；localStorage JSON 持久化。

## 7. debugStore / wizardStore / mockStore

- debugStore：`enabled`（`debug_enabled` localStorage，默认关）→ 路由守卫放行 `/debug`。
- wizardStore：`staged`（WizardStaged：language/outputDir/recordFormat/segmentSeconds/diskThresholdGb/autostart/trayMinimize/theme/ffmpegPath/ffprobePath）——向导第二步暂存内存，完成时 `stagedToConfig.ts` 转配置补丁随 save 落盘（FFmpeg 路径不单独写配置）。
- mockStore：`data`（后端 get_mock_state 镜像）/ `enabled`；编辑动作调后端命令，`mock:status_changed` 事件同步开关与计数。

## 8. 已知陷阱

- **事件订阅成对**：`startListening`/`stopListening` 需在组件 onMounted/onUnmounted 或应用生命周期成对调用，防止重复监听（handler Set 是幂等的，但 listen 本身会叠加）。
- `DEFAULT_CONFIG` 与后端默认值漂移 = 前端兜底值与真实配置不一致（改后端默认值时必改前端）。
- `anchorStore.statusMap` 由事件增量更新：初始 fetch 后任何缺失主播状态默认「离线/未录制」；后端归并语义（api_live ∥ recording）在前端不做二次归并（见架构总览 §6.2）。
- 播放 URL 用 asset 协议（`convertFileSrc`）：文件被移动/删除后旧 URL 失效是预期行为（playerStore @error 提示）。
- 外观偏好影响全局 CSS 变量：新增「可调项」需在 appearanceStore 的类型 + applyPrefs + settings 外观分节三处同步。
