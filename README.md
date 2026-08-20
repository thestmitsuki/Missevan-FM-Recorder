# Missevan FM Recorder

**猫耳 FM（Missevan FM）主播直播音频流的自动化采集与归档系统**

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://github.com/thestmitsuki/Missevan-FM-Recorder/blob/main/LICENSE)
[![Version: 0.2.0](https://img.shields.io/badge/version-0.2.0-blue.svg)](https://github.com/thestmitsuki/Missevan-FM-Recorder/releases)
[![CI](https://img.shields.io/github/actions/workflow/status/thestmitsuki/Missevan-FM-Recorder/ci.yml?branch=main&label=CI)](https://github.com/thestmitsuki/Missevan-FM-Recorder/actions)
[![Platform: Windows 10/11](https://img.shields.io/badge/platform-Windows%2010%2F11-lightgrey.svg)](https://github.com/thestmitsuki/Missevan-FM-Recorder)

**中文** | [English](README_EN.md)

> ⚠️ **免责声明**：本软件仅供个人学习与研究使用。使用者应自行遵守猫耳 FM 平台服务条款及适用法律法规；录制内容的著作权归原权利人所有。开发者不对因使用本工具产生的任何后果承担责任。

## 目录

- [1. 项目概述](#1-项目概述)
- [2. 运作原理](#2-运作原理)
- [3. 功能规格](#3-功能规格)
- [4. 系统要求](#4-系统要求)
- [5. 安装与部署](#5-安装与部署)
- [6. 使用指南](#6-使用指南)
- [7. 工程与开发](#7-工程与开发)
- [8. 贡献](#8-贡献)
- [9. 许可证](#9-许可证)

## 1. 项目概述

Missevan FM Recorder 是一款面向 Windows 10 / 11 的桌面应用程序（基于 Tauri 2 构建；后端 Rust，前端 Vue 3），用于对猫耳 FM 平台主播直播间的**音频流**进行自动化采集、归档与管理。

系统的设计目标是在无人值守条件下持续运行：自动监测关注主播的开播状态，触发音频录制，并对录制产物实施结构化组织与检索。全程仅采集音频轨，不涉及画面与弹幕内容。

**技术栈**：

| 层 | 技术 |
| --- | --- |
| 桌面框架 | Tauri 2（双窗口：主窗口 + 设置向导） |
| 后端 | Rust（2021 edition）· tokio 异步运行时 |
| 前端 | Vue 3.5 · TypeScript（strict）· Vite 8 · Pinia 4 · Tailwind CSS 4 |
| 国际化 | vue-i18n 10（简体中文 / English） |
| 录制引擎 | FFmpeg / ffprobe（子进程调用） |

## 2. 运作原理

系统以「**轮询检测 → 录制执行 → 文件归档 → 事件通知**」为主链路，端到端流程如下：

```
周期性轮询开播状态 → 开播判定 → 磁盘预检 → 获取流地址 → 延迟窗口复检
→ 启动 FFmpeg 子进程 → 分段写入音频 → 进程监控（周期 5 s）
→ 结束清理 → 文件缓存刷新 → 事件通知
```

### 2.1 开播检测

- 检测循环按可配置周期轮询（默认 `check_interval_secs = 120`），每轮附加 0–60 秒随机抖动，以降低对平台接口的周期性请求特征。
- 开播判定采用**双重验证**：将平台 API 返回状态与录制侧状态归并（`merge_live_state`）得出最终直播状态，避免单一数据源误判。
- 请求错误按类别处置：服务端错误（5XX / HTTP 429）触发重试与指数退避，429 附加冷却；网络错误触发重试；格式错误与未知错误不重试。

### 2.2 录制执行

- 开播确认后依次执行：磁盘空间预检（阈值 `disk_space_limit_gb`）→ 获取流地址 → 进入可取消的延迟窗口（`pre_record_delay_secs`）→ 复检仍为直播且未在录制后启动 FFmpeg 子进程。
- FFmpeg 仅采集音频轨（`-vn`），支持 M4A / MP3 封装、分段录制（`-f segment`）与码率选择（64 / 128 / 192 / 256 / 320 kbps，默认 128）。
- 并发防护：按主播去重、活跃任务数受并发上限约束、进程级单实例文件锁，三重机制防止重复录制。
- 子进程异常退出时按指数退避策略自动重试（崩溃熔断），并保留 `.part` 残留标记供排查。

### 2.3 文件管理与归档

- 输出目录由文件缓存服务扫描维护；前端按日期分组展示（今天 / 昨天 / 本周 / 本月 / 年月），分段文件自动折叠为组并支持整组连续播放。
- 支持搜索、日期筛选、重命名与删除；**录制中的文件受保护**，禁止改名与删除。
- 录制结束自动清理：按保留天数（`retention_days`）或存储总量上限（`max_total_gb`）删除过期文件。

### 2.4 通知与常驻运行

- 通知调度器按过滤矩阵（总开关 + 7 类事件勾选）分发：Windows 原生 toast（以应用身份注册 AUMID）、系统提示音、前端通知中心（环形缓冲 500 条）。
- 系统托盘常驻，菜单动态显示录制状态与最近录制（至多 5 条）；关闭窗口默认隐藏至托盘，退出走统一关闭流程。
- 首次启动的设置向导执行环境检查（FFmpeg / ffprobe 候选、磁盘空间、写入权限），并可在 Windows 下自动下载便携版 FFmpeg。
- 更新检查通过 GitHub Releases API 解析 `v{version}` 标签，与当前版本比对。

## 3. 功能规格

| 功能域 | 规格 |
| --- | --- |
| 直播监控 | 周期可配置（默认 120 s + 0–60 s 抖动）；双重验证开播判定；429 指数退避 |
| 自动录制 | 仅音频轨；M4A / MP3；码率 64–320 kbps（默认 128）；分段录制；并发上限可配置 |
| 文件管理 | 日期分组；分段折叠；连续播放；搜索 / 筛选 / 重命名 / 删除；录制中文件保护；自动清理 |
| 主播标签 | 固定 5 类：音乐 / 唱歌 / 日常 / ASMR / 杂谈 |
| 通知 | Windows 原生 toast + 提示音；事件类型与提示音可配置 |
| 后台运行 | 系统托盘；开机自启（可选）；单实例运行 |
| 双语界面 | 简体中文 / English；亮色 / 暗色 / 跟随系统 |
| 调试面板 | 实时日志 / 网络记录 / 引擎状态 / Mock 模拟环境（默认关闭）；脱敏诊断报告导出 |
| 数据安全 | 配置原子写入 + 自动备份（保留 5 份）；导出配置敏感字段混淆（enc:v1:） |

## 4. 系统要求

| 项目 | 要求 |
| --- | --- |
| 操作系统 | Windows 10 / 11（Linux 可编译；官方安装包仅提供 Windows NSIS） |
| 运行时 | WebView2 Runtime（Windows 11 内置，Windows 10 需安装） |
| 录制引擎 | FFmpeg / ffprobe（首启向导自动获取，或置于程序目录 `ffmpeg/`） |

## 5. 安装与部署

1. 从 [Releases](https://github.com/thestmitsuki/Missevan-FM-Recorder/releases) 下载最新安装包。
2. 运行安装程序；首次启动进入设置向导，按步骤完成输出目录、录制格式等基础配置。
3. 向导完成环境检查与 FFmpeg 就绪性校验后进入主界面，检测循环随即启动。

## 6. 使用指南

### 6.1 添加主播

在「直播」页面点击 **＋**，输入直播间链接（格式：`https://fm.missevan.com/live/{数字房间号}`）。系统自动解析房间号、主播名称与头像。

> 个别直播间需登录态方可录制，可在主播设置中为该主播独立配置 Cookie。

### 6.2 配置项

| 配置域 | 入口 | 主要参数 |
| --- | --- | --- |
| 录制 | 设置 → 录制 | 输出目录、封装格式、码率、分段策略、并发上限、自动清理 |
| 通知 | 设置 → 通知 | 事件类型、提示音 |
| 常规 | 设置 → 常规 | 语言、主题、开机自启 |
| 诊断 | 设置 → 关于 | 调试面板、诊断报告导出 |

### 6.3 数据位置

- 录制文件：输出目录，按 **主播名 / 日期** 组织（默认文件名模板：`{主播名}/{房间号}/{日期}/{时间}.{扩展名}`）。
- 配置与日志：`%APPDATA%\missevan-recorder\`（日志位于 `logs/` 子目录）。

## 7. 工程与开发

> 架构细节、命令与事件清单、测试矩阵详见 [`DOCS/`](DOCS/README.md) 文档集。

**代码组织**（后端三层，依赖单向）：

| 层 | 职责 |
| --- | --- |
| `api` | Tauri 命令薄壳：参数校验 → 调用领域层 → 错误包装（9 模块 54 命令） |
| `domain` | 业务规则（配置 / 检测 / 录制 / 文件服务 / 平台客户端 / 工具），纯 Rust 可单测 |
| `infrastructure` | 平台适配（状态 / 日志 / 通知 / 托盘 / 健康检查 / 单实例锁） |

**构建与验证**：

```bash
npm ci                     # 安装依赖
npm run build              # 前端类型检查（vue-tsc）+ 构建
npm run tauri dev          # 开发模式（热更新）
npm run tauri build        # 打包安装程序
cd src-tauri && cargo test # 后端单元测试
```

CI（`.github/workflows/ci.yml`）在每次 push / PR 执行：前端 `vue-tsc` 类型检查 + `vite build`；后端 `cargo check` + `cargo test`。

## 8. 贡献

欢迎提交 Issue 与 Pull Request。Bug 报告请使用 [Bug 报告模板](.github/ISSUE_TEMPLATE/bug_report.yml)，附应用版本、操作系统、复现步骤与日志（`%APPDATA%\missevan-recorder\logs\`）。

## 9. 许可证

[MIT](LICENSE) © thestmitsuki

内置 FFmpeg 二进制遵循其自身的 LGPL/GPL 许可。
