# Missevan FM Recorder

**猫耳 FM 主播直播流自动录制工具** —— 自动检测开播、无感录制音频、一站式文件管理，支持无人值守后台运行。

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://github.com/thestmitsuki/Missevan-FM-Recorder/blob/main/LICENSE)
[![Version: 0.1.0](https://img.shields.io/badge/version-0.1.0-blue.svg)](https://github.com/thestmitsuki/Missevan-FM-Recorder/releases)
[![CI](https://img.shields.io/github/actions/workflow/status/thestmitsuki/Missevan-FM-Recorder/ci.yml?branch=main&label=CI)](https://github.com/thestmitsuki/Missevan-FM-Recorder/actions)
[![Platform: Windows 10/11](https://img.shields.io/badge/platform-Windows%2010%2F11-lightgrey.svg)](https://github.com/thestmitsuki/Missevan-FM-Recorder)

**中文** | [English](README_EN.md)

> ⚠️ **免责声明**：本软件仅供个人学习与研究使用。请遵守猫耳 FM 平台服务条款及适用法律法规；录制内容的版权归原权利人所有。开发者不承担因使用本工具产生的任何责任。

## 功能特性

- 🎙️ **直播监控** — 轮询关注主播的开播状态，检测间隔与随机抖动可配置（默认 120 秒 + 0–60 秒随机抖动，降低平台风控风险）；开播判定采用双重验证（API 状态 + 录制状态），遭遇 429 限流时自动指数退避冷却
- 📹 **自动录制** — 检测到开播自动拉起 FFmpeg 录制（仅音频轨），支持 M4A / MP3 格式、分段录制、码率选择（64/128/192/256/320 kbps，默认 128）；多主播并发上限可配置
- 📁 **文件管理** — 录制文件按日期分组（今天 / 昨天 / 本周 / 本月 / 年月）展示，分段自动折叠成组；支持搜索、按日期筛选、内置播放器连续播放（分段组整组连播）、重命名与删除（录制中的文件自动保护，禁止改名与删除）
- 🏷️ **主播标签** — 固定 5 类标签（音乐 / 唱歌 / 日常 / ASMR / 杂谈）归类与筛选
- 🔔 **系统通知** — Windows 原生 toast 通知（以应用身份注册 AUMID 发送，非 PowerShell 兜底）+ 系统默认提示音，事件类型与提示音可配置
- 🖥️ **系统托盘** — 最小化到托盘后台运行，托盘菜单实时显示录制状态与最近文件（最多 5 条），点击即可打开
- 🌐 **中英双语** — 完整 i18n（简体中文 / English），主题亮色 / 暗色 / 跟随系统，强调色可调
- 🧪 **调试面板** — 实时日志、网络请求记录、检测与录制引擎状态、Mock 模拟直播环境（默认关闭，可在「设置 → 关于」开启），支持一键导出脱敏诊断报告

**其他亮点**：首次启动引导向导（环境检查：FFmpeg / ffprobe / 磁盘空间 / 写入权限，可自动下载便携版 FFmpeg）· 每主播独立 Cookie（应对个别直播间鉴权）· 自动清理（保留天数 / 总量上限 / 定时）· 自动更新检查 · 开机自启动（可选）· 单实例运行。

## 快速开始

1. **下载** — 前往 [Releases](https://github.com/thestmitsuki/Missevan-FM-Recorder/releases) 下载最新 NSIS 安装包。支持 Windows 10 / 11（需要 WebView2 Runtime，Windows 11 自带）
2. **安装配置** — 运行安装程序，首次启动的引导向导将引导完成输出目录、录制格式等基础设置，并自动检查 / 下载 FFmpeg（也可自行放入程序目录 `ffmpeg/` 文件夹）
3. **添加主播** — 在「直播」页点击 **+**，粘贴直播间 URL（如 `https://fm.missevan.com/live/100000001`），即可开始监控录制

## 使用说明

### 获取直播间 URL

- 猫耳 FM 手机 App 或网页端进入直播间，复制地址栏链接，格式为 `https://fm.missevan.com/live/数字`
- 在应用内添加主播时粘贴该 URL，应用会自动提取房间号并获取主播名称与头像；个别直播间需要登录态时，可在主播设置中填写 Cookie

### 添加主播与标签

- 主播列表支持启用 / 停用「自动检测」，可设置别名与标签
- 标签用于文件页的分类筛选，每名主播可打多个标签

### 设置要点

- **录制**（设置 → 录制）：输出目录、格式（M4A / MP3）、码率、分段时长、文件名模板、并发录制上限
- **通知**（设置 → 通知）：通知开关、事件类型、提示音开关
- **常规**（设置 → 常规）：语言、主题、开机自启动、关闭行为（最小化到托盘 / 退出）
- **调试面板**（设置 → 关于）：开启后主导航出现「调试面板」入口
- **日志**：位于 `%APPDATA%\missevan-recorder\logs\`，按日轮转

## 常见问题（FAQ）

- **为什么检测不到开播？** 检查该主播的「自动检测」开关是否开启；检测间隔默认为 120 秒，开播后最多等待一个检测周期（外加随机抖动）
- **录制没有声音 / 文件为空？** 确认 FFmpeg 已正确安装（设置 → 高级或引导向导环境检查）；个别直播间可能需要 Cookie（在主播设置中填写）
- **下载 FFmpeg 失败？** 引导向导提供手动下载链接；也可以从 [ffmpeg.org](https://ffmpeg.org/download.html) 自行下载，将 `ffmpeg.exe` / `ffprobe.exe` 放入程序目录 `ffmpeg/` 文件夹，或在设置中指定路径
- **提示音没有？** 检查 Windows「专注助手 / 免打扰」设置；通知声音跟随系统默认提示音（可在通知设置中关闭）
- **误报「直播中」但无录制？** 开播判定为双重验证（API + 录制状态）结果，API 抖动时短暂偏差属正常，下一轮检测会自动校正
- **为什么录制文件放在「主播名」文件夹里，文件名也带主播名？** 默认文件名模板为 `{anchor_name}/{date}_{time}_{anchor_name}_{index}.{ext}`——以主播名作为稳定标识（直播间标题会变化，不宜用于文件名），按主播名建目录、文件名含日期时间与录制序号。可在「设置 → 录制」中自定义模板，支持占位符：`{anchor_name}`（主播名）、`{room_id}`（房间号）、`{date}`（日期）、`{time}`（时间）、`{index}`（每主播录制序号）、`{ext}`（格式扩展名）
- **日志在哪里？如何反馈问题？** 日志位于 `%APPDATA%\missevan-recorder\logs\`（按日轮转）；也可在「调试面板」中导出完整诊断报告（配置脱敏），反馈问题时一并附上
- **修改了设置为什么不生效？** 自启动、关闭行为、托盘显隐、日志级别等字段修改后需重启应用生效（设置页有「重启生效」标注）；「自定义 DNS」为界面预留，暂未接入运行时逻辑（标注「暂未生效」）

## 已知限制

- 「自定义 DNS」（设置 → 网络）标注「暂未生效」，为界面预留，尚未接入运行时逻辑
- 全局快捷键为占位展示，尚未接入后端注册，编辑功能已禁用（规划在后续版本接入）
- 调试面板中的性能监控模块为实验性占位
- 弹幕录制不在计划内（本工具仅做音频录制）
- 部分设置（自启动 / 关闭行为 / 托盘显隐 / 日志级别）修改后需重启应用生效

## 开发

### 技术栈

- 前端：Vue 3 · Vite · TypeScript · Pinia · vue-i18n · Tailwind CSS v4 · shadcn-vue（本地组件）
- 后端：Rust · Tauri 2 · tokio · reqwest

### 构建

```bash
# 前端（安装依赖 + 类型检查 + 构建）
npm ci
npx vue-tsc -p tsconfig.app.json --noEmit
npm run build

# 后端（Windows）
cd src-tauri
cargo check
cargo test
cd ..

# 开发运行 / 打包安装程序
npm run tauri dev
npm run tauri build
```

### 目录结构

```
src/              前端（views 页面 / stores 状态 / services API 调用 / components 组件 / locales 语言包）
src-tauri/src/    后端（api Tauri 命令 / domain 领域逻辑 / infrastructure 基础设施）
src-tauri/icons/  应用图标
.github/          CI 工作流（前端类型检查 + 构建、后端 cargo check + test）与 Issue 模板
```

## 贡献指南

欢迎提交 Issue 与 Pull Request：

1. **报告问题** — 请通过 [GitHub Issues](https://github.com/thestmitsuki/Missevan-FM-Recorder/issues)（使用 [Bug 报告模板](https://github.com/thestmitsuki/Missevan-FM-Recorder/issues/new?template=bug_report.yml)）反馈，并附上：应用版本（设置 → 关于）、操作系统与架构、复现步骤、应用日志（`%APPDATA%\missevan-recorder\logs\`）或调试面板导出的诊断报告
2. **提交代码** — Fork 本仓库，在独立分支上完成修改后提交 Pull Request；CI 会自动运行前端类型检查与构建、后端 `cargo check` 与测试，请确保全部通过后再请求合并
3. **代码风格** — 请与现有代码保持一致的风格（前端 TypeScript 严格模式；后端 rustfmt 默认格式，遵循 api / domain / infrastructure 分层）

## 许可证

[MIT](LICENSE) © thestmitsuki

内置 FFmpeg 二进制遵循其自身的 LGPL/GPL 许可。
