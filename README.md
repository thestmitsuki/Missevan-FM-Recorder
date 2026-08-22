# 🦐 猫耳FM录制器（Missevan FM Recorder）

猫耳 FM（Missevan，<https://fm.missevan.com/>）主播直播流的桌面自动录制工具：自动检测开播、无人值守后台录制、分段管理与文件管理一站式完成。

**版本** `0.1.0` · **平台** Windows 10/11（首选）· Linux（实验性）· **许可** MIT · **状态** 开发中

> ⚠️ **免责声明**：本软件仅供个人学习与研究使用。请遵守猫耳 FM 平台服务条款及适用法律法规；录制内容的版权归原权利人所有。开发者不承担因使用本工具产生的任何责任。

## 目录

- [功能特性](#功能特性)
- [系统要求](#系统要求)
- [安装与构建](#安装与构建)
- [使用说明](#使用说明)
  - [首次启动向导](#首次启动向导)
  - [添加主播](#添加主播)
  - [直播页](#直播页)
  - [文件页](#文件页)
  - [设置页](#设置页)
  - [系统托盘与关闭行为](#系统托盘与关闭行为)
  - [常见问题](#常见问题)
- [配置说明](#配置说明)
- [日志与诊断](#日志与诊断)
- [目录结构](#目录结构)
- [开发指南](#开发指南)
- [测试](#测试)
- [路线图](#路线图)
- [已知限制](#已知限制)
- [贡献指南](#贡献指南)
- [许可证](#许可证)

## 功能特性

- 🎙️ **直播监控** — 周期检测关注主播开播状态（默认 120s），检测间隔与随机抖动可配置（默认抖动上限 60s，降低风控风险）；失败自动重试（指数退避）、429 限流冷却、检测并发信号量；每个主播可独立开关检测
- 📹 **自动录制** — 检测到开播自动启动 FFmpeg 录制（仅音频轨），支持 M4A / MP3 格式、可选分段录制、比特率可选（64/128/192/256/320 kbps，默认 128）、并发录制上限（默认 3）、录制前延迟窗口可取消
- 🧱 **文件名模板** — 输出路径完全模板化，支持 `{anchor_name}` / `{room_id}` / `{date}` / `{time}` / `{ext}` 变量，路径逐组件消毒防穿越，空模板自动回退默认
- ⚡ **录制后动作** — 录制结束后可配置：无操作 / 打开所在文件夹 / 执行自定义命令（`{file}` / `{output_dir}` / `{anchor_name}` / `{room_id}` 变量，变量值消毒防命令注入）
- 📁 **文件管理** — 录制文件按日期分组、分段组折叠，支持月份折叠（联动同月分段组整体隐藏）、搜索/筛选/排序、内置播放器连续播放、重命名/删除（录制中的文件受保护不可删除）
- 🏷️ **主播标签** — 固定 5 类标签（音乐 / 唱歌 / 日常 / ASMR / 杂谈）筛选与归类
- 🔔 **系统通知** — Windows 原生 WinRT toast（应用身份，非 PowerShell）+ 系统默认提示音；7 类事件、系统通知 / 声音开关均可配置
- 🖥️ **系统托盘** — 最小化到托盘后台运行，菜单显示录制状态与最近 5 条录制文件，支持 `--minimized` 参数静默启动（开机自启场景）
- ⚙️ **配置管理** — 全局 `config.toml` + 主播独立 `anchors/*.toml`；原子写入、保存前自动备份（保留 5 份）、损坏自动恢复；导入/导出（导出自动脱敏）；代理支持 HTTP / SOCKS5 + 认证；Cookie / 代理密码混淆落盘
- 🔄 **开机自启与单实例** — 开机自启（Windows HKCU Run 键 / Linux XDG autostart）；fs2 文件锁单实例（防双开导致重复录制）
- 📊 **日志与诊断** — 控制台 / 文件 / 内存环形缓冲三层输出，按日轮转、保留 7 天、全出口脱敏；一键导出诊断报告（zip）
- 🌐 **中英双语** — 完整 i18n（zh-CN / en）；主题亮 / 暗 / 跟随系统，强调色可调
- 🧪 **调试面板** — 实时日志、网络请求记录、检测 / 录制引擎状态、Mock 模拟直播环境（默认关闭，可在设置-关于开启）
- 🔄 **更新检查** — 启动时 / 手动检查 GitHub Releases 最新版本

## 系统要求

- **Windows 10 / 11**（需要 WebView2 Runtime，Windows 11 自带）
- **[FFmpeg](https://ffmpeg.org/)**（运行时依赖）：首次启动向导可自动下载便携版到程序目录 `ffmpeg/`（仅 Windows，gyan.dev 官方构建，含 ffmpeg.exe / ffprobe.exe）；也可手动放入该目录、在设置中指定 `ffmpeg_path` 或加入 PATH
- **Linux（实验性）**：提供 deb / AppImage 构建目标；运行需 webkit2gtk-4.1 / gtk3 / libsoup3 / librsvg 等系统库；FFmpeg 请用系统包安装（如 Arch Linux：`sudo pacman -S ffmpeg`）；详见 [docs/ARCHLINUX.md](docs/ARCHLINUX.md) 与 [packaging/arch](packaging/arch)
- **macOS**：代码含条件编译分支，但尚无 CI 与构建目标覆盖（见[已知限制](#已知限制)）

## 安装与构建

### 前置依赖

| 依赖 | 说明 |
|---|---|
| Node.js 20+ | 前端构建 |
| Rust 工具链 | rustc ≥ 1.89.0（见 `src-tauri/Cargo.toml` 的 `rust-version`） |
| FFmpeg / ffprobe | 运行时依赖；应用内可检测，引导向导提供 Windows 便携版自动下载 |
| Tauri 2 CLI | 随 `npm ci` 安装（`@tauri-apps/cli`） |

### 从源码运行

```bash
# 安装依赖
npm ci

# 开发模式（Vite 热更新 + 应用窗口）
npm run tauri dev
```

### 构建发布

```bash
# 前端类型检查 + 构建
npm run build

# 后端检查与测试
cd src-tauri
cargo check
cargo test
cd ..

# 打包安装程序（Windows: NSIS；Linux: deb / AppImage）
npm run tauri build
```

也可以直接使用 [Releases](https://github.com/thestmitsuki/Missevan-FM-Recorder/releases) 页面发布的安装包。

## 使用说明

### 首次启动向导

首次启动进入设置向导：

1. **环境检查** — 检测 FFmpeg / ffprobe、磁盘空间、输出目录可写性
2. **FFmpeg 安装**（Windows）— 一键下载便携版并解压到程序目录 `ffmpeg/`（带下载进度）；Linux 提示使用系统包安装
3. **基础配置** — 输出目录、录制格式（m4a / mp3）
4. **进入应用**

向导未完成时，每次启动都会重新进入。

### 添加主播

在直播页点击 **+**，粘贴直播间 URL（手机 App 或网页端进入直播间后复制地址栏链接，格式为 `https://fm.missevan.com/live/数字`）。应用自动提取房间号并获取主播名称与头像；可为主播设置「启用检测」开关、Cookie 与标签。

### 直播页

- 卡片 / 列表视图切换，固定标签筛选
- 展示每主播的开播状态与录制状态（状态为 API + 录制状态双重验证）
- 录制由检测循环自动触发；可手动**停止录制**（含取消录制前延迟窗口中的启动）

### 文件页

- 录制文件按日期分组展示，分段录制自动折叠为分段组；按日期分组头可折叠整个月份（联动同月分段组整体隐藏，折叠状态 localStorage 记忆）
- 搜索 / 筛选 / 排序；内置播放器可连续播放分段组
- 重命名 / 删除 / 打开所在文件夹；录制中的文件拒绝删除

### 设置页

- **常规**：输出目录、格式、分段秒数（0 = 不分割）、比特率、文件名模板
- **录制后动作**：无 / 打开文件夹 / 自定义命令（变量见[配置说明](#配置说明)）
- **网络**：全局代理（none / HTTP / SOCKS5 + 认证）、API 与流超时
- **通知**：总开关 + 7 类事件勾选 + 系统通知 / 提示音
- **文件**：自动清理开关、保留天数、总大小上限
- **高级**：日志级别（即时生效）、检测并发数、FFmpeg / ffprobe 路径、检测随机抖动上限、开机自启
- **关于**：版本信息、检查更新、导出诊断报告、开启调试面板（Mock 模拟等）

### 系统托盘与关闭行为

关闭窗口按「关闭行为」设置处理（默认 `tray`）：

- **驻留托盘**：窗口隐藏，后台继续检测与录制；托盘菜单含「显示主窗口」「录制状态（当前录制数）」「最近录制」（最多 5 条，点击在文件管理器中定位）「退出应用」
- **直接退出**（`exit` 或 Linux）：关闭即退出

「退出应用」执行优雅退出：保存配置 → 停止检测循环 → 等待录制任务结束（≤5s）→ 强制终止剩余 FFmpeg 进程。

Linux 不创建托盘（显式设计决策），关闭即退出。

### 常见问题

- **为什么检测不到开播？** 检查主播是否开启「启用检测」开关；检测间隔默认为 120 秒，开播后最多等待一个周期
- **录制没有声音 / 文件为空？** 确认 FFmpeg 已安装（设置-高级或向导环境检查）；个别直播间可能需要在该主播设置中填写 Cookie
- **下载 FFmpeg 失败？** 向导提供手动下载链接，也可从 <https://ffmpeg.org> 手动获取后放入 `ffmpeg/` 目录或配置 `ffmpeg_path`
- **提示音没有？** 检查系统「专注助手 / 免打扰」设置；通知声音跟随系统默认提示音
- **误报「直播中」但无录制？** 状态为 API + 录制状态双重验证，API 抖动时短暂偏差属正常，下一轮检测自动校正

## 配置说明

### 配置文件位置（TOML）

| 平台 | 全局配置 | 主播配置 |
|---|---|---|
| Windows | `{程序目录}/config/config.toml`（便携式布局，程序目录旁） | `{程序目录}/config/anchors/*.toml` |
| Linux | `~/.config/missevan-recorder/config.toml` | `~/.config/missevan-recorder/anchors/*.toml` |

- 主播配置每主播一个文件，字段含 `id` / `name` / `url` / `room_id` / `proxy` / `cookie` / `enable_check` / `tags`
- 配置写入为「原子写」（唯一临时文件 + 重命名），保存前自动备份并保留最近 5 份；配置损坏时自动回退备份并保留损坏文件留证
- **导入 / 导出**（备份恢复）：导出自动脱敏（密码置空、代理 URL 掩码）；导入前全量校验，失败不落盘

### 录制文件名模板

设置页可编辑，默认 `{anchor_name}/{date}_{time}_{anchor_name}.{ext}`：

| 变量 | 含义 |
|---|---|
| `{anchor_name}` | 主播名（作为路径组件消毒） |
| `{room_id}` | 房间号 |
| `{date}` | 日期 `YYYY-MM-DD` |
| `{time}` | 时间 `HH-MM-SS` |
| `{ext}` | 扩展名（m4a / mp3） |

模板支持 `/` 或 `\` 作为目录分隔符；渲染结果按路径组件逐段消毒（`..`、绝对路径、Windows 非法字符 → `_`），无法逃逸输出目录；模板为空或不含变量时回退默认模板。

### 录制后自定义命令变量

`{file}`（输出文件完整路径）、`{output_dir}`（所在目录）、`{anchor_name}`、`{room_id}`。变量值先消毒、再以双引号包裹后经系统 shell 执行（Windows `cmd /C`，其余平台 `sh -c`），防止命令注入；命令在后台执行，不阻塞录制结束流程。

### 代理

全局代理支持 `none` / `http` / `socks5`（地址 + 端口 + 可选账号密码认证），应用于检测与录制相关的网络请求；代理密码混淆落盘。

## 日志与诊断

- **日志目录**：Windows `%APPDATA%\missevan-recorder\logs\`；Linux `~/.local/share/missevan-recorder/logs/`
- **轮转与保留**：文件命名 `missevan-recorder.log.YYYY-MM-DD`，按日轮转（JSON 格式）；只保留最近 7 天，启动时与运行中每 24h 各清理一次
- **日志级别**：设置-高级 → `log_level`（默认 `info`，保存后即时生效，无需重启；控制台与调试面板即时切换，文件层始终全级别落盘）
- **脱敏**：Cookie / Authorization / Password 一律输出 `***`；FFmpeg stderr 截断 256 字符，防止签名 URL 泄漏
- **诊断报告**：设置-关于 → 导出诊断报告（zip：环境健康检查 + 配置 + 日志 + 网络请求记录 + 统计），自动脱敏，反馈问题时请附上

## 目录结构

```
├── src/                    # 前端（Vue 3 + Vite + TypeScript）
│   ├── views/              # live（直播）/ files（文件）/ settings（设置）/ wizard（向导）/ debug（调试）
│   ├── stores/             # Pinia：anchor / file / config / player / theme / mock ...
│   ├── services/           # Tauri 命令封装与事件订阅
│   ├── components/         # 通用组件（含 shadcn-vue 本地组件）
│   ├── locales/            # i18n（zh-CN / en）
│   └── lib/                # 领域常量与工具（如主播标签定义）
├── src-tauri/              # 后端（Rust + Tauri 2）
│   ├── src/
│   │   ├── api/            # 命令层：anchor / config / file / recording / wizard / debug / update / mock
│   │   ├── domain/         # 领域层：config / detector / recorder / services / spider / tools
│   │   ├── infrastructure/ # 基础设施层：state / tray / logging / notification / checker / crypto / single_instance
│   │   ├── lib.rs          # 应用装配、启动流程、优雅退出
│   │   └── main.rs
│   ├── Cargo.toml
│   └── tauri.conf.json
├── docs/                   # 设计文档、审计报告、Linux 移植说明
├── packaging/arch/         # Arch Linux PKGBUILD
├── .github/workflows/      # CI（Windows + Linux 双平台）
└── LICENSE
```

## 开发指南

### 技术栈

- **前端**：Vue 3 · Vite · TypeScript · Pinia · vue-i18n · Tailwind CSS v4 · shadcn-vue（本地组件）
- **后端**：Rust · Tauri 2 · tokio · reqwest · tracing

### 架构

后端采用分层架构 `api → domain → infrastructure`：

- `api/` — Tauri 命令薄层，参数校验与响应组装
- `domain/` — 核心业务逻辑（检测循环、录制引擎、配置管理、文件服务、猫耳 API 客户端），纯 Rust 可单测
- `infrastructure/` — 平台能力（托盘、日志、通知、状态存储、环境检查、加密、单实例锁）

运行状态经 tauri state 注入；关键设计决策以模块头部注释记录。

### 常用命令

```bash
npm run dev            # 仅前端（Vite dev server）
npm run build          # 前端类型检查 + 构建
npm run lint           # ESLint（含 no-use-before-define 防 TDZ 回归）
npm test               # 前端单元测试（vitest）
npm run tauri dev      # 开发模式运行桌面应用
npm run tauri build    # 打包安装程序

cd src-tauri
cargo check            # 后端编译检查
cargo test             # 后端单元测试
```

### 代码风格

- 行尾统一 **LF**（`.gitattributes` 已声明，含 `*.md`）
- 注释以中文为主，复杂逻辑与设计决策务必说明「为什么这么写」
- 提交前请确保 `cargo test`、`npm test`、`npm run lint` 与 `npm run build` 全部通过
- ESLint 启用 `no-use-before-define(variables)`：**禁止「引用先于声明」**（防止类似文件页二次进入的 TDZ 回归）

## 测试

- **后端单元测试**：覆盖配置管理（原子写 / 备份 / 损坏恢复 / 导入校验）、文件名模板渲染、SSRF 流地址校验、shell 注入消毒、单实例锁、日志轮转清理 / 级别热更新、路径穿越防护等（`cargo test`，335 用例）
- **前端单元测试**（vitest + @vue/test-utils，97 用例）：覆盖去抖、虚拟滚动范围计算、anchorStore / fileStore 纯逻辑、表单校验、向导 staged→config 映射、文件页月份折叠联动
- **静态质量门**：`npm run lint`（ESLint，含防 TDZ 的 `no-use-before-define`）、`vue-tsc` 类型检查（构建时）
- **端到端测试（E2E）**（Playwright + vite dev + 注入式 tauri mock，`npm run test:e2e`，6 用例）：覆盖首次启动向导全流程（欢迎 → 基本设置 → 环境检查）、主播管理（空态 → 添加 → 列表）、设置页日志级别热更新（保存即生效、无重启横幅）、文件页（文件夹树渲染 / 搜索过滤 / 空态）。mock 在浏览器上下文注入 `window.__TAURI_INTERNALS__`（tauri v2 IPC 边界），纯前端即可跑通完整 UI 流程；测试控制面 `__tauriMock` 支持预置数据与后端事件模拟（见 `e2e/mocks/tauri.ts`）
- **CI**（`.github/workflows/ci.yml`）：Windows + Linux 双平台运行前端 `vue-tsc` 类型检查 + `vite build`，后端 `cargo check` + `cargo test`

## 路线图

以下为后续规划，**当前尚未实现**：

- 🧪 **真后端 E2E（tauri-driver）** — 现有 Web 层 E2E 覆盖完整 UI 流程（vite dev + 注入式 IPC mock）；后续引入 tauri-driver 驱动真后端做冒烟（启动 → 单实例 → 托盘 → 优雅退出）与 Mock 模式 + 假 FFmpeg 的「检测 → 录制 → 文件产出 → 文件列表」链路验证
- 🔍 **禁用检测的主播跳过 API 请求** — 当前每轮全量轮询，`enable_check=false` 仅抑制录制触发不抑制请求
- 🔐 **敏感字段存储升级** — 当前为 XOR + Base64 轻量混淆，计划升级为系统凭据库（DPAPI / keyring）
- ⚙️ **占位项决策** — `custom_dns` 与全局快捷键当前仅落盘未接线，需实装或移除
- 🍎 **macOS 验证** — 补充 macOS CI 与构建目标，验证托盘 / 通知 / 开机自启
- 📦 **文件缓存上限 / 懒加载** — 超大输出目录时内存线性增长
- 🐧 **Linux 真机验证** — ARCH 自包含验证包（`ARCH/`，含 7 个验证脚本 + PKGBUILD）已就绪，待真机执行构建 / 冒烟 / 录制 / 自启验证，并产出 deb / AppImage

已从路线图完成并移除：前端单元测试（vitest 97 用例）、文件列表虚拟滚动 + 搜索防抖、录制序号 `{index}` 移除、Tauri 权限 fail-closed ACL、日志级别热更新（运行时 reload）、Web 层端到端测试（Playwright，`e2e/`）。

## 已知限制

- **敏感字段为轻量混淆而非强加密**：Cookie / 代理密码以 `enc:v1:` 混淆落盘，仅防静态查看，不提供防篡改 / 强保密保证
- **禁用检测的主播仍每轮发起请求**：`enable_check=false` 仅不触发录制，为保持直播状态实时性的设计取舍
- **`custom_dns` 与全局快捷键「配置了未接线」**：字段已落盘但无运行时逻辑（快捷键 UI 已标注）
- **端到端测试（E2E）覆盖 Web 层而非真后端**：tauri-driver 真后端冒烟（启动 / 单实例 / 托盘）与 Mock + 假 FFmpeg 的录制链路验证列为后续（见[路线图](#路线图)）
- **macOS 支持未验证**：无 CI / 构建目标覆盖
- **Linux 无托盘**（显式设计决策）；**FFmpeg 自动下载仅限 Windows**
- **弹幕录制不在计划内**（仅音频轨）
- **端到端测试（E2E）尚未建立**；前端单元测试已覆盖 97 用例（见[测试](#测试)）
- **历史旧文件的分段识别歧义**：移除录制序号后新文件不再产生 `_NNN` 尾部，但早期含录制序号（如 `_001`）的历史文件在文件页可能被识别为分段组（显示层歧义，不影响文件本身）

## 贡献指南

- **报告问题**：请使用 [Issue 模板](https://github.com/thestmitsuki/Missevan-FM-Recorder/issues/new/choose)（`.github/ISSUE_TEMPLATE/bug_report.yml`），附上应用版本、操作系统与架构、复现步骤、诊断报告（设置-关于 → 导出）
- **提交代码**：Fork → 创建分支 → 修改 → 本地通过 `cargo test` 与 `npm run build` → 提交 Pull Request；CI 会自动运行前端 type-check / build 与后端 check / test（Windows + Linux）
- **代码风格**：LF 行尾、中文注释、按分层架构（api / domain / infrastructure）修改

## 许可证

[MIT](LICENSE) © thestmitsuki

内置 FFmpeg 二进制遵循其自身的 LGPL/GPL 许可。
