# 03 · views/live —— 直播页

> 文件：`src/views/live/{LiveView,AnchorCard,AnchorSettingsSheet,AddAnchorDialog}.vue`

## 1. 职责

直播监控主页面：主播卡片/列表双视图、开播/录制状态展示、主播增删改、标签筛选、录制控制入口。

## 2. LiveView.vue —— 页面容器

- **布局**：左侧 `NavRail`（56px 竖排操作栏）+ 可选筛选面板 + 右侧内容区。
- **双视图**：卡片（grid 自适应，最小列宽 300px）/ 列表（行式）；`viewMode` 存 anchorStore（localStorage）。
- **四态**：加载（Skeleton 网格）/ 空（EmptyState）/ 错误（重试按钮）/ 无结果（筛选后）。
- **状态更新策略**：首次全量 `fetchAnchors()`，之后**只依赖 `recording_status_changed` 事件**增量更新单条（统一走 `events.ts → anchorStore.updateStatusFromEvent`，页面内不直接 listen）。
- **删除**：菜单 → `ConfirmDialog` → `removeAnchor`。
- **键盘导航**：卡片/列表项 Tab 聚焦，Enter 打开设置，Delete 触发删除确认。
- **回顶按钮**：内容区滚动监听（替代旧 anchorsCount 近似判断）。
- 操作入口：新增（AddAnchorDialog）、编辑（AnchorSettingsSheet）、停止录制（菜单）、手动刷新、筛选面板开关。

## 3. AnchorCard.vue —— 主播卡片/列表项

- 展示：头像（avatar_url，失败占位）、主播名、房间号（可隐藏）、标签（固定 5 类，可多选筛选）、直播状态徽标（StatusBadge）、录制状态徽标、开播/录制时长（liveSinceOf / recordingSinceOf）。
- 操作：设置（编辑）、停止录制（录制中显示）、删除（确认框）。
- 卡片显示项由 appearanceStore 控制（cardShowAvatar / cardShowTags / cardShowRoomId / cardShowStatusIcon）。
- 状态展示细节：`NotEffectiveBadge`（enable_check=false 的「未启用检测」提示）、录制中进度展示。

## 4. AnchorSettingsSheet.vue —— 主播编辑抽屉

- 字段：名称 / 直播间 URL（前端 `extractRoomId` 校验，错误即时提示）/ 标签（固定 5 标签多选）/ 代理（可选）/ Cookie（可选，长度校验与后端 COOKIE_MAX_LEN 一致）/ 启用检测开关。
- 保存 → `updateAnchor`（后端更新配置 + 状态刷新）；URL 或 room_id 变化时后端联动刷新。
- 主播简介区：`getAnchorProfile` 拉取（名称/头像/简介）。

## 5. AddAnchorDialog.vue —— 新增主播对话框

- 字段：URL（必填，格式校验）/ 名称（可选，留空由后端/API 补齐）/ 标签 / 代理 / Cookie / 启用检测（默认开）。
- 提交 → `addAnchor`（后端 room_id 去重校验，重复时前端提示）。

## 6. 筛选与标签

- `anchorStore.filters`：searchQuery（名称/房间号子串）、tagFilter（多选，固定 5 标签子集）、recordFilter（all/recording/not-recording）、liveFilter（all/live/not-live）。
- 标签常量：`lib/anchorTags.ts`（`ANCHOR_TAGS` i18n 键 ↔ `ANCHOR_TAG_VALUES` 中文落盘值，按下标一一对应）。

## 7. 跨模块依赖

| 依赖 | 用途 |
| --- | --- |
| `stores/anchorStore` | 数据 + 状态 + 筛选 |
| `services/api` | 主播 CRUD / 状态 / 资料 |
| `services/events` | recording_status_changed 增量更新 |
| `components/common/*` | ConfirmDialog / StatusBadge / EmptyState / NavRail / NotEffectiveBadge |
| `components/ui/*` | button / dialog / sheet / dropdown / input / skeleton / switch 等 |
| `lib/anchorTags` | 标签常量 |
| `locales` | 全部文案 |

## 8. 已知陷阱

- 状态更新走事件：刚打开页面前后端已停止推送的「静默期」状态变更不会回放——首次 fetch 是权威快照，事件只做增量。
- `merge_live_state` 在后端完成：前端不要自己写 `isLive = apiLive || recording`（会造成与后端展示不一致的回闪）。
- 删除主播是**破坏性操作**：后端会先停录制再删（remove_anchor），前端确认框文案要提示「将同时停止其录制」。
- 标签筛选与主播 tags 匹配用**中文规范值**（`ANCHOR_TAG_VALUES`），不是 i18n 键——切语言后筛选不失效。
