# 07 · views/wizard —— 设置向导页

> 文件：`src/views/wizard/{WizardView,stagedToConfig}.vue|ts` + `steps/*`（4 步）

## 1. 职责

首启 4 步引导（规格「引导菜单」第 1-4 节）：欢迎 → 基本设置 → 环境检查 → 完成。**仅渲染于独立 wizard 窗口**（App.vue 按窗口 label 分流，无 AppLayout）。

## 2. 步骤流程

| 步骤 | 文件 | 内容 |
| --- | --- | --- |
| 1 欢迎 | `WelcomeStep.vue` | 品牌介绍 / 免责声明 / 开始按钮 |
| 2 基本设置 | `BasicSettingsStep.vue` | 语言 / 输出目录（选择器）/ 录制格式 / 分段 / 磁盘阈值 / 自启 / 托盘最小化 / 主题——**暂存 wizardStore.staged（内存，不写盘）** |
| 3 环境检查 | `EnvCheckStep.vue` | `run_wizard_health_check`（FFmpeg / ffprobe / 磁盘 / 写入权限）；Windows 可「下载 FFmpeg」（`download_ffmpeg` 流式下载 + `download:progress` 进度条）；Linux 提示系统包安装 |
| 4 完成 | `CompleteStep.vue` | 汇总 → 「完成」→ `finish_wizard` |

## 3. 完成流程（finish_wizard 后端命令）

1. 关闭向导窗（destroy 语义）；
2. 显示/聚焦主窗口；
3. 刷新文件缓存（初始扫描输出目录）；
4. 唤醒检测循环（`detection_wake.notify_one()` 立即检测）。

配置在完成时统一落盘：`stagedToConfig.ts` 把 `WizardStaged` 转 `GlobalConfig` 补丁 → `save_config`（FFmpeg 路径作为 `ffmpeg_path` 随配置保存——下载后**不单独写配置**，避免半成品状态）。

## 4. stagedToConfig.ts

```ts
export function stagedToConfigPatch(staged: WizardStaged): Partial<GlobalConfig>
// output_dir / record_format / segment_seconds / disk_space_limit_gb /
// autostart / close_behavior / ffmpeg_path / ffprobe_path(如适用) ...
```

- 与 `configStore.saveConfig` 共用同一命令；前端语言（`locale`）也在此写入 localStorage。
- 校验与设置页 `validation.ts` 复用（口径一致）。

## 5. 窗口生命周期细节

- 后端 setup：首启 → 显示 wizard + 隐藏 main；非首启 → `wizard.destroy()`。
- 向导窗 `onCloseRequested` 会 `prevent_default()`（用户手动关窗 = 停留在向导态，防止半配置退出）——后端因此用 `destroy()` 而非 `close()`（不触发事件）。
- 用户中途关闭向导（如点系统关闭按钮）：后端 `exit_app` 命令提供「退出应用」；关闭窗口行为与主窗不同（向导无托盘隐藏语义）。

## 6. 跨模块依赖

| 依赖 | 用途 |
| --- | --- |
| `stores/wizardStore` | 暂存配置 |
| `stores/configStore` / `themeStore` | 配置 / 主题 |
| `services/api` | run_wizard_health_check / download_ffmpeg / finish_wizard / exit_app / save_config |
| `services/events` | download:progress |
| `services/window` | isWizardWindow |
| `locales` | 全流程文案 |

## 7. 已知陷阱

- **向导配置是「暂存-提交」模式**：中途退出（非完成）不落盘任何配置——这是有意设计（防半配置）；用户再次启动仍会进向导。
- `finish_wizard` 依赖主窗口 label `main`：改窗口 label 需同步（tauri.conf.json + lib.rs + 前端 isWizardWindow 判断）。
- 向导窗口**只有向导权限**（capabilities/wizard.json）：向导内不能调用文件/调试/主播管理命令——新增向导功能时注意权限边界。
- 下载 FFmpeg 是 Windows-only（Linux 返回错误提示系统包安装）：跨平台代码用 `cfg(windows)` 条件编译，前端 `isWindowsPlatform()` 控制按钮显隐。
- 完成标记 localStorage 逻辑已移除（只写不读的历史遗留，M7 审查跟进）——**不要重新引入**，首启判定只依赖配置文件存在性。
