# 02 · domain/config —— 配置域

> 文件：`src-tauri/src/domain/config/model.rs`、`manager.rs`（`mod.rs` 仅导出）

## 1. 职责

- `model.rs`：配置数据结构（`GlobalConfig` / `AnchorConfig` / `Config` 聚合）与字段级校验。
- `manager.rs`：配置生命周期（加载 / 校验 / 原子写 / 备份 / 导入导出 / 敏感字段混淆落盘 / anchors 独立文件管理）。

## 2. 数据模型

### GlobalConfig（`#[serde(default)]`，缺失字段无损升级旧配置）

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `output_dir` | String | `./recordings` | 输出目录 |
| `record_format` | String | `m4a` | `m4a` / `mp3`（`is_valid_record_format` 校验） |
| `segment_seconds` | u64 | 0 | 0 = 不分割 |
| `disk_space_limit_gb` | u64 | 10 | 0 = 不限制 |
| `ffmpeg_path` | Option\<String\> | None | 配置指定路径优先 |
| `anchor_ids` | Vec\<String\> | [] | **遗留字段**：早期「启用主播 ID 列表」，当前版本不使用（主播实存于 anchors/*.toml），保留仅为向后兼容 |
| `check_interval_secs` | u64 | 120 | 检测间隔 |
| `max_retries` | u32 | 3 | 录制重试次数 |
| `retry_delay_secs` | u64 | 5 | 重试间隔 |
| `autostart` / `close_behavior` / `show_tray` / `check_updates` | — | — | 通用项（`close_behavior`: `tray`/`exit`） |
| `bitrate_kbps` | u32 | 128 | 音频码率 |
| `audio_only` | bool | true | 仅音频轨 |
| `filename_template` | String | 默认模板 | 见 `template.rs` |
| `pre_record_delay_secs` | u64 | 0 | 开播→录制延迟窗口（可取消） |
| `max_concurrent_recordings` | u64 | 0 | 0 = 不限 |
| 通知 8 项 | — | — | `notifications_enabled` / `notify_system` / `notify_sound` + 7 事件勾选（映射 `NotifySettings`） |
| 代理 4 项 | — | — | `proxy_type`(`none`/`http`/`socks5`) / `proxy_url` / `proxy_port` / `proxy_username` / `proxy_password`（混淆落盘） |
| `post_record_action` | String | `none` | `none`/`open_folder`/`command`；`post_record_command` 支持 `${output}` 占位 |
| `cleanup_time`（遗留） | — | — | 定时清理已移除（M 审查跟进），字段保留兼容 |
| `log_level` | String | `info` | 小写，支持运行时热更新 |

### AnchorConfig（每主播一个 `anchors/<id>.toml`）

`id`（uuid）/ `name` / `url`（fm.missevan.com/live/\<数字\>）/ `room_id` / `proxy?` / `cookie?` / `enable_check` / `tags: Vec<String>`（固定 5 标签：音乐/唱歌/日常/ASMR/杂谈）/ `avatar_url`（动态获取，不落盘）。

### Config 聚合

```rust
pub struct Config {
    pub global: GlobalConfig,
    pub anchors: Vec<AnchorConfig>,
}
```

## 3. 持久化设计

```
数据目录（dirs::data_dir()/missevan-recorder 或 app_config_dir）
├── config.toml              # 全局配置（原子写：tmp + rename）
├── config.toml.bak.<ts>     # 备份（保留最近 5 份，MAX_BACKUPS）
└── anchors/
    └── <anchor_id>.toml     # 每主播独立文件
```

- **原子写**（M1 审查跟进）：std `Mutex` 串行化「写 `config.toml.tmp` → rename 替换」整段；无锁时并发写（save ∥ import / load 恢复回写 ∥ save）会互相覆盖 tmp 文件。
- **损坏恢复**：load 失败 → 备份损坏文件 → 回退默认配置（用户数据不丢，下次保存生成新文件）。
- **敏感字段**：cookie / 代理密码经 `infrastructure::crypto` 混淆（`enc:v1:` 前缀）后落盘；读取时 `deobfuscate_or_plain`（旧明文兼容）。
- **导入导出**：`import_config`（replace / merge 两种模式 + `ImportSummary{global_replaced, anchors_added/removed/skipped/total}`）；`export_config` 输出完整 TOML。

## 4. 校验规则（save/import 共用）

- `record_format` ∈ {m4a, mp3}；`output_dir` 非空；`check_interval_secs` 上下限；
- 各数值字段上限（超限报错，错误消息含实际值与上限，便于用户理解）；
- cookie 长度上限（`COOKIE_MAX_LEN`，anchor_cmds 的 `validate_cookie_len` 与 ConfigManager 双保险）。

## 5. 跨模块依赖

| 消费方 | 用途 |
| --- | --- |
| 全部 api 命令 | 读/写配置与主播 |
| `detector/loop.rs` | 读检测间隔/启用状态/录制参数 |
| `recorder/engine.rs`、`builder.rs`、`template.rs`、`monitor.rs` | 录制参数 + 输出路径 + 结束后行为 |
| `services/cleanup.rs`、`file_cache.rs` | 清理策略 / 扫描根目录 |
| `infrastructure/state/app_state.rs` | 任务参数快照 |
| `api/fs_utils.rs`、`update_cmds.rs`、`debug_cmds.rs` | 输出目录 / 代理 / 调试信息 |

## 6. 测试

- 模型反序列化兼容（旧配置缺字段 / 遗留字段 anchor_ids、cleanup_time）；
- 字段校验（非法格式 / 超上限报错文案含上下限）；
- 原子写与备份轮转（保留 5 份，字典序=时间序）；
- 导入导出（replace/merge 语义、重复 id 跳过）；
- 损坏配置恢复；
- 混淆往返（enc:v1: 前缀、错密钥不可还原）。

## 7. 已知陷阱

- `anchor_ids` 与 `cleanup_time` 是遗留字段：**只做兼容，不消费**；前端也不再写入/读取（保存/导入按空列表带过）。
- 新增全局配置项 = 三处同步：`model.rs` 字段（+默认值）、前端 `types/config.ts` 的 `GlobalConfig` 与 `DEFAULT_CONFIG`（`stores/configStore.ts`）、设置页表单（`views/settings/sections/*`）。遗漏会导致前端类型不匹配或表单丢失字段。
- `GlobalConfig::default()` 是前端 `DEFAULT_CONFIG` 的权威来源（注释注明 Task 3 已核对），改动默认值时需同步前端。
- 配置文件路径的解析逻辑集中在 ConfigManager（`config_path()` / `global_config_path()` / `anchors_dir()`），改动目录策略只改此处。
