# 16 · lib.rs —— 应用装配（核心枢纽）

> 文件：`src-tauri/src/lib.rs`（745 行，全项目最关键的装配文件）

## 1. 职责

Tauri 应用的**唯一装配点**：panic hook、日志、单实例、共享状态创建与注入、插件注册、命令注册、窗口管理（wizard/main 分流）、检测循环启动、窗口关闭决策、优雅退出。

## 2. run() 执行顺序（全景）

```
main() → lib::run()
 │
 ├─ 1. install_panic_hook()
 │      取回默认 hook → set 包装 hook（tracing::error 写 [panic] + 链式默认 hook）
 │      进程级：主线程与 tokio/tauri 线程的 panic 都经过；不可先覆盖默认 hook（会吞 backtrace）
 │
 ├─ 2. init_logging()（tracing：文件 + LogLayer 缓冲 + 脱敏）
 │
 ├─ 3. acquire() 单实例锁 → 失败：提示 + 退出
 │
 ├─ 4. 共享状态装配：
 │      ConfigManager::new（加载/损坏恢复）
 │      RecorderState（tasks / pending_starts / history / crash / avatar caches / mock_store）
 │      FfmpegRecorder::new()（⚠️ 全局唯一共享实例 —— 双录防御 #2/#3 前提）
 │      FileCache、LogBuffer、NetworkStore、live_cache、avatar_cache(+negative)、
 │      NotificationDispatcher、detection_wake(Notify)
 │
 ├─ 5. Tauri Builder：
 │      plugins: dialog / notification / autostart("MissevanRecorder", --minimized) / opener
 │      .manage() 注入全部共享状态
 │      .invoke_handler(54 命令)
 │      .setup(|app| { ... })   ← 核心业务装配（见 §3）
 │      .on_window_event(|win, event| { close 决策 })
 │
 └─ 6. run(generate_context!())
```

## 3. setup 闭包内做什么

1. **首启判定**：`config_manager.global_config_path().exists()`
   - 首启：显示 wizard 窗、隐藏 main 窗；
   - 非首启：`wizard.destroy()`（**不用 close()**——wizard 前端注册了 onCloseRequested 且 prevent_default，close 会被无条件取消；setup 期 JS 未挂载也有竞态；destroy 不触发事件）。
2. **提取 Arc**（config_manager / notifier / live_cache / avatar caches / recorder_shared / detection_wake / log_buffer / network_store ...）供闭包捕获。
3. **启动 DetectionLoop**（tokio::spawn）——见检测域文档。
4. **注入 app_handle** 到 RecorderState（`OnceLock::set`，后续命令 emit/退出用）。
5. 注册窗口事件（close → should_hide_to_tray 决策）。
6. 返回 Ok。

## 4. 关闭流程（do_shutdown / on_window_event）

```
窗口 close 事件：
 ├─ should_hide_to_tray(close_behavior, is_exiting, tray_available)？
 │    是 → prevent_default + 隐藏窗口（后台运行）
 │    否 → 继续关闭 → 走 do_shutdown
 └─ 托盘「退出应用」→ 同一 do_shutdown

do_shutdown：
 ├─ 保存配置（ConfigManager）
 ├─ 停 DetectionLoop（running 标志置 false）
 ├─ cancel 全部 tasks（等 JoinHandle ≤5s）
 ├─ try_state 取回 FfmpegRecorder → 强制终止残留 ffmpeg 进程（B2 退出兜底）
 └─ app.exit(0)
```

## 5. 命令注册表（54 个，generate_handler!）

分组见 `api-命令层.md`；注册时用 `crate::api::xxx::yyy` 全路径引用，命令须 `pub(crate)`（宏可见性）。

## 6. 关键设计注释（历史坑，勿回归）

| 注释编号 | 内容 |
| --- | --- |
| #2/#3 | 共享 FfmpegRecorder：进程表全局唯一是双录防御前提；**每次录制 new 实例 = 回归** |
| #12 | tray 模块 Linux 仍编译但 `try_state` 恒 None → 关闭即退出 |
| B2 | 退出兜底：do_shutdown 经 try_state 强制终止剩余录制进程 |
| O2 | 头像失败负缓存（avatar_negative_cache），防坏 URL 反复请求 |
| M1 | 配置原子写锁（std Mutex 串行化 tmp+rename） |
| S2a/S2b/S3 | 磁盘预检 / 崩溃熔断 / 磁盘通知节流（recorder/disk.rs） |
| U5 | 日志级别热更新 |
| L5 | start_recording 占位禁用语义（api/recording_cmds.rs） |

## 7. 测试

- `install_panic_hook` 相关纯函数（panic payload 提取：字符串 / 非字符串占位）；
- 其余为集成行为（无单测），靠模块级单测 + CI 把关。

## 8. 已知陷阱（装配期）

- **共享状态必须 `.manage()` 且闭包持有 Arc**：setup 闭包与命令都依赖注入；漏 manage → 运行时 `state not managed` panic。
- `wizard.destroy()` 与 `finish_wizard` 是两条销毁路径（setup 首启判定 vs 用户走完向导），都不得用 `close()`（prevent_default 竞态）。
- `OnceLock::set` 的 app_handle 在 setup 注入；若未来在 setup 之前有命令被调用会 panic/get None——保持现有顺序。
- 新增命令/插件/状态时，同步更新本文件 4 处：插件列表、manage、invoke_handler、setup 内的闭包捕获。
