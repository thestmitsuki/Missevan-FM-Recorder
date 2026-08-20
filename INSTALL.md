# 猫耳FM录制器（Missevan Recorder）安装手册

> 适用版本：v0.2.0（Tauri 2 + Rust + Vue 3）
> 本手册覆盖 Windows / Linux（AppImage）/ Arch Linux（PKG）三个平台的安装、配置、卸载与常见问题排查。

---

## 目录

1. [构建产物一览](#1-构建产物一览)
2. [如何获取构建产物（手动触发）](#2-如何获取构建产物手动触发)
3. [Windows：NSIS 安装程序](#3-windowsnsis-安装程序)
4. [Windows：单文件绿色版 EXE](#4-windows单文件绿色版-exe)
5. [Linux：AppImage](#5-linuxappimage)
6. [Arch Linux：PKG 安装包](#6-arch-linuxpkg-安装包)
7. [首次运行向导](#7-首次运行向导)
8. [数据与配置文件位置](#8-数据与配置文件位置)
9. [开机自启动](#9-开机自启动)
10. [卸载](#10-卸载)
11. [常见问题排查（FAQ）](#11-常见问题排查faq)
12. [系统要求总表](#12-系统要求总表)

---

## 1. 构建产物一览

仓库通过 GitHub Actions 工作流 `.github/workflows/release.yml` 手动触发构建，一次运行产出 4 个安装包/可执行文件：

| 产物 | 平台 | 文件名示例 | 说明 |
| --- | --- | --- | --- |
| NSIS 安装程序 | Windows | `missevan-recorder_0.2.0_x64-setup.exe` | 标准安装向导，自动创建开始菜单/桌面快捷方式 |
| 单文件绿色版 | Windows | `missevan-recorder_0.2.0_x64_portable.exe` | 免安装，双击即用，前端资源已内嵌 |
| AppImage | Linux 通用 | `missevan-recorder_0.2.0_amd64.AppImage` | 免安装，兼容绝大多数 Linux 发行版 |
| PKG 安装包 | Arch Linux | `missevan-recorder-0.2.0-1-x86_64.pkg.tar.zst` | pacman 原生格式，自动处理依赖 |

> 文件名中的 `0.2.0` 为当前版本号，随 `src-tauri/tauri.conf.json` 中的 version 字段变化。

---

## 2. 如何获取构建产物（手动触发）

构建仅在 **GitHub Actions 手动触发** 时执行，不会因推送代码或打标签自动触发。

1. 打开仓库页面：`https://github.com/thestmitsuki/Missevan-FM-Recorder`
2. 进入 **Actions** 标签页
3. 在左侧工作流列表点击 **Build Release**
4. 点击右侧 **Run workflow** 按钮
   - 分支选择 `main`（或需要构建的分支）
   - 点击绿色的 **Run workflow** 确认
5. 等待三个任务全部完成（Windows / AppImage / Arch PKG 并行构建，通常 10~30 分钟）
6. 在运行记录页面底部的 **Artifacts** 区域下载：
   - `windows-*` → NSIS 安装程序 + 绿色版 EXE
   - `appimage-*` → AppImage 文件
   - `arch-pkg-*` → PKG 安装包

> 说明：手动触发的构建产物以 Actions Artifacts 形式提供（需要登录 GitHub 下载），不会自动发布 GitHub Release。

---

## 3. Windows：NSIS 安装程序

### 3.1 系统要求

- Windows 10 / 11（64 位）
- **Microsoft Edge WebView2 运行时**（Win10/11 一般已预装；若缺失，应用无法启动，见 [FAQ](#111-webview2-运行时缺失)）
- 建议磁盘剩余空间 ≥ 500 MB

### 3.2 安装步骤

1. 双击 `missevan-recorder_0.2.0_x64-setup.exe`
2. 若出现 Windows SmartScreen 提示，点击 **更多信息 → 仍要运行**（软件未做代码签名，属正常提示，见 [FAQ](#114-smartscreen-提示)）
3. 按安装向导提示选择安装目录（默认安装到当前用户目录 `%LOCALAPPDATA%\missevan-recorder`）
4. 点击 **安装**，等待完成
5. 安装完成后可选择立即运行，或在开始菜单中找到 **Missevan Recorder** 启动

### 3.3 安装后的文件布局

```
%LOCALAPPDATA%\missevan-recorder\
├── missevan-recorder.exe      ← 主程序
├── config\                    ← 应用配置（保存在程序目录旁）
└── uninstall.exe              ← 卸载程序
```

> 应用数据（日志、网络日志等）保存在 `%APPDATA%\missevan-recorder`，录音输出目录由用户在向导/设置中指定，详见[第 8 节](#8-数据与配置文件位置)。

---

## 4. Windows：单文件绿色版 EXE

### 4.1 特点

- **免安装**：单个 `.exe` 文件，前端资源已内嵌，无需额外文件即可运行
- **免写入系统**：不写注册表（开机自启动除外，见[第 9 节](#9-开机自启动)）、不创建快捷方式
- **便携**：可放入 U 盘 / 任意目录 / 云盘同步目录，换电脑拷走即用

### 4.2 使用方法

1. 将 `missevan-recorder_0.2.0_x64_portable.exe` 放到任意目录（建议放到如 `D:\Tools\Missevan-Recorder\` 这样的专属目录）
2. 双击运行即可
3. 如需开始菜单/桌面快捷方式，手动发送快捷方式即可（绿色版不会自动创建）

### 4.3 注意事项

- 依赖系统自带的 **WebView2 运行时**（与安装版一致）
- 配置文件保存在 **exe 所在目录的 `config\` 子文件夹**，请勿把 exe 单独拷走而丢弃 config（会丢失已保存的设置）
- 升级时直接替换 exe 文件即可，config 目录保留则设置不丢
- 应用为**单实例**设计：重复双击不会开第二个进程，而是聚焦已有窗口

---

## 5. Linux：AppImage

### 5.1 系统要求

- 任意主流 x86_64 Linux 发行版（Ubuntu / Debian / Fedora / openSUSE / 国产发行版等）
- 运行所需系统库（Tauri 2 / WebKitGTK 依赖），缺失时启动报错，见 [FAQ](#112-linux-启动报错缺少-so-库)
- 可选：`libfuse2`（部分发行版运行 AppImage 必需，见 [FAQ](#113-appimage-报-fuse-错误)）

### 5.2 安装（本质是授权可执行）

AppImage 无需"安装"，两步即可运行：

```bash
# 1. 赋予可执行权限
chmod +x missevan-recorder_0.2.0_amd64.AppImage

# 2. 运行
./missevan-recorder_0.2.0_amd64.AppImage
```

也可以在文件管理器中右键 → 属性 → 勾选"允许作为程序执行"，然后双击运行。

### 5.3 桌面集成（可选）

- **方式一（推荐）**：安装 [AppImageLauncher](https://github.com/TheAssassin/AppImageLauncher)，双击 AppImage 时选择"Integrate and run"，自动生成桌面图标与开始菜单项。
- **方式二**：手动创建 `.desktop` 文件到 `~/.local/share/applications/`，将 `Exec` 指向 AppImage 的绝对路径。
- **方式三**：每次从终端运行，不做集成。

### 5.4 常见发行版运行依赖

| 发行版 | 安装命令 |
| --- | --- |
| Ubuntu / Debian | `sudo apt install libwebkit2gtk-4.1-0 libgtk-3-0 libsoup-3.0-0 librsvg2-2 libappindicator3-1` |
| Fedora | `sudo dnf install webkit2gtk4.1 gtk3 libsoup3 librsvg2 libappindicator-gtk3` |
| Arch / Manjaro | `sudo pacman -S webkit2gtk-4.1 gtk3 libsoup3 librsvg libappindicator-gtk3` |

> FFmpeg 不在应用内打包，首次运行向导会检查；Linux 需自行安装系统包（如 `sudo apt install ffmpeg` / `sudo pacman -S ffmpeg`）。

---

## 6. Arch Linux：PKG 安装包

### 6.1 安装

```bash
# 在下载目录执行（文件名按实际版本）
sudo pacman -U missevan-recorder-0.2.0-1-x86_64.pkg.tar.zst
```

`pacman` 会自动安装依赖：`webkit2gtk-4.1`、`gtk3`、`libsoup3`、`librsvg`、`libappindicator-gtk3`。

### 6.2 安装后的文件

| 路径 | 说明 |
| --- | --- |
| `/usr/bin/missevan-recorder` | 主程序（可命令行直接运行） |
| `/usr/share/applications/missevan-recorder.desktop` | 桌面启动项（应用菜单可见） |
| `/usr/share/icons/hicolor/128x128/apps/missevan-recorder.png` | 图标（128px） |
| `/usr/share/icons/hicolor/512x512/apps/missevan-recorder.png` | 图标（512px） |

安装后可在应用菜单中搜索 **Missevan Recorder** 启动。

### 6.3 说明

- 配置保存在 `~/.config/missevan-recorder`（XDG 标准，因 `/usr/bin` 只读，配置不再跟随程序目录）
- 升级：下载新版 PKG 后再次 `sudo pacman -U <新版.pkg.tar.zst>` 即可覆盖安装

---

## 7. 首次运行向导

首次启动会进入 **设置向导**（安装/绿色版/AppImage/PKG 均相同）：

1. **环境检查**：自动检测 `FFmpeg` / `ffprobe`、磁盘空间与写入权限
2. **FFmpeg 处理**：
   - Windows：可在向导内一键下载 FFmpeg（需联网，有进度条）
   - Linux：向导提示用系统包管理器安装（如 `sudo pacman -S ffmpeg`）
3. **输出目录**：选择录音文件保存位置（默认 `./recordings`，建议改为固定目录如 `D:\Music\猫耳录音` 或 `~/Music/猫耳录音`）
4. **完成**：进入主界面，可添加主播并开启"检测与自动录制"

---

## 8. 数据与配置文件位置

| 内容 | Windows（安装版/绿色版） | Linux（AppImage/PKG） |
| --- | --- | --- |
| 应用配置（设置、主播列表等） | `{程序目录}\config\` | `~/.config/missevan-recorder/` |
| 应用数据（日志、网络日志、崩溃信息） | `%APPDATA%\missevan-recorder\` | `~/.local/share/missevan-recorder/` |
| 录音输出 | 向导/设置中指定的输出目录 | 同左 |

要点：

- **Windows 配置跟程序走**（exe 旁的 `config` 目录）：绿色版换目录 = 配置目录跟着变；安装版卸载时会一并删除。
- **Linux 配置在用户目录**：AppImage 删除后配置仍在 `~/.config/missevan-recorder`；PKG 卸载时 `pacman` 不会删除用户配置，重装可保留。
- **备份建议**：迁移电脑时备份上表中的"应用配置"目录即可完整迁移设置；录音文件位于输出目录，请自行备份。
- **敏感字段**：配置中的敏感项（如 Cookie/令牌）以绑定"可执行文件路径 + 主机名"的密钥混淆保存，换机器或换目录后会自动回退为明文并重新保存——属设计行为。

---

## 9. 开机自启动

应用内设置"开机自启动"后：

| 平台 | 实现方式 |
| --- | --- |
| Windows | 写入注册表 `HKCU\...\CurrentVersion\Run`，键名 `MissevanRecorder`，命令为 `"{程序路径}" --minimized`（以 `--minimized` 参数启动时驻留托盘、不弹主窗口） |
| Linux | 写入 XDG autostart 文件 `~/.config/autostart/` |

- 关闭自启动：在应用设置中关闭，或手动删除注册表项/autostart 文件
- `--minimized` 仅在托盘创建成功时生效；Linux 无托盘（决策 #2），该参数在 Linux 上恒显示主窗口
- 绿色版 EXE 的自启动指向 exe 当前路径，移动 exe 后需重新开启自启动

---

## 10. 卸载

### Windows（NSIS 安装版）

1. **设置 → 应用 → 已安装的应用**，搜索 *Missevan Recorder* → 卸载
2. 或运行安装目录中的 `uninstall.exe`
3. 卸载后建议手动删除残留：`%APPDATA%\missevan-recorder`（日志数据）

### Windows（绿色版）

- 直接删除 exe 及其所在目录（如需保留设置，先备份 `config`）
- 若曾开启自启动，先在应用设置中关闭，或手动删除注册表 `HKCU\...\CurrentVersion\Run` 中的 `MissevanRecorder` 键

### Linux（AppImage）

- 删除 AppImage 文件即可
- 若做过桌面集成，删除 `~/.local/share/applications/` 下对应的 `.desktop` 文件
- 可选清理：`rm -rf ~/.config/missevan-recorder ~/.local/share/missevan-recorder`

### Arch Linux（PKG）

```bash
sudo pacman -R missevan-recorder
```

> 如需连同用户配置一起删除：`sudo pacman -Rns missevan-recorder` 后手动删除 `~/.config/missevan-recorder`、`~/.local/share/missevan-recorder`。

---

## 11. 常见问题排查（FAQ）

### 11.1 WebView2 运行时缺失

**现象**：Windows 双击后无反应，或弹出"未找到 WebView2 运行时"错误。

**解决**：前往 [Microsoft Edge WebView2](https://developer.microsoft.com/microsoft-edge/webview2/) 下载并安装 **Evergreen 运行时**（永久链接版），重启应用。

### 11.2 Linux 启动报错缺少 .so 库

**现象**：`error while loading shared libraries: libwebkit2gtk-4.1.so.0 ...` 或类似提示。

**解决**：按[第 5.4 节](#54-常见发行版运行依赖)的表格安装对应系统依赖后重试。

### 11.3 AppImage 报 FUSE 错误

**现象**：`AppImages require FUSE to run` / `fuse: failed to exec fusermount`。

**解决**：

```bash
# Ubuntu/Debian
sudo apt install libfuse2

# Fedora
sudo dnf install fuse

# 或绕过 FUSE 直接解包运行
./missevan-recorder_0.2.0_amd64.AppImage --appimage-extract-and-run
```

### 11.4 SmartScreen 提示

**现象**：Windows 弹出"Windows 已保护你的电脑"蓝屏提示。

**原因**：软件未购买代码签名证书，SmartScreen 无法验证发布者。

**解决**：点击 **更多信息 → 仍要运行**。文件来自本仓库 GitHub Actions 构建，可校验产物哈希后放心使用。

### 11.5 双击无反应（Windows）

1. 打开任务管理器确认是否已在运行（单实例设计会聚焦已有窗口而非新开）
2. 查看日志：`%APPDATA%\missevan-recorder\logs\`（如有）
3. 确认 WebView2 运行时已安装（见 11.1）

### 11.6 录制没有声音 / 没有文件

1. 确认向导中 **FFmpeg 环境检查** 通过（无 FFmpeg 无法录制）
2. 检查输出目录是否有写入权限、磁盘是否充足
3. 在应用"调试/日志"页查看实时日志与网络日志
4. 确认主播开启检测且处于开播状态（录制引擎只在检测到开播后启动）

### 11.7 日志在哪里

- Windows：`%APPDATA%\missevan-recorder\logs\`
- Linux：`~/.local/share/missevan-recorder/logs/`
- 应用内"调试"页面可查看实时日志与网络请求日志，排查问题时建议先截图该页面

---

## 12. 系统要求总表

| 项 | Windows | Linux（AppImage） | Arch Linux（PKG） |
| --- | --- | --- | --- |
| 系统 | Windows 10/11 64 位 | 主流 x86_64 发行版 | Arch / Manjaro / EndeavourOS 等 |
| 架构 | x64 | amd64（x86_64） | x86_64 |
| WebView2 运行时 | 必需（一般预装） | —（使用系统 WebKitGTK） | —（webkit2gtk-4.1） |
| FFmpeg | 向导内可一键下载 | 系统包安装 | `sudo pacman -S ffmpeg` |
| 网络 | 首次需联网（FFmpeg 下载/主播检测） | 同左 | 同左 |
| 磁盘 | ≥ 500 MB（不含录音） | 同左 | 同左 |

---

*手册生成于 2026-08-21，对应 release.yml 手动触发构建产物（Windows NSIS + 绿色版 EXE、Linux AppImage、Arch PKG）。*
