# 01 · API 命令层（src-tauri/src/api/）

## 1. 职责

Tauri 命令层是**前后端唯一同步入口**：每个命令 = 一个 `#[tauri::command]` 异步函数。命令壳保持「薄」——参数校验 → 调用 domain/infrastructure → 包装 `AppError` 返回。业务规则一律下沉 domain。

## 2. 模块与命令清单（54 个）

### anchor_cmds.rs（8）

| 命令 | 说明 | 关键行为 |
| --- | --- | --- |
| `get_anchors` | 全部主播 | 从 ConfigManager 读 anchors + 合并 live_cache 直播状态 + 头像 |
| `add_anchor` | 新增主播 | 校验 URL 格式/room_id 去重/cookie 长度上限（COOKIE_MAX_LEN）；头像拉取计划（负缓存 O2） |
| `remove_anchor` | 删除主播 | 若录制中先停止；清理头像缓存；live_cache 移除 |
| `refresh_anchor` | 手动检测单个主播 | 调用 `check_live` → 更新配置/状态 |
| `update_anchor` | 编辑主播 | 保存 tags/url/proxy/cookie 等；URL 或 room_id 变化时刷新 |
| `get_anchor_profile` | 主播公开资料 | `MissevanClient::get_anchor_profile`（简介/头像） |
| `get_recording_status` | 全量录制+直播状态 | `build_statuses`（API 直播 ∥ 录制中归并） |
| `stop_anchors_recording` | 批量停止 | 遍历停止 + 汇总结果 |

### config_cmds.rs（8）

| 命令 | 说明 |
| --- | --- |
| `get_config` | 读全局配置（路径不存在时返回默认值） |
| `save_config` | 校验 + 原子写盘 + 变更联动（output_dir→重建缓存、autostart→系统自启、log_level→热更新） |
| `export_config` | 导出 TOML（含 anchors，敏感字段已混淆） |
| `import_config` | 导入（replace/merge 模式，返回 ImportSummary） |
| `reset_config` | 恢复默认 |
| `set_autostart` | 开机自启开关 |
| `set_shortcut` | 全局快捷键（预留） |
| `run_cleanup_now` | 立即清理（`run_cleanup`，保留录制中文件） |

> 另含内部辅助 `allow_output_dir`（**非命令**）：FS scope 放行输出目录，save_config 与 lib.rs setup 共用（Task 20）。

### file_cmds.rs（6）

| 命令 | 说明 |
| --- | --- |
| `get_recording_files` | 文件表 + 文件夹树 + 活跃标记（锁序：先 active_paths 再锁 cache） |
| `refresh_recording_files` | 重新扫描输出目录（fs_walk 安全遍历）并 emit `recording_files_changed` |
| `rename_recording_file` | 重命名（活跃文件拒绝；扩展名保留；防逃逸） |
| `delete_recording_file` | 删除（活跃文件拒绝；`ensure_within_output_dir` 防路径逃逸） |
| `play_recording_file` | 返回播放 URL（asset 协议） |
| `pick_output_dir` | 系统目录选择器（设置页/向导「选择输出目录」）→ `Option<String>` |

### recording_cmds.rs（2）

| 命令 | 说明 |
| --- | --- |
| `start_recording` | **占位禁用**：始终返回错误。真实入口在 detector loop → engine。历史空任务占位曾造成并发上限误占用（L5 审查跟进），禁止恢复旧行为 |
| `stop_recording` | 停止指定主播：取消 pending_starts（延迟中）→ cancel tasks 令牌 → 等 JoinHandle ≤5s → 引擎进程表兜底终止 |

### debug_cmds.rs（14）

`run_health_check` / `get_debug_info` / `get_logs` / `clear_logs` / `get_network_logs` / `clear_network_logs` / `get_detector_stats` / **`trigger_detection_now`**（立即触发一轮检测，调试用）/ **`reset_detector_stats`**（统计清零）/ `get_recorder_state` / `get_file_cache_state` / **`clear_file_cache`**（清空缓存并重扫）/ `get_mock_state` / `export_diagnostic_report`（脱敏导出诊断报告）。

### mock_cmds.rs（8）

`set_mock_live_data`（单条写入 + emit `mock:status_changed`）/ **`set_mock_mode`**（开关 mock 模式）/ **`list_mock_anchors`** / `add_mock_anchor` / **`update_mock_anchor`** / `remove_mock_anchor` / `set_all_mock_live` / `reset_mock`。

### update_cmds.rs（3）

`check_update`（GitHub Releases API，404/网络失败返回 Err 不崩溃）/ `get_app_info` / `open_browser`（仅 http/https，防注入）。

### wizard_cmds.rs（4）

`download_ffmpeg`（Windows 流式下载 gyan.dev zip → `{exe_dir}/ffmpeg/`，emit `download:progress`，解压后触发 FfmpegCheck；**不写配置**——路径由前端暂存，完成时随 save 落盘）/ **`run_wizard_health_check`**（向导环境检查）/ `exit_app` / `finish_wizard`（关向导窗 → 显示主窗 → 刷新缓存 → 唤醒检测）。

### fs_utils.rs（1）

`open_output_dir`（opener `open_path`，目录不存在先创建）。

## 3. 注册与权限（新增命令三步）

1. **注册**：`lib.rs::run` 的 `invoke_handler(tauri::generate_handler![...])` 追加全路径引用。注意命令须 `pub(crate)`（宏导入可见性，E0255/E0603 坑，见 wizard_cmds.rs 头部注释）。
2. **权限**：`src-tauri/permissions/<组>.toml` 定义 `allow-<snake 命令名>` 权限（`commands = { allow = ["<命令名>"], deny = [] }`）。
3. **授权**：`src-tauri/capabilities/default.json`（main 窗）或 `wizard.json`（向导窗）加入 `allow-xxx`。向导窗**刻意不授权**文件/调试/主播管理类命令（权限最小化）。

## 4. 通用模式

- **State 注入**：`State<'_, Arc<X>>` 或 `State<'_, RecorderState>`；`app: tauri::AppHandle` 用于取窗口/退出。
- **事件推送**：`window.emit(...)` / `app.emit(...)`（见 `04-附录/01-命令与事件清单.md`）。
- **错误**：一律 `Result<_, AppError>`；`AppError::system / config / network / recording / internal` 等构造器。
- **大函数避免**：命令壳内尽量只做编排；复杂归并（如 `build_statuses`）也在本层（anchor_cmds），但纯算法已抽成模块级函数并单测。

## 5. 已知陷阱

- `start_recording` 是「注册保持 + 禁用语义」，前端不该调用；后续若实现「手动指定流录制」，必须走 `engine::start_ffmpeg_recording` 同款防线（双录/并发/模板），不得恢复空任务占位。
- `stop_recording` 的取消路径有三段（pending_starts → tasks → processes），改引擎时保持三段兜底完整。
- 头像拉取带失败负缓存（`avatar_negative_cache` + `in_negative_cooldown`），避免坏 URL 反复请求；改动时注意冷却语义。
- `file_cmds` 的锁序（先 active_paths 再 cache）是死锁修复（与 refresh 侧反转），不得调换。
