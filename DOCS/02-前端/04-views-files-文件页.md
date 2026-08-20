# 04 · views/files —— 文件页

> 文件：`src/views/files/FilesView.vue`

## 1. 职责

录制文件管理：文件夹树/表格双视图、搜索与筛选、内置播放器（含分段组连播）、重命名/删除（录制中保护）。

## 2. 布局

- 左侧：文件夹树（按主播 → 日期分组：今天/昨天/本周/本月/YYYY-MM）；
- 右侧：文件表格（@tanstack/vue-table）+ 底部播放条（playerStore 消费）。

## 3. 数据与筛选（fileStore）

| 筛选 | 说明 |
| --- | --- |
| 搜索 | 文件名/主播名子串（`searchQuery`） |
| 类型 | `typeFilter`：all / m4a / mp3（按扩展名小写） |
| 日期区间 | `dateRange`：start/end（YYYY-MM-DD，`input type=date` 对齐） |

过滤为前端纯函数（`filteredFiles` / `groupedByDate`），大列表用 `lib/virtualList.ts` 虚拟滚动（固定行高 + overscan）。

## 4. 文件操作

| 操作 | 后端命令 | 保护 |
| --- | --- | --- |
| 播放 | `playRecordingFile` → `convertFileSrc` asset URL | 见 playerStore（单例 audio） |
| 重命名 | `renameRecordingFile(old, new)` | 活跃文件（is_active）拒绝 |
| 删除 | `deleteRecordingFile(path)` | 活跃文件拒绝 + 后端防路径逃逸 |

- 分段组：同主播同场次的 `_001/_002...` 文件自动折叠为组，整组连播（`playFiles(组内文件)` 队列）。
- 刷新：手动按钮 + `recording_files_changed` 事件自动刷新（`fileStore.startListener`）。

## 5. 播放条（playerStore）

- 底部常驻（页面内）：封面/文件名、播放/暂停、进度条（seek）、音量、队列指示（isQueuePlay）、停止。
- 跨页面续播：audio 挂在 body，切到其他页不中断；切回文件页 UI 从 store 恢复（syncState）。

## 6. 跨模块依赖

| 依赖 | 用途 |
| --- | --- |
| `stores/fileStore` | 数据 + 筛选 |
| `stores/playerStore` | 播放 |
| `services/api` | get/refresh/rename/delete/play |
| `services/events` | recording_files_changed |
| `lib/virtualList` | 大列表虚拟滚动 |
| `types/file` | `systemTimeToDate`（SystemTime JSON → Date） |
| `components/ui/*` | table / dialog / input / slider / button / badge 等 |

## 7. 已知陷阱

- **`created_at` 是 serde 默认序列化的 SystemTime JSON**（`{secs_since_epoch, nanos_since_epoch}`），不是 ISO 字符串——必须经 `systemTimeToDate` 转换（types/file.ts 有明确注释）。
- 活跃文件（`is_active`）禁删/禁改由后端标记 + 前端按钮禁用双层保证；绕过前端直接调命令也会被后端拒绝。
- 日期分组使用**本地时区**（浏览器时区）；后端 mtime 按文件系统时区——跨时区用户（如挂代理/换区）分组边界可能有 1 天偏差（已知边界，非 bug）。
- 虚拟列表要求**固定行高**：改行高样式时同步 `virtualList.ts` 的常量/计算，否则滚动错位。
- 大目录扫描是后端异步任务（refresh 命令），页面显示 loading 而非阻塞；扫描日志（≤20 条）在调试页 FileCache 模块可见。
