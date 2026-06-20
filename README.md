<div align="center">
  <h1>🎙️ 猫耳FM录制器</h1>
  <p><strong>Missevan FM Recorder</strong></p>
  <p>猫耳FM (missevan.com) 主播直播流自动录制工具</p>
  <p>
    <img src="https://img.shields.io/badge/platform-Windows-blue?style=flat-square" alt="Platform">
    <img src="https://img.shields.io/badge/tauri-2.x-purple?style=flat-square" alt="Tauri">
    <img src="https://img.shields.io/badge/vue-3.x-brightgreen?style=flat-square" alt="Vue 3">
  </p>
  <p>
    <i>简体中文</i> · <a href="#english">English</a>
  </p>
</div>

---

## 📖 简介

**猫耳FM录制器** 是一款基于 Tauri 2.x 的桌面应用，用于自动监测和录制猫耳FM（Missevan FM）平台主播的直播流。

核心功能：
- ⏱️ **自动监测** — 定时检测主播直播状态，开播即录
- 🔄 **自动重连** — 录制中断自动重试，确保不丢片段
- 🎚️ **灵活格式** — 支持 M4A / MP3 录制格式
- 📂 **文件管理** — 内置文件浏览器，支持重命名、删除、系统打开
- 🌐 **代理支持** — 每位主播可独立配置代理和 Cookie
- 🖥️ **优雅界面** — Apple 设计语言，毛玻璃质感，明暗主题自适应

---

## 🚀 快速开始

### 系统要求

- **Windows 10 / Windows 11** (64位)
- [FFmpeg](https://ffmpeg.org/download.html)（录制必需）

### 下载

从 [Releases](https://github.com/your-username/missevan-recorder/releases) 页面下载最新版本安装包。

### 首次使用

1. **安装 FFmpeg**（任选一种）：
   - 将 `ffmpeg.exe` 放入程序目录下的 `ffmpeg/` 文件夹
   - 或将 FFmpeg 添加到系统 PATH
   - 或在软件「设置」中手动指定路径
2. **启动程序**，点击「添加主播」卡片
3. 填写 **主播名称** 和 **直播间 URL**（如 `https://fm.missevan.com/live/123456`）
4. 添加成功，卡片自动出现在监控网格中

> 💡 配置代理/Cookie：点击卡片 `···` 菜单 →「直播间设置」

---

## 🛠️ 从源码构建

| 依赖 | 安装方式 |
|------|---------|
| Rust | https://rustup.rs |
| Node.js 18+ | https://nodejs.org |
| FFmpeg | https://ffmpeg.org/download.html |

### 方法一（推荐）：使用 Tauri CLI 一步完成

```bash
# 安装前端依赖
npm install

# 安装 Tauri CLI 并构建（自动编译前端 + 后端）
npm run tauri build
```

### 方法二：分步构建

```bash
# 1. 安装前端依赖
npm install

# 2. 构建前端
npm run build

# 3. 编译 Rust 后端（从 src-tauri/ 目录）
cd src-tauri
cargo build --release
```

> **⚠️ 注意**: 如果使用分步构建，**务必先执行 `npm run build`**。
> 跳过此步骤会导致 `dist/` 目录缺失，编译出的程序没有界面，
> 运行时会报 `localhost 拒绝连接` 的错误。

### 构建产物

`src-tauri/target/release/missevan-recorder.exe`

### 类型检查（可选）

```bash
npm run typecheck
```

---

## ⚙️ 功能详情

<details>
<summary><b>直播监控</b></summary>

- 主播卡片网格展示，实时状态一目了然
- 每 5 秒自动刷新直播状态
- 支持主播头像缓存加载
</details>

<details>
<summary><b>自动录制</b></summary>

- 检测开播后自动启动 FFmpeg 录制
- 录制期间每 10 秒检测直播状态，直播结束自动停止
- 支持断线重连（最多 3 次）
- 实时监测磁盘空间，不足自动停止
</details>

<details>
<summary><b>录制配置</b></summary>

- 输出格式：M4A (AAC) / MP3 (libmp3lame)，码率 320k
- 分段录制：按指定秒数分割文件
- 磁盘空间阈值设置
</details>

<details>
<summary><b>文件管理</b></summary>

- 按主播文件夹分组查看
- 搜索、排序（日期/名称/大小）
- 重命名、删除
- 调用系统默认播放器打开
</details>

---

## 🧩 技术栈

| 层 | 技术 |
|----|------|
| 桌面框架 | Tauri 2.x |
| 前端 | Vue 3 + Pinia + vue-i18n |
| 样式 | CSS 自定义属性 + 毛玻璃设计 |
| 后端 | Rust (tokio, reqwest) |
| 录制 | FFmpeg 子进程 |
| 配置 | TOML 文件 |

---

## 📁 文件结构

```
config/
├── config.toml           # 全局配置
└── anchors/              # 主播独立配置
    └── *.toml

downloads/
└── 猫耳FM直播/
    └── 主播名/           # 按主播自动分类
        └── *.m4a
```

---

## 🤝 贡献

1. Fork 本仓库
2. `git checkout -b feature/amazing-feature`
3. `git commit -m 'feat: add amazing feature'`
4. `git push origin feature/amazing-feature`
5. 创建 Pull Request

---


## ⚠️ 免责声明

- 本工具仅用于个人学习研究，请遵守猫耳FM平台服务条款
- 录制内容请勿用于商业用途或二次分发
- 请尊重主播版权

---

<h2 id="english">English</h2>

## Missevan FM Recorder

A desktop app built with **Tauri 2.x + Vue 3 + Rust** for automatically monitoring and recording live streams from [Missevan FM](https://fm.missevan.com).

### Features

- **Auto Monitoring** — Periodically checks live status, records automatically
- **Auto Reconnect** — Retries up to 3 times on failure
- **Flexible Formats** — M4A (AAC) / MP3 at 320kbps
- **File Manager** — Browse, rename, delete, play with system player
- **Per-broadcaster Config** — Independent proxy, cookie, check interval

### Quick Start

```bash
npm install
npm run tauri build    # one-step build (frontend + backend)
```

> **Note**: If building manually, run `npm run build` BEFORE `cargo build --release`.
> Missing frontend build causes `localhost refused connection` error.

Requires: Rust, Node.js 18+, FFmpeg

---

<p align="center">
  <sub>Made with ❤️ for the Missevan FM community</sub>
</p>
