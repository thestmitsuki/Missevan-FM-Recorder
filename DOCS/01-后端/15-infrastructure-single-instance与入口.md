# 15 · infrastructure/single_instance 与入口 —— 单实例 & 进程入口

> 文件：`src-tauri/src/infrastructure/single_instance.rs`、`src-tauri/src/main.rs`

## 1. single_instance.rs —— 单实例锁

### 背景（双录根因候选②）

应用双开时，两个实例各自持有独立的检测循环、任务表与 FFmpeg 进程表，会为同一主播同时启动两个录制进程（双录）。单实例锁从**进程层面**根治。

### 实现

```rust
pub struct InstanceGuard { /* 持有文件句柄直至 Drop */ }
pub fn acquire() -> Result<InstanceGuard, AcquireError>
```

- `fs2::FileExt::try_lock_exclusive` 独占文件锁（Windows `LockFileEx` / Unix `flock`）；
- 锁文件目录按平台区分：
  - Windows：`dirs::cache_dir()/missevan-recorder`（`%LOCALAPPDATA%\...\cache\missevan-recorder`）；
  - Linux/Unix：`$XDG_RUNTIME_DIR/missevan-recorder`（用户级运行时目录，重启自动清理）；未设置时回退 `~/.cache/missevan-recorder`；
- Guard Drop / 进程崩溃 → 内核释放锁（无死锁残留）；
- 获取失败 → 提示「已在运行」并退出（lib.rs 装配）。

### 实现取舍（Task 7 依赖纪律）

未用 `tauri-plugin-single-instance`：其 2.4.3 rust-version=1.77.2 兼容，但加入后重解析依赖树会把 `tauri-runtime` 的 toml 从 0.9.12 升到 1.1.0（依赖树整体变动）。改用依赖树中已有的 fs2 0.4.3（**零新增依赖**）。

## 2. main.rs —— 进程入口

```rust
#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]
fn main() { missevan_recorder_lib::run() }
```

- Windows release 下 `windows_subsystem = "windows"`：**不弹控制台窗口**（注释警告「DO NOT REMOVE」）；
- 库名 `missevan_recorder_lib`（Cargo.toml `[lib]`，crate-type 含 staticlib/cdylib/rlib）；
- 二进制名 `missevan-recorder`。

## 3. 跨模块依赖

| 依赖 | 用途 |
| --- | --- |
| fs2 0.4.3 | 文件锁 |
| dirs | 平台目录 |
| `lib.rs` | 启动早期 acquire（日志初始化后、Builder 前） |

## 4. 测试

- 同路径二次 acquire 失败（互斥）；
- 不同名锁文件互不影响；
- Drop 释放后可重新获取。

## 5. 已知陷阱

- 锁文件在缓存/运行时目录（不是数据目录），**卸载/清缓存不影响配置**；但手动删除锁目录会导致双开风险——正常流程无需人工干预。
- 单实例锁是进程级防御，**不替代**引擎内的 `is_recording` 双录防御（进程表）：两者互补（双开防不住同一实例内的重复触发）。
- `windows_subsystem` 属性移除会导致 release 版本弹黑窗——提交时勿改。
