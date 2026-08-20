# 04 · domain/recorder —— 录制域

> 文件：`src-tauri/src/domain/recorder/{builder,disk,engine,monitor,template}.rs`（`mod.rs` 导出）

**本域是项目最核心、防御最密集的模块**，负责 FFmpeg 录制全链路：命令构建 → 启动（双录/并发/磁盘防线）→ 进程监控 → 崩溃熔断 → 结束后行为。

## 1. 模块地图

| 文件 | 职责 |
| --- | --- |
| `engine.rs` | `FfmpegRecorder`（进程表）+ `start_ffmpeg_recording`（启动防线）+ 任务注册 + 状态事件 |
| `monitor.rs` | `monitor_recording`：子进程存活/磁盘巡检、异常退出判定、崩溃标记、清理联动、重试 |
| `builder.rs` | `FfmpegCommandBuilder`：FFmpeg 参数组装（Builder 模式） |
| `template.rs` | 文件名模板渲染（`{anchor_name}/{room_id}/{date}/{time}/{ext}`）+ 路径组件消毒 |
| `disk.rs` | 磁盘阈值检查 + `CrashBackoff`（崩溃熔断）+ `DiskNotifyThrottle`（通知节流） |

## 2. engine.rs —— FfmpegRecorder

### 关键结构

```rust
pub struct FfmpegRecorder {
    processes: Mutex<HashMap<String, Child>>,   // anchor_id -> ffmpeg 子进程（进程表，双录防御 #2/#3 核心）
}
```

- **共享单例**（lib.rs 创建一次，`.manage` 注入）：所有录制启动/监控共用同一实例——进程表全局唯一，`is_recording` 与 `insert_process` 锁内去重才跨启动调用生效。
- 历史坑（注释明确）：此前每次录制 `new` 一个实例，进程表形同虚设 → 双录。禁止回归。

### start_ffmpeg_recording（启动防线，按序）

1. **磁盘预检**（S2a）：`check_disk_space`，低于阈值 → `RC_DISK_LOW` 拒绝 + DISK 通知（节流）。
2. **双录防御 #1**：`is_recording(anchor_id)` → `RC_ALREADY_RECORDING`。
3. **并发上限**：`max_concurrent_recordings >= 1` 且活跃数达上限 → `RC_CONCURRENCY_LIMIT`。
4. 流地址获取（`MissevanClient::get_stream_url`，失败 → REC_API_FAILED 通知）。
5. 构建命令（builder）→ 渲染输出路径（template，含已存在去重 `_2/_3`）→ 创建父目录。
6. spawn（Windows 下 `CREATE_NO_WINDOW` 禁黑窗）→ `insert_process` → 注册 `AppState.tasks` → 状态事件 + REC_START 通知 → `monitor_recording`。

### 关键辅助

- `is_abnormal_exit`：区分「被 cancel 令牌终止」（正常）与「ffmpeg 自己崩溃」（异常）。
- `mark_crash_partials`：崩溃后标记 `.part` 残留（不删除，供排查；自动清理按 mtime 处理）。
- `deduplicate_output_path`：非分段模式目标已存在 → 扩展名前追加 `_2/_3`…（兜底同秒碰撞与上次残留；比旧 `{index}` 模板变量更稳，`{index}` 已移除）。
- `resolve_ffmpeg_executable`：候选顺序 = 配置路径 → `{exe_dir}/ffmpeg/ffmpeg.exe` → PATH（与 tools.rs / checker 口径一致）。

## 3. builder.rs —— FFmpeg 参数

| 项 | 说明 |
| --- | --- |
| 输入 | `-rw_timeout`（读超时，0=不传）、UA、referer、代理（http/socks5 + 认证）、cookie（每主播独立） |
| 音频 | `-vn`（audio_only=true 时）、`-b:a <kbps>k`（bitrate_kbps>0）、`-q:a`（mp3 质量，vbr） |
| 输出 | `-f segment -segment_time <sec>`（分段，segment_seconds>0）、`-y` 覆盖、`-c copy` 或重编码按格式 |
| 平台 | Windows `CREATE_NO_WINDOW`（0x08000000）防黑窗口 |

Builder 为纯函数式组装（输入配置 → 输出 args），单测覆盖参数矩阵（含 `-vn`、`-q:a 2`、UA/代理注入等）。

## 4. monitor.rs —— 录制监控

```
monitor_recording(anchor_id, cancel_token, ...)
 ├─ 循环（每 5s）：
 │    ├─ 子进程存活？→ 退出则 break
 │    ├─ 磁盘检查（5 分钟节流 + DiskNotifyThrottle）
 │    └─ cancel_token 触发？→ 优雅终止
 ├─ 退出后统一出口（cleanup_on_recording_end）：
 │    ├─ 自动清理（retention_days / max_total_gb，跳过活跃文件）
 │    ├─ 刷新文件缓存 → emit recording_files_changed
 │    ├─ 历史追加 RecordingSummary（≤50 条）
 │    ├─ 通知 REC_END / REC_ERROR / REC_CRASH
 │    └─ 重试调度（可重试原因 && max_retries 未耗尽 → 延迟后重新走启动防线）
 └─ post_record_action：none / open_folder / command（${output} 占位 + 超时保护）
```

## 5. template.rs —— 文件名模板

- 变量：`{anchor_name}` `{room_id}` `{date}`（YYYY-MM-DD）`{time}`（HH-MM-SS）`{ext}`。
- 安全：渲染结果按 `/` `\` 拆组件 → 逐组件 `sanitize_path_component`（非法字符/控制字符/`..` → `_`）→ `/` 重拼——**模板无法逃逸出输出目录**。
- 回退：模板空 / 无变量 / 渲染后无有效组件 → 默认模板 `{anchor_name}/{date}_{time}_{anchor_name}.{ext}`。
- 兼容：只作用于新录制，旧文件原样保留。

## 6. disk.rs —— 磁盘保护与熔断

- `check_disk_space(path, threshold_gb)`：fs2 单次 statfs / GetDiskFreeSpaceEx；`Ok{available_gb}` / `Low{available_gb, threshold_gb}` / `Failed`（调用方按放行处理）。
- `CrashBackoff`：连续崩溃计数 → 达阈值暂停自动重启 → 指数退避至上限 → 冷却后恢复（按主播维度）。
- `DiskNotifyThrottle`：DISK 通知冷却（避免磁盘不足期间刷屏）。
- `now_ms()`：毫秒时间戳（app_state 复用）。

## 7. 跨模块依赖

| 依赖 | 用途 |
| --- | --- |
| `domain/config` | 录制参数 / 输出路径 / 结束后行为 |
| `domain/services/file_cache` | 缓存刷新 / 活跃文件保护 |
| `domain/services/cleanup` | 录制结束自动清理 |
| `domain/spider` | 流地址 / 主播名（错误通知文案） |
| `infrastructure/state/app_state` | 任务表 / 历史 / 取消令牌 |
| `infrastructure/notification` | REC_* 事件通知 |
| `infrastructure/error` | AppError / 错误码 |

## 8. 测试

- 命令参数矩阵（builder）；
- 模板渲染（变量替换 / 非法字符 / `..` 逃逸 / 空模板回退 / mp3 扩展名）；
- 输出路径去重（`_2/_3` 递增、`.part` 残留清理语义）；
- 磁盘状态判定（阈值 0 = 不限制）；
- 熔断退避序列、通知节流；
- 崩溃标记（新旧 `.part` 处理差异）。

## 9. 已知陷阱

- **进程表在 FfmpegRecorder 单例上**，不是 AppState——改双录/并发逻辑时先确认改的是哪个表。
- `{index}` 模板变量已移除（`deduplicate_output_path` 更稳），**不要重新引入**。
- 分段模式下文件名去重靠 ffmpeg 自身序列号（`_000` 等），`deduplicate_output_path` 只处理非分段；两套语义别混。
- Windows 子进程必须带 `CREATE_NO_WINDOW`（黑窗闪现问题），spawn 用 tokio Command + `as_std_mut()`。
- 重试必须重新走完整防线（磁盘/并发/双录），不能绕过 `start_ffmpeg_recording`。
- `post_record_command` 是用户自定义命令，超时保护 + `${output}` 占位替换，注意命令注入面（仅本地执行，风险自担）。
