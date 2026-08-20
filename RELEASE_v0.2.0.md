# Release Notes — v0.2.0

> Missevan FM Recorder（猫耳 FM 录制器）· 2026-08-21
>
> 上一个发布：v0.1.0

## 概述

v0.2.0 是首个功能性里程碑版本：在 v0.1.0 的录制能力之上，重点补全了**稳定性、数据安全与工程化**。本版本包含多轮代码审查（静态暴力审查 / 运行时审查 / 配置审查）的修复成果，并完成设置项实装、依赖安全升级与发布流程打通。

## 新特性

- **设置项全面实装**：代理（含认证）、超时、重试、并发数、录制延迟、录制后动作、日志级别等占位设置已接入真实逻辑（此前仅 UI 占位）
- **首次启动引导完善**：设置向导未完成时关闭会重新打开引导，避免误入主界面（`wizard_completed` 迁移兼容旧配置）
- **录制结束后自动清理**：每日定时清理改为录制结束触发检查，清理更及时（旧定时配置保留兼容）
- **文件名模板增强**：支持模板变量、变量光标位置插入、缺扩展名自动补全、命令变量逐条说明

## 稳定性与安全修复

- **磁盘空间保护（S2）**：录制启动前与运行期定期检查磁盘阈值（`disk_space_limit_gb`，0=不限制）；连续崩溃熔断退避（指数退避至上限），避免主播异常退出时无限重启；磁盘不足通知节流防刷屏
- **安全目录遍历（S1）**：清理与文件缓存统一使用不跟随符号链接/junction 的安全遍历，防止误删输出目录外文件、越权列出文件
- **崩溃熔断（S3）**：录制进程异常退出处理与自动重启策略强化
- **配置安全**：写盘前备份 + 损坏自动恢复 + 敏感字段混淆（代理密码等日志脱敏）
- **代理密码日志脱敏**：URL 内嵌密码在日志/错误信息中统一替换为 `***`
- **路径穿越防护**：锚点 ID 校验拒绝 `../`、`..\`、绝对路径与非 ASCII 字符
- **托盘/窗口修复**：托盘双重注册消除、子进程隐藏控制台窗口、幽灵图标提示

## 工程化

- **发布流程**：GitHub Actions 构建 Release（Windows NSIS/便携版、Linux deb/AppImage/Arch 包），tag 推送自动触发
- **CI**：前端 type-check + build、后端 `cargo check` + `cargo test`（Windows + Linux）
- **文档**：README 全面重写并新增英文版（README_EN.md）；新增 INSTALL.md 安装指南与 DOCS/ 架构文档
- **依赖安全**：postcss 8.5.19 → 8.5.26（Dependabot 自动升级）

## 变更文件概览

- 后端 Rust：~25 个模块（api/domain/infrastructure），新增 `disk.rs`、`fs_walk.rs`
- 前端 Vue/TS：~90 个文件，新增 `platform.ts`、`debounce.ts`、`virtualList.ts` 等
- 配置：Tauri 权限系统（`src-tauri/permissions/`）、capabilities、图标（含托盘图标）

## 兼容性说明

- 配置文件兼容 v0.1.0：旧配置无需迁移即可升级（新增字段均有默认值）
- 旧版自启动注册表键名 `MissevanRecorder` 保留兼容
- 最低系统要求：Windows 10/11、Linux（见 INSTALL.md）

## 已知问题

- 自动更新检查依赖 GitHub Releases API，网络受限环境会显示「检查失败」但不影响使用

## 下载

安装包由 CI 构建并发布至 GitHub Releases（tag `v0.2.0`）：

- Windows：`missevan-recorder_0.2.0_x64-setup.exe`（NSIS）/ `missevan-recorder_0.2.0_x64_portable.exe`
- Linux：`missevan-recorder_0.2.0_amd64.AppImage` / `missevan-recorder-0.2.0-1-x86_64.pkg.tar.zst`
