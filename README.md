# Missevan FM Recorder

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

猫耳 FM（Missevan，https://fm.missevan.com/）主播直播流的桌面录制工具。自动检测开播、音频录制、文件管理一站式完成，支持无人值守后台运行。

> ⚠️ **免责声明**：本软件仅供个人学习与研究使用。请遵守猫耳 FM 平台服务条款及适用法律法规；录制内容的版权归原权利人所有。开发者不承担因使用本工具产生的任何责任。

## 功能特性

- 🎙️ **直播监控** — 自动检测关注主播的开播状态，检测间隔与随机抖动可配置（默认 120s + 0-60s 抖动，降低平台风控风险）
- 📹 **自动录制** — 检测到开播自动启动 FFmpeg 录制（仅音频轨），支持 M4A / MP3 格式、分段录制、比特率选择（最高 320k）
- 📁 **文件管理** — 录制文件按日期分组、分段组折叠、搜索/筛选、内置播放器连续播放、重命名/删除
- 🏷️ **主播标签** — 固定 5 类标签（音乐/唱歌/日常/ASMR/杂谈）筛选与归类
- 🔔 **系统通知** — Windows 原生 toast 通知（应用身份，非 PowerShell）+ 系统默认提示音，事件类型可配置
- 🖥️ **系统托盘** — 最小化到托盘后台运行，托盘菜单显示录制状态与最近文件
- 🌐 **中英双语** — 完整 i18n，主题亮/暗/跟随系统，强调色可调
- 🧪 **调试面板** — 实时日志、网络请求记录、检测/录制引擎状态、Mock 模拟直播环境（默认关闭，可在设置-关于开启）

## 截图

<!-- 在此添加应用截图（建议 2-3 张：直播页 / 文件页 / 设置页） -->

## 系统要求

- Windows 10 / 11（需要 WebView2 Runtime，Windows 11 自带）
- [FFmpeg](https://ffmpeg.org/) — 首次启动的引导向导可自动下载安装便携版；或自行放入程序目录 `ffmpeg/` 文件夹

## 安装

1. 前往 [Releases](https://github.com/thestmitsuki/Missevan-FM-Recorder/releases) 下载最新安装包（NSIS 安装程序）
2. 运行安装程序，按向导完成首次配置（输出目录、格式、环境检查）
3. 在直播页点击 **+** 添加主播，粘贴直播间 URL（如 `https://fm.missevan.com/live/100000001`）

## 使用说明

### 获取直播间 URL

- 猫耳 FM 手机 App 或网页端进入直播间，复制地址栏链接，格式为 `https://fm.missevan.com/live/数字`
- 在应用内添加主播时粘贴该 URL，会自动提取房间号并获取主播名称与头像

### 常见问题（FAQ）

- **为什么检测不到开播？** 检查主播是否已开启"自动检测"开关；检测间隔默认为 120 秒，开播后最多等待一个周期
- **录制没有声音/文件为空？** 确认 FFmpeg 已安装（设置-高级或引导向导环境检查）；个别直播间可能需要 Cookie（在主播设置中填写）
- **下载 FFmpeg 失败？** 引导向导提供手动下载链接，或从 https://ffmpeg.org 手动获取并放入 `ffmpeg/` 目录
- **提示音没有？** 检查系统"专注助手/免打扰"设置；通知声音跟随系统默认提示音
- **误报"直播中"但无录制？** 状态为双重验证（API + 录制状态）结果，API 抖动时短暂偏差属正常，下一轮检测自动校正

### 已知限制

- 设置页标注「重启生效」/「暂未生效」的字段（文件名模板、录制后动作、代理、日志级别等）为界面预留，尚未接入运行时逻辑
- 全局快捷键为占位展示，暂未绑定实际按键
- 性能监控模块为实验性占位
- 弹幕录制不在计划内（仅音频录制）

## 开发

### 技术栈

- 前端：Vue 3 · Vite · TypeScript · Pinia · vue-i18n · Tailwind CSS v4 · shadcn-vue（本地组件）
- 后端：Rust · Tauri 2 · tokio · reqwest

### 构建

```bash
# 前端
npm ci
npm run build        # vue-tsc + vite build

# 后端（Windows）
cd src-tauri
cargo check
cargo test
cd ..
npm run tauri dev    # 开发运行
npm run tauri build  # 打包安装程序
```

### 目录结构

```
src/              前端（views/stores/services/components/ui）
src-tauri/src/    后端（api 命令 / domain 领域 / infrastructure 基础设施）
docs/             设计文档与实施计划
```

## 报告问题

遇到问题请通过 [GitHub Issues](https://github.com/thestmitsuki/Missevan-FM-Recorder/issues) 反馈，请附上：

- 应用版本（设置-关于）
- 操作系统与架构
- 复现步骤
- 应用日志（`%APPDATA%\missevan-recorder\logs\`）

## 许可

[MIT](LICENSE) © thestmitsuki

内置 FFmpeg 二进制遵循其自身的 LGPL/GPL 许可。
