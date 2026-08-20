# 05 · views/settings —— 设置页

> 文件：`src/views/settings/{SettingsView,AboutDialog,validation,useNumberField}.vue|ts` + `sections/*`

## 1. 职责

全部应用设置：常规 / 录制 / 文件 / 网络 / 通知 / 外观 / 高级 / 快捷键 8 个分类分节表单 + 关于对话框。

## 2. SettingsView.vue —— 容器

- 左侧分类导航（`sections/index.ts` 注册表驱动：id / title(i18n) / icon / component / fields[]）。
- 右侧当前分类表单：本地编辑 → 统一「保存」→ `configStore.saveConfig()`（后端校验 + 原子写盘）。
- 字段级校验前置：`validation.ts`（前端即时错误提示，与后端规则对齐）。
- 「恢复默认」：`resetConfig` 后端重置 + 前端刷新表单。

## 3. sections —— 8 个分节

| 分节 | 文件 | 关键字段 |
| --- | --- | --- |
| 常规 General | `GeneralSection.vue` | 语言 / 主题 / 检查更新 / 开机自启（autostart） |
| 录制 Recording | `RecordingSection.vue` | output_dir（选择器）/ record_format / segment_seconds（分钟换算）/ bitrate_kbps / disk_space_limit_gb / filename_template（含预览）/ audio_only / max_concurrent_recordings / pre_record_delay_secs / max_retries / retry_delay_secs / post_record_action + command |
| 文件 File | `FileSection.vue` | retention_days / max_total_gb / 立即清理按钮（run_cleanup_now + CleanupSummary 结果展示） |
| 网络 Network | `NetworkSection.vue` | proxy_type / proxy_url / proxy_port / proxy_username / proxy_password（脱敏显示） |
| 通知 Notification | `NotificationSection.vue` | notifications_enabled / notify_system / notify_sound / 7 个事件勾选 |
| 外观 Appearance | `AppearanceSection.vue` | accent 色板 / 字号 / 密度 / 卡片显示项（appearanceStore） |
| 高级 Advanced | `AdvancedSection.vue` | log_level（热更新）/ 调试模式开关（debugStore）/ 导出 / 导入配置 / 检查更新 / 打开输出目录 |
| 快捷键 Shortcut | `ShortcutSection.vue` | 全局快捷键（set_shortcut 预留） |

`types.ts`（sections/types.ts）定义分节配置类型；`index.ts` 注册表供容器渲染。

## 4. validation.ts / useNumberField.ts

- `validation.ts`：字段规则（record_format 枚举、output_dir 非空、数值上下限、URL 格式、cookie 长度等），返回错误 Map；与后端 ConfigManager 校验规则对齐（前端即时反馈，后端权威）。
- `useNumberField.ts`：受控数字输入组合式函数（字符串→数字解析、范围钳制、错误态）。

## 5. AboutDialog.vue

- 应用信息（get_app_info：版本 / 构建日期 / OS / Rust / Tauri）、检查更新（check_update → 显示最新版本 + 下载链接，`openBrowser` 打开）、开源许可、诊断导出入口（export_diagnostic_report）、调试模式开关（debugStore）。

## 6. 跨模块依赖

| 依赖 | 用途 |
| --- | --- |
| `stores/configStore` | 配置读写 |
| `stores/appearanceStore` / `themeStore` / `debugStore` | 外观/调试偏好 |
| `services/api` | 全部设置命令 |
| `lib/anchorTags` | 标签（主播设置处复用） |
| `views/settings/sections/index.ts` | 分节注册表 |

## 7. 已知陷阱

- **改设置项 = 四处同步**：后端 `model.rs` 字段 + 默认值、`types/config.ts` + `DEFAULT_CONFIG`、设置分节表单、i18n 文案（zh-CN/en）。遗漏会导致「保存后丢失」或类型错误。
- `filename_template` 改动立即影响**后续录制**（已录制文件不动）；表单提供预览（渲染示例），与后端 `template.rs` 同规则。
- 代理密码输入框用脱敏回显（后端返回密文 → 前端只显示占位/是否已设置），保存空值 = 清除代理认证（与后端语义对齐）。
- 「立即清理」是同步执行的删除操作：大目录会卡 UI 片刻（后端无进度事件）；删除前有确认框（CleanupSummary 展示结果）。
- 导入配置是**高风险操作**（replace 模式会覆盖本地）：对话框需明确提示 replace/merge 差异与 ImportSummary 结果。
- 快捷键分节为预留实现（set_shortcut 后端命令存在但未接真实快捷键绑定）——文档标注，避免误以为已可用。
