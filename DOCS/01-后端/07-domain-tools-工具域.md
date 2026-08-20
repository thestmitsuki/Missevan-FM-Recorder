# 07 · domain/tools —— 工具域

> 文件：`src-tauri/src/domain/tools.rs`

## 1. 职责

FFmpeg / ffprobe 可执行文件路径解析（统一候选顺序）、Windows 子进程无控制台标志、子进程回收辅助。

## 2. 核心能力

### ffmpeg_candidates（统一候选顺序）

```rust
pub fn ffmpeg_candidates(config_ffmpeg_path: Option<&str>) -> Vec<PathBuf>
```

**配置指定路径（若非空）→ `{exe_dir}/ffmpeg/<工具>[.exe]`（若存在）→ PATH 裸名**

- Windows 首启向导下载的便携版位于 `{exe_dir}/ffmpeg/`（`wizard_cmds::download_ffmpeg` 解压产物）；
- Linux 无便携版（候选不命中，仅保持顺序一致）；
- 所有消费方**必须**遵循同一候选顺序：健康检查候选（`checker/checks.rs`）、录制引擎 spawn（`resolve_ffmpeg_executable`）、向导/调试信息（`debug_cmds` / `wizard_cmds`），否则出现「配置没写路径 → 找不到已下载 FFmpeg」的漏匹配（历史坑）。

### no_console_window（Windows）

- 为子进程设置 `CREATE_NO_WINDOW`（0x08000000），禁止新建控制台窗口；
- 发布构建 `windows_subsystem = "windows"` 下父进程无控制台，spawn 控制台子系统子进程（ffmpeg/ffprobe/cmd）且未设标志 → Windows 会**新建黑窗口闪现**；
- 录制引擎已内置同款处理（recorder/builder.rs），本辅助供工具探测（debug/wizard/checks）与录制后命令（monitor.rs）共用；
- 调用方式：`cmd.as_std_mut()`（tokio Command 包装 std Command，spawn 时生效）。

### reap_in_background

- 子进程句柄回收（后台线程 wait + 忽略退出码），避免句柄泄漏；单测覆盖 spawn 与回收线程 join。

## 3. 跨模块依赖

| 消费方 | 用途 |
| --- | --- |
| `infrastructure/checker/checks.rs` | FfmpegCheck / ffprobe 检查候选（`prepared_candidates` + `clean_path` 清洗） |
| `domain/recorder/engine.rs` | `resolve_ffmpeg_executable` |
| `api/debug_cmds.rs` / `api/wizard_cmds.rs` | 工具探测 |
| `domain/recorder/monitor.rs` | 录制后命令（no_console_window） |

## 4. 测试

- 候选顺序（配置路径优先 / exe_dir 命中 / PATH 兜底）；
- Windows 标志位设置；
- 子进程回收（spawn + join 不 panic）。

## 5. 已知陷阱

- 新增「探测工具」逻辑时必须复用 `ffmpeg_candidates`，否则与引擎口径不一致（漏匹配）。
- `clean_path`（checker 侧）清洗控制字符与双向文本符（U+202A–U+202E 等），与 `sanitize_path_component`（template）职责不同——前者防路径注入展示，后者防文件名逃逸，别混用。
- PATH 裸名命中时 `found.path` 返回工具名本身（非解析后绝对路径），调试页展示以此为准。
