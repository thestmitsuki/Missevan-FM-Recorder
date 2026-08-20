# 06 · domain/spider —— 爬虫域

> 文件：`src-tauri/src/domain/spider.rs`

## 1. 职责

猫耳 FM（missevan.com）API 客户端：直播状态检测、主播公开资料、流地址获取、房间号提取、检测失败分类；同时承担**网络请求插桩**（写入 `infrastructure::logging::network` 的全局环形缓冲）。

## 2. 核心类型

```rust
pub struct LiveCheckResult {
    pub is_live: bool,
    pub anchor_name: Option<String>,
    pub title: Option<String>,
    pub stream_url: Option<String>,
    pub avatar: Option<String>,
}

pub enum CheckErrorKind { Server, Network, Format, Other }   // 检测失败分类
```

### 检测失败分类（检测循环据此决定重试与状态判定）

| 分类 | 触发条件 | 处理策略 | 状态判定 |
| --- | --- | --- | --- |
| `Server` | HTTP 5XX / 429 | 重试 + 指数退避；429 额外冷却 | 不判离线（未知） |
| `Network` | 连接失败 / 超时 / DNS | 重试 + 指数退避 | 不判离线（未知） |
| `Format` | JSON 解析失败 / 结构不符（API 格式变化） | 记 warn，不重试 | 不判离线（未知） |
| `Other` | 其他（4XX 房间不存在等） | 不重试 | 依调用方语义 |

## 3. 公开方法（节选）

| 方法 | 说明 |
| --- | --- |
| `new(proxy, cookie?)` / `with_auth` | 代理（http/socks5 + 认证）与每主播 Cookie |
| `check_live(room_id) -> Result<LiveCheckResult, CheckError>` | 检测直播状态 + 流地址 |
| `get_anchor_profile(room_id) -> AnchorProfile` | 主播简介/头像 |
| `get_stream_url(room_id)` | 流地址获取（录制启动前） |
| `extract_room_id(url) -> Option<String>` | `https://fm.missevan.com/live/<数字>` 提取；路径只允许一段（`/live/123/456` 拒绝，防前后端取段错配） |

## 4. 网络插桩（Task 15）

- **方案**：不引入 reqwest middleware（保持依赖面可控），在 `check_live` / `get_anchor_profile` **调用点**记录（URL / 方法 / 状态码 / 耗时 / room_id）→ `infrastructure::logging::network::global_store`（进程级全局单例，容量 500）。
- 客户端在多处创建（detector / anchor_cmds / 录制 monitor），因此 store 用全局单例而非传参。
- 写入前脱敏：URL 与错误信息中的 Cookie / Authorization / Password 值 → `***`（复用 `sanitize_message`）。

## 5. 工具函数

- `truncate_bytes(s, max)`：按字节上限截断且**保证字符边界**（不产生 U+FFFD 替换符）——用于日志/通知文案长度控制。
- 代理 URL 脱敏（`redact_proxy_url`）：密码不打印（debug 信息输出时）。

## 6. 跨模块依赖

| 依赖 | 用途 |
| --- | --- |
| `domain/config` | 代理 / cookie 配置 |
| `infrastructure/logging/network` | 请求插桩 |
| `infrastructure/error` | AppError（调用方包装） |
| 消费者 | `detector/loop.rs`（检测）、`anchor_cmds.rs`（profile/刷新）、`recorder/engine.rs`（流地址）、`update_cmds.rs`（构建客户端） |

## 7. 测试

- 房间号提取（合法 URL / 多段路径拒绝 / 查询串锚点放行）；
- `truncate_bytes` 字符边界（emoji 不切碎）；
- 代理 URL 脱敏（含密码 URL 打码、无密码原样）。

## 8. 已知陷阱

- **API 格式变化**归类为 `Format` 且不重试、不判离线——平台接口变更时表现是「一直未知」，排查先看日志 warn。
- 429 冷却由 detector 的 RateLimiter 管理（spider 只返回分类），改动冷却逻辑在 `detector/loop.rs`。
- 插桩点改动（新增 API 调用）记得同步写 `network.rs` 的调用点记录，否则调试页「网络请求」看不到。
- 流地址获取失败（`REC_API_FAILED`）会跳过本轮录制但不判离线——与「直播中」展示解耦，避免风控期误停。
