# 11 · infrastructure/checker —— 健康检查域

> 文件：`src-tauri/src/infrastructure/checker/{checks,runner,report}.rs`

## 1. 职责

可插拔的健康检查框架：环境自检（FFmpeg / ffprobe / 磁盘空间 / 写入权限等），供**首启向导**（`run_wizard_health_check`）、**设置页「运行健康检查」**（`run_health_check`）与**调试页诊断导出**（`export_diagnostic_report`）共用。

## 2. report.rs —— 报告模型

```rust
pub enum CheckStatus { Passed, Failed, Warning, Skipped }

pub struct CheckResult {
    check_name: String, status: CheckStatus,
    message: String, details: Option<String>,
    suggestion: Option<String>, duration_ms: u64,
}

pub struct DiagnosticReport {
    results: Vec<CheckResult>, total, passed, failed, warnings,
    timestamp: String,   // RFC3339
}
```

## 3. checks.rs —— 检查项

```rust
#[async_trait]
pub trait HealthCheck: Send + Sync {
    fn name(&self) -> &'static str;
    async fn run(&self) -> CheckResult;
}
```

### 现有检查项

| 检查项 | 判定 | 说明 |
| --- | --- | --- |
| `FfmpegCheck` | 候选可执行 | `ffmpeg_candidates` 候选 + `clean_path` 清洗（与引擎/向导口径一致） |
| `ffprobe` 检查 | 同上 | 辅助工具探测 |
| `DiskSpaceCheck` | 剩余空间 ≥ 阈值 | 与 `recorder/disk.rs` 阈值语义一致：**0 = 不限制** |
| 写入权限检查 | 试写临时文件 | 输出目录可写性 |

`clean_path`：清洗控制字符与双向文本符（U+202A–U+202E、U+2066–U+2069），防路径注入展示。

## 4. runner.rs —— CheckRunner

- `register(check)`：注册检查项（组合模式）；
- `run_all()`：顺序执行全部 → `DiagnosticReport`；
- `run_named(name)`：执行指定项（向导「仅 FFmpeg」场景）。

## 5. 跨模块依赖

| 依赖 | 用途 |
| --- | --- |
| `domain/tools` | `ffmpeg_candidates` |
| `api/debug_cmds.rs` / `api/wizard_cmds.rs` | 命令入口 |
| `domain/config` | 输出目录 / 磁盘阈值 / ffmpeg_path |

## 6. 测试

- `clean_path`（控制字符 / 双向文本符清洗）；
- 候选清洗与候选顺序。

## 7. 已知陷阱

- **检查口径必须与引擎一致**：FFmpeg 候选顺序、磁盘阈值语义（0=不限制）与 `engine.rs` / `disk.rs` 同步维护，否则出现「健康检查通过但录制失败」或相反。
- `CheckRunner` 是同步顺序执行（async trait），新增检查项注意总耗时（向导环境检查的等待时间）。
- 检查结果 `message` 是用户可读文案（前端直接展示），写检查项时文案要面向用户而非开发者。
