# 13 · infrastructure/error —— 错误域

> 文件：`src-tauri/src/infrastructure/error/types.rs`

## 1. 职责

统一错误模型：所有命令返回 `Result<_, AppError>`，序列化给前端；错误码常量供前端按码分支处理。

## 2. AppError 结构

```rust
pub struct AppError {
    pub code: String,            // 错误码常量（见下）
    pub category: ErrorCategory, // Config / System / Network / Recording / Internal / Io ...
    pub severity: ErrorSeverity, // Error / Warning / Info
    pub message: String,         // 用户可读
    pub suggestion: Option<String>,
    pub technical: Option<String>, // 技术细节（脱敏后入日志；前端默认不展示）
}
```

### 构造器

`AppError::system(code, msg)` / `::config(...)` / `::network(...)` / `::recording(...)` / `::internal(...)` / `::io(...)`，均支持 `.with_technical()` / `.with_suggestion()` 链式补充。

### From 实现

- `From<std::io::Error>`、`From<serde_json::Error>` → `internal` 包装（命令层 `?` 便捷传播）。

## 3. 错误码常量（节选，按前缀分组）

| 前缀 | 类别 | 示例 |
| --- | --- | --- |
| `CF_` | 配置 | `CF_PARSE_FAIL`（解析失败）/ `CF_INVALID_FIELD`（字段校验失败） |
| `NF_` | 系统/FFmpeg | `NF_FFMPEG_NOT_FOUND` / `NF_FFMPEG_EXEC_FAIL` |
| `NW_` | 网络 | `NW_API_UNREACHABLE` / `NW_API_RESPONSE_ERR` |
| `RC_` | 录制 | `RC_PROCESS_CRASH` / `RC_STREAM_UNAVAILABLE` / `RC_ALREADY_RECORDING`（双录）/ `RC_CONCURRENCY_LIMIT`（并发上限）/ `RC_DISK_LOW`（磁盘不足） |
| `IO_` | IO | `IO_WRITE_FAIL` 等 |
| `INT_` | 内部 | `INT_UNEXPECTED`（不可预期路径） |

## 4. 前端消费约定

- `services/api.ts` 的 `invoke` 包装：失败时抛 `{ code, message, ... }`（Tauri 序列化 AppError）；
- 页面/组件按 `code` 分支（如 `RC_ALREADY_RECORDING` → 提示「已在录制」）；未知码 → 通用错误提示 + 建议查看日志。

## 5. 测试

- 错误码/分类/严重级构造正确性；
- `serde_json::Error` 与 `io::Error` 的 From 转换。

## 6. 已知陷阱

- **新增错误码 = 前端分支同步**：`src/types/*` 或业务代码中按码判断处；遗漏时前端只显示通用文案（功能可用但体验下降）。
- `message` 面向用户（可展示），`technical` 面向日志——不要把堆栈/敏感值放进 message。
- 命令层 catch 一切 `Err` 并转 AppError，**不得 panic**（panic 会进进程级 hook，属于最后防线，不是正常路径）。
