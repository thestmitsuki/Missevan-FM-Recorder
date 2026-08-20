# 01 · Tauri 配置与权限体系

> 文件：`src-tauri/tauri.conf.json`、`src-tauri/capabilities/*.json`、`src-tauri/permissions/*.toml`

## 1. tauri.conf.json

| 项 | 值 | 说明 |
| --- | --- | --- |
| productName | `missevan-recorder` | 安装/产物名 |
| version | `0.1.0` | 与 Cargo.toml / package.json 一致 |
| identifier | `com.missevan-recorder.app` | 应用 ID；**与 windows_toast 的 AUMID 绑定**，勿随意改 |
| build.frontendDist | `../dist` | 生产前端产物 |
| build.devUrl | `http://localhost:1420` | vite dev server（`npm run dev`） |
| beforeDevCommand | `npm run dev` | 自动拉起 vite |
| beforeBuildCommand | `npm run build` | 构建前先编译前端 |

### 窗口定义（双窗口）

| label | 标题 | 尺寸 | 说明 |
| --- | --- | --- | --- |
| `wizard` | Missevan 录制器 设置向导 | 560×760（min 480×640） | 首启向导，visible=false（setup 决定显示） |
| `main` | 猫耳FM录制器 | 800×620（min 720×500） | 主窗口，visible=false（首启时隐藏） |

其余：bundle 图标（icons/*）、identifier、安全（详见 Tauri 2 默认 CSP——配置中无显式 CSP 时用默认）。

## 2. capabilities（窗口能力）

### default.json（main 窗）

- `core:default` + `core:window:allow-set-theme`（主题切换需要）
- `dialog:default`（目录选择）
- `notification:default`（系统通知）
- **全部业务命令权限**（allow-get-anchors … allow-open-output-dir 等，覆盖 54 个命令）

### wizard.json（向导窗）

- `core:default` + dialog + notification
- **仅向导相关权限**：get_config / run_wizard_health_check / download_ffmpeg / finish_wizard / exit_app / open_output_dir 等（不包含文件/调试/主播管理命令——**权限最小化**）

> 权限模型：窗口 → capability → permission（`allow-<command>`）→ 具体命令。前端 invoke 命令时 Tauri ACL 校验，未授权 → `command not allowed` 错误。

## 3. permissions（8 个业务权限文件）

| 文件 | 覆盖命令组 | 权限项（示例） |
| --- | --- | --- |
| `anchors.toml` | 主播（8） | allow-get-anchors / allow-add-anchor / allow-remove-anchor / allow-refresh-anchor / allow-update-anchor / allow-get-anchor-profile / allow-get-recording-status / allow-stop-anchors-recording |
| `config.toml` | 配置（8） | allow-get-config / allow-save-config / allow-export-config / allow-import-config / allow-reset-config / allow-set-autostart / allow-set-shortcut / allow-run-cleanup-now |
| `file.toml` | 文件（6） | allow-get-recording-files / allow-refresh-recording-files / allow-rename-recording-file / allow-delete-recording-file / allow-play-recording-file / allow-pick-output-dir |
| `recording.toml` | 录制（2） | allow-start-recording / allow-stop-recording |
| `debug.toml` | 调试（14） | allow-run-health-check / allow-get-debug-info / allow-get-logs / allow-clear-logs / allow-get-network-logs / allow-clear-network-logs / allow-get-detector-stats / allow-trigger-detection-now / allow-reset-detector-stats / allow-get-recorder-state / allow-get-file-cache-state / allow-clear-file-cache / allow-get-mock-state / allow-export-diagnostic-report |
| `mock.toml` | Mock（8） | allow-set-mock-live-data / allow-set-mock-mode / allow-list-mock-anchors / allow-add-mock-anchor / allow-update-mock-anchor / allow-remove-mock-anchor / allow-set-all-mock-live / allow-reset-mock |
| `update.toml` | 更新（3） | allow-check-update / allow-get-app-info / allow-open-browser |
| `wizard.toml` | 向导（4） | allow-download-ffmpeg / allow-run-wizard-health-check / allow-exit-app / allow-finish-wizard |
| `fs_utils.toml` | 文件系统（1） | allow-open-output-dir |

权限定义格式：

```toml
[[permission]]
identifier = "allow-stop-recording"
description = "启用 stop_recording 命令"
commands = { allow = ["stop_recording"], deny = [] }
```

## 4. 新增命令的权限接线（四步）

1. `api/<模块>.rs` 写 `#[tauri::command] pub(crate) async fn`；
2. `lib.rs` `generate_handler![...]` 注册；
3. `permissions/<组>.toml` 增加 `allow-<命令>` 权限；
4. `capabilities/default.json`（+ wizard.json 如需要）加入权限项。

遗漏任一步 → 前端 invoke 报 `command <name> not allowed`（Tauri 2 ACL）。

## 5. 已知陷阱

- **identifier 是 AUMID 的一部分**：改 `com.missevan-recorder.app` 需同步 `windows_toast.rs`，否则 toast 失效/冒充 PowerShell（历史坑）。
- 向导窗权限最小化是**有意设计**：向导内不能调用 debug/file 命令——新增向导功能时先在 wizard.json 加权限（并评估风险）。
- 命令名是蛇形（`stop_anchors_recording`）而权限标识是连字符（`allow-stop-anchors-recording`）：命名时注意对应关系。
- Tauri 2 的 ACL 校验发生在 invoke 边界：前端拿不到未授权命令的返回值（安全设计），别试图绕过。
