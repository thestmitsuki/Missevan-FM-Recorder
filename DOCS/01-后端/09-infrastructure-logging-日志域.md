# 09 · infrastructure/logging —— 日志域

> 文件：`src-tauri/src/infrastructure/logging/{setup,buffer,network}.rs`

## 1. 职责

基于 tracing 的三通道日志体系：

1. **文件日志**（setup.rs）：滚动写入数据目录；
2. **内存环形缓冲**（buffer.rs）：调试页「实时日志」（容量 1000，节流 100 条/s 推送前端）；
3. **网络插桩缓冲**（network.rs）：调试页「网络请求」（容量 500，spider 调用点写入）。

## 2. setup.rs —— 初始化与热更新

### init_logging

- 组装 `tracing_subscriber`：EnvFilter + 文件 appender（非阻塞 worker，`WorkerGuard` 需保活）+ 自定义 `LogLayer`；
- 级别白名单 `normalize_log_level`：仅 error/warn/info/debug/trace，非法回退 `info`（纯函数，启动与热更新共用）；
- **运行时热更新**（U5）：`LogLevelReload` 机制——`save_config` 改 `log_level` 后不重启即生效；
- 进程级 panic hook（lib.rs）：`tracing::error!` 写入 `[panic]` + 默认 hook 链式调用（保留 backtrace 提示）。

### 脱敏

`sanitize_message`：Cookie / Authorization / Password 值 → `***`（冒号后取键值，含 `Cookie:` 无值不替换等边界），写入缓冲与网络记录前统一脱敏。

## 3. buffer.rs —— LogBuffer / LogLayer

```rust
pub struct LogBuffer { /* 容量 1000 环形，超限丢最旧 */ }
pub struct LogLayer { /* tracing Layer：捕获事件 → 脱敏 → 写缓冲 + emit "debug:log" */ }
pub struct LogEntry { timestamp(RFC3339), level(小写), module(target), message }
```

- **事件契约**：`debug:log` 载荷 `LogEntry`；`module` 为 tracing target（前端「来源过滤」按子串匹配）。
- 节流：最大 100 条/秒 emit，防前端刷屏。
- 查询：`get_logs`（按级别/来源/文本过滤 + 分页）与 `clear_logs`。

## 4. network.rs —— NetworkStore（全局单例）

```rust
pub fn global_store() -> &'static NetworkStore   // OnceLock 进程级单例
pub struct NetworkLog { timestamp, method, url, status, duration_ms, room_id, error }
```

- 容量 500 环形缓冲；`spider.rs` 调用点记录（不引入 reqwest middleware 的取舍，见 spider 文档）；
- 脱敏（URL / 错误信息中的敏感值 → `***`）；
- 查询：`get_network_logs`（过滤 + 分页）/ `clear_network_logs`。

## 5. 跨模块依赖

| 依赖 | 用途 |
| --- | --- |
| tauri AppHandle / Emitter | emit `debug:log` |
| `api/debug_cmds.rs` | 日志/网络记录查询命令 |
| `domain/spider.rs` | 网络插桩写入 |
| `domain/config`（save_config） | log_level 热更新 |
| `lib.rs` | init_logging + panic hook + `.manage` |

## 6. 测试

- 环形缓冲容量与丢旧语义；
- 脱敏规则矩阵（Cookie/Authorization/Password、无值边界、多键行）；
- 日志级别过滤（升/降级后可见性）与热更新行为；
- 网络记录写入/查询/清空；
- sanitize 与级别 normalize 的纯函数测试。

## 7. 已知陷阱

- **`WorkerGuard` 必须保活**：drop 会停 flush，日志丢失——lib.rs 持有到进程退出。
- 脱敏是**写缓冲前**的，tracing 文件里原始日志可能含敏感值（文件是本地数据目录，勿提交/勿外发）；诊断导出（`export_diagnostic_report`）会再脱敏一遍。
- 网络插桩在 spider 调用点手动调用 `network::log(...)`——新增 API 调用忘记插桩时，调试页看不到该请求（不是 bug，是插桩点缺失）。
- emit 节流（100/s）是丢事件式节流（非合并），高并发日志时前端看到的是抽样。
