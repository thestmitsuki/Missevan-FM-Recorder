# 03 · domain/detector —— 检测域

> 文件：`src-tauri/src/domain/detector/{mod,loop,stats}.rs`

## 1. 职责

后台**直播检测循环**：周期性轮询关注主播的开播状态，驱动自动录制启动；同时维护检测统计与「直播中」状态归并。

## 2. 模块划分

| 文件 | 内容 |
| --- | --- |
| `mod.rs` | `merge_live_state(api_live, is_recording) -> bool`：**直播展示状态 = API 检测结果 ∥ 正在录制**。归并点统一在后端状态生产处（检测循环事件 / get_recording_status / 调试统计），前端不重复归并（避免事件时序回闪） |
| `loop.rs` | `DetectionLoop`：异步循环 + 重试退避 + 429 冷却 + mock 分支 + 录制启动触发 |
| `stats.rs` | `DetectorStats`：原子计数器（total/success/failed/unknown、last_check_at、running）+ 快照 |

## 3. DetectionLoop 核心流程

```
spawn(loop)
 └─ 初始化：读取配置（check_interval_secs 等）
 └─ 循环：
     1. 抖动 sleep：interval + random(0..60)s（降低平台风控）
     2. mock 模式？→ 从 MockStore 读条目，跳过网络
     3. 对每个 enable_check 主播：
        a. 429 冷却中？→ skip（记 unknown）
        b. MissevanClient::check_live
        c. 错误分类 → 退避/冷却/未知判定（见 spider 文档）
        d. 成功 → 更新 live_cache[room_id]
     4. 状态归并 + 推送 recording_status_changed
     5. 对「直播且未录制」的主播 → 触发录制启动（engine::start_ffmpeg_recording）
     6. 统计写入 stats
```

关键设计：

- **重试退避**：`retry_delay_base` 指数退避到上限；429 进入冷却（`RateLimiter`：冷却期内所有主播检测跳过，记 unknown）。
- **双录防御**：启动录制前 `is_recording` 检查（进程表全局唯一实例的锁内去重）。
- **可唤醒**：`detection_wake: Arc<Notify>`——`finish_wizard` 调用 `notify_one()` 立即执行一轮检测（而非等下一个 interval）。
- **可停止**：持有 `Arc<AtomicBool>` running 标志（do_shutdown 置 false）；循环退出时清理。
- **mock 模式**：`MockStore.is_mock_mode()` 时全部主播检测走内存数据；`mock://` 占位流地址（空串 = 故意无效地址，用于测试 FFmpeg 失败路径）。
- 每主播独立 `enable_check`：关闭检测的主播不参与轮询（`stop_recording_if_check_disabled` 逻辑在 anchor_cmds 处理「关闭检测时若在录制则停止」）。

## 4. DetectorStats 计数口径

| 计数 | 口径 |
| --- | --- |
| `total_checks` / `success_checks` / `failed_checks` | 按**单次主播检测**计（mock 同样计数） |
| `unknown_checks` | Server/Network/Format 错误 + 429 冷却跳过；计入 failed_checks（规格：未知计入失败数），另单独计数便于观察 |
| `last_check_at` | 每轮检测开始时间 |
| `enabled_anchors` / `live_anchors` / `recording_anchors` | 由命令层从配置 / live_cache / AppState 实时聚合（快照默认 0） |

调试页「检测循环」模块通过 `get_detector_stats` 读取快照。

## 5. 跨模块依赖

| 依赖 | 用途 |
| --- | --- |
| `domain/config` | 检测间隔 / 主播列表 / 录制参数 |
| `domain/spider` | `MissevanClient::check_live` + `CheckErrorKind` |
| `domain/recorder/disk` | 每轮磁盘检查（低开销 statfs） |
| `domain/recorder/engine` | 触发录制启动 |
| `infrastructure/state/app_state` | 录制状态查询 / live_cache |
| `infrastructure/state/mock_store` | mock 数据源 |
| `infrastructure/notification` | 直播开播/结束、API 失败、磁盘告警通知 |

## 6. 测试

- `merge_live_state` 真值表（4 组合）；
- 退避序列（指数增长至上限）；
- 429 冷却（冷却期阻止、到期放行、冷却归零后从 60s 重新开始而非累积）；
- mock 计数与真实计数一致性。

## 7. 已知陷阱

- **检测间隔抖动**是防风控设计（0–60s），调试时注意不是 bug。
- `merge_live_state` 语义：API 判直播但录制未启动（门控原因）→ 仍显示直播；API 判离线但 FFmpeg 在录 → 保持直播（离线可能是 API 抖动/风控误报）。
- 循环的「每轮磁盘检查」与 monitor 的「每 5 分钟磁盘检查」是两条独立路径，共用 `check_disk_space`，改动阈值语义时两处都要看（0 = 不限制）。
- 新增录制触发条件时，必须保持 `is_recording` 锁内去重在前（双录防御 #2/#3 的进程表在 FfmpegRecorder 单例上，不是 AppState.tasks）。
