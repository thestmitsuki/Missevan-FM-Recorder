# 14 · infrastructure/tray —— 系统托盘域

> 文件：`src-tauri/src/infrastructure/tray/mod.rs`

## 1. 职责

系统托盘图标与动态右键菜单：显示主窗、实时录制计数、最近录制文件直达、优雅退出。规格 §1.1「系统托盘与后台运行」、设计文档 §11.5（Task 17）。

## 2. 菜单结构

```
[图标] Missevan Recorder
 ├─ 显示主窗口
 ├─ 录制中：N            （N>0 可点击 → 显示主窗 + emit "tray:open_live_page" → 前端跳转直播页）
 ├─ 最近录制 ▶           （最多 5 条，最新在前；点击 → 资源管理器选中该文件）
 └─ 退出应用              （优雅退出：保存配置 → 停检测 → cancel 全部任务 → 等 ≤5s → app.exit(0)）
```

## 3. 动态更新机制

- 后台轮询任务每 **2s** 读 `AppState.active_count()` 与 `AppState.history`，与上次菜单数据比对（`TrayMenuData` 相等性），**有变化才 `set_menu` 重建**；
- 选轮询而非监听 `recording_status_changed` / `recording_files_changed`：这两个事件目前只 `window.emit`（无 AppHandle 级广播），轮询直接从共享状态取值——无事件接线改动、无竞态、2s 间隔开销可忽略。

## 4. 可测性设计

托盘本体是平台 API（需 AppHandle / 事件循环），**不可单测**；可测部分已抽出纯函数：

| 函数 | 说明 |
| --- | --- |
| `recent_files_from_history(history, max) -> Vec<RecentFile>` | 历史 → 最近文件（去重/截断/路径规范化） |
| `truncated_label(name, max_chars)` | 文件名截断（省略号） |
| `should_hide_to_tray(close_behavior, is_exiting, tray_available)` | 关闭行为决策（tray/exit） |
| 菜单数据比较 | 无变化不重建（单测覆盖相等/录制数变化/最近文件变化） |

## 5. 平台差异

- **Linux：不实例化托盘**（移植决策 #2）。`try_state::<TrayManager>()` 恒为 None → 关闭即退出。`tray` 模块在 Linux 仍编译（#12 兼容），前端 `isLinuxPlatform()` 禁用托盘相关 UI 并提示。
- Windows / macOS：正常创建；关闭窗口时按 `close_behavior` 决定隐藏到托盘或退出。

## 6. 跨模块依赖

| 依赖 | 用途 |
| --- | --- |
| `infrastructure/state/app_state` | active_count / history |
| `tauri` TrayIcon API | 托盘图标 / 菜单 |
| 前端事件 | `tray:open_live_page`（services/events.ts 监听） |
| `lib.rs` | 创建 TrayManager / close 决策 / do_shutdown 接线 |

## 7. 测试

- `recent_files_from_history`（顺序/上限/路径清洗）；
- `truncated_label`（长名截断、短名原样）；
- `should_hide_to_tray`（close_behavior 矩阵 + 退出中强制不隐藏）；
- 菜单数据相等性（无变化不重建）。

## 8. 已知陷阱

- 轮询间隔（2s）与菜单重建是**按需**的（数据变化才重建），不要改成每次都重建（会闪烁/浪费）。
- 「退出应用」必须走 `do_shutdown` 同款流程（保存配置 + 停循环 + cancel 任务 + 等 JoinHandle），否则录制进程残留（B2 退出兜底在 lib.rs 的 do_shutdown，另有 try_state 强制终止）。
- Linux 关闭即退出是**有意行为**（无托盘），不要在前端绕过（除非未来实现 Linux 托盘）。
- 菜单图标/文案改动需同步 `locales/*`（托盘文案当前为中文硬编码或 i18n 键，改动时确认）。
