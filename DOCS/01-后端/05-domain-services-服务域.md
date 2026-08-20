# 05 · domain/services —— 服务域

> 文件：`src-tauri/src/domain/services/{file_cache,cleanup,fs_walk}.rs`（`mod.rs` 导出）

## 1. 职责

面向「录制文件」的横切服务：输出目录扫描与缓存、按策略清理、安全目录遍历。

## 2. file_cache.rs —— 录制文件缓存

### 核心结构

```rust
pub struct FileCacheManager {
    state: FileCacheState,     // 文件表 + 文件夹树 + 扫描日志
}
pub type FileCacheHandle = Arc<Mutex<FileCacheManager>>;   // Tauri 托管类型
```

`FileCacheState`（`get_file_cache_state` 返回，调试页展示）：

- `files: Vec<RecordingFile>`（id / name / path / size / duration / anchor_name / created_at(SystemTime) / is_active）
- `folders: Vec<FileFolder>`（按主播名分组 → 日期分组树：今天/昨天/本周/本月/YYYY-MM）
- `scan_log: Vec<ScanLogEntry>`（上限 `SCAN_LOG_LIMIT`=20，供调试页排查）

### 关键行为

- **扫描**：`scan_output_dir` 用 `fs_walk` 安全遍历（不跟随链接/junction，S1 修复）；输出目录不存在/不可读 → 记录扫描日志，不崩溃。
- **`path_key()`**：统一 `/` 分隔符——引擎 output_path 用 `/` 拼接、Path 产出 `\`，直接比较会漏判（Windows）。
- **`mark_active`**：对照 `AppState.active_output_paths()` 标记 `is_active`（前端据此禁删/禁改）。
- **事件**：`refresh_recording_files` 扫描后 emit `recording_files_changed`（`RecordingFilesPayload`：files + folders）；缓存为空差异时发全量。
- **构建文件夹树**：`build_folder_tree`（anchor_name 剥离 `-房间号` → 日期分组）。

### 跨模块

- 生产者：录制引擎（录制开始/结束触发刷新）、`file_cmds`、`wizard finish_wizard`。
- 消费者：文件页（`get_recording_files`）、清理服务（`scan_recording_files` 同源扫描）、调试页。

## 3. cleanup.rs —— 录制文件清理

### 策略（规格「文件分类」）

1. 先按 mtime 删除 `retention_days` 天前的旧文件（`retention_days == 0` = 不按天清理）；
2. 若总量仍超 `max_total_gb`（`0` = 不限制），按**最旧优先**继续删除直到达标或清空；
3. **跳过活跃文件**（正在录制的输出路径，防删正在写的文件）。

### 触发方式

- `run_cleanup_now` 命令（设置页「立即清理」按钮）；
- 每次录制任务结束（`monitor.rs` 的 `cleanup_on_recording_end` 统一出口）；
- 原 `cleanup_time` 每日定时调度已移除（M 审查跟进）。

### 安全保证

- 扫描走 `fs_walk`（不跟随链接）——junction 指向目录外的文件**不会**进入清理候选（S1 数据丢失修复）；
- 删除路径校验在输出目录内（`ensure_within_output_dir` 语义，防前缀欺骗 `out2` 视为 `out` 子路径）。

### 返回

`CleanupSummary`（删除数量 / 释放字节等），前端展示清理结果。

## 4. fs_walk.rs —— 安全目录遍历

S1 修复的收敛点：清理与文件缓存原本各自实现递归遍历（可能跟随目录链接 → 越权列出/删除目录外文件）。现统一：

```rust
pub fn collect_files(root: &Path) -> io::Result<Vec<PathBuf>>
```

- 条目类型一律以 `symlink_metadata` 判定（**不跟随**链接）；
- 符号链接 / Windows junction / 目录符号链接：既不入目录、也不作为文件产出；
- 深度优先，顺序不定。

单测覆盖：普通目录递归、junction 指向目录外文件不入结果。

## 5. 已知陷阱

- **新加「遍历输出目录」的逻辑必须复用 `fs_walk::collect_files`**，禁止再手写递归（S1 红线）。
- `path_key` 归一化只处理 `\`→`/`；大小写不归一（Windows 文件系统大小写不敏感但比较是敏感的，碰撞场景：引擎写 `A.m4a`、扫描读 `a.m4a`——当前语义按字符串比较，改动需谨慎）。
- `FileCacheState` 的内存副本在多处（get / refresh / 调试页）互相同步靠 Tauri 托管 Mutex；`get_recording_files` 与 `refresh_recording_files` 的锁序（先 active_paths 再 cache）是死锁修复，勿调换。
- 清理策略变更（如新增「按大小删除最早文件」）会直接影响用户数据，必须同步更新单测与文档。
