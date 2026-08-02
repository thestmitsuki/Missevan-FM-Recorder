//! 调试日志环形缓冲（Task 15）
//!
//! - `LogBuffer`：容量 1000 的内存环形缓冲（超限丢最旧），供调试页「实时日志」模块查询
//! - `LogLayer`：tracing Layer，捕获日志事件 → 脱敏 → 写入缓冲并 emit `debug:log`
//!   （节流：最大 100 条/秒，防前端刷屏）
//! - `sanitize_message`：写入缓冲前脱敏（Cookie / Authorization / Password 值 → `***`）
//!
//! 事件契约（`debug:log`）：`LogEntry { timestamp, level, module, message }`，
//! `timestamp` 为 RFC3339，`level` 为小写字符串（trace/debug/info/warn/error），
//! `module` 为 tracing target（模块路径，前端「来源过滤」按子串匹配）。

use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

/// 单条日志（`debug:log` 事件载荷）
#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    /// RFC3339 时间戳
    pub timestamp: String,
    /// trace | debug | info | warn | error
    pub level: String,
    /// tracing target（模块路径，如 `domain::detector::loop`）
    pub module: String,
    /// 日志消息（已脱敏）
    pub message: String,
}

/// 日志环形缓冲（容量 1000，超限丢最旧）
pub struct LogBuffer {
    inner: Arc<RwLock<VecDeque<LogEntry>>>,
    capacity: usize,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(VecDeque::with_capacity(capacity))),
            capacity,
        }
    }

    /// 追加一条日志（超限丢最旧）
    pub fn push(&self, entry: LogEntry) {
        let mut buf = self.inner.write().unwrap_or_else(|p| p.into_inner());
        if buf.len() >= self.capacity {
            buf.pop_front();
        }
        buf.push_back(entry);
    }

    /// 全部日志（最新在前）
    pub fn all(&self) -> Vec<LogEntry> {
        self.filter(None, None)
    }

    /// 按级别 / 来源过滤（最新在前）。
    /// - `level`：精确匹配（"info" / "error" / ...）
    /// - `source`：module 子串匹配（如 "detector" 命中 `domain::detector::loop`）
    pub fn filter(&self, level: Option<&str>, source: Option<&str>) -> Vec<LogEntry> {
        self.inner
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .rev()
            .filter(|e| level.map(|l| e.level == l).unwrap_or(true))
            .filter(|e| source.map(|s| e.module.contains(s)).unwrap_or(true))
            .cloned()
            .collect()
    }

    pub fn clear(&self) {
        self.inner
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }

    pub fn len(&self) -> usize {
        self.inner
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// `debug:log` 事件节流上限（条/秒）
const MAX_EVENTS_PER_SEC: u32 = 100;

/// 节流状态：每秒一个窗口，窗口内最多 100 条
struct ThrottleState {
    sec_start: Instant,
    count: u32,
}

/// tracing Layer：捕获日志事件 → 脱敏 → 环形缓冲 + `debug:log` 事件（节流）
pub struct LogLayer {
    buffer: Arc<LogBuffer>,
    app_handle: Arc<Mutex<Option<AppHandle>>>,
    throttle: Mutex<ThrottleState>,
}

impl LogLayer {
    pub fn new(buffer: Arc<LogBuffer>, app_handle: Arc<Mutex<Option<AppHandle>>>) -> Self {
        Self {
            buffer,
            app_handle,
            throttle: Mutex::new(ThrottleState {
                sec_start: Instant::now(),
                count: 0,
            }),
        }
    }
}

impl<S> Layer<S> for LogLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        // 1. 提取 message 字段（无消息的事件跳过，如纯字段事件）
        let mut msg = String::new();
        {
            let mut visitor = MessageVisitor(&mut msg);
            event.record(&mut visitor);
        }
        if msg.is_empty() {
            return;
        }
        let message = sanitize_message(&msg);

        // 2. 节流：超过 100 条/秒丢弃（只影响 debug 缓冲与事件，不影响其他层）
        {
            let mut th = self.throttle.lock().unwrap_or_else(|p| p.into_inner());
            let now = Instant::now();
            if now.duration_since(th.sec_start) >= Duration::from_secs(1) {
                th.sec_start = now;
                th.count = 0;
            }
            if th.count >= MAX_EVENTS_PER_SEC {
                return;
            }
            th.count += 1;
        }

        // 3. 写入环形缓冲 + emit `debug:log`
        let entry = LogEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            level: level_str(event.metadata().level()).to_string(),
            module: event.metadata().target().to_string(),
            message,
        };
        self.buffer.push(entry.clone());

        if let Some(handle) = self.app_handle.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            let _ = handle.emit("debug:log", &entry);
        }
    }
}

/// 提取事件 message 字段的 visitor（与 tracing-subscriber fmt 层同款逻辑：
/// message 值 Debug 格式化即消息文本本身）
struct MessageVisitor<'a>(&'a mut String);

impl<'a> tracing::field::Visit for MessageVisitor<'a> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            use std::fmt::Write;
            let _ = write!(self.0, "{:?}", value);
        }
    }
}

fn level_str(level: &tracing::Level) -> &'static str {
    match *level {
        tracing::Level::TRACE => "trace",
        tracing::Level::DEBUG => "debug",
        tracing::Level::INFO => "info",
        tracing::Level::WARN => "warn",
        tracing::Level::ERROR => "error",
    }
}

// ── 脱敏 ──────────────────────────────────────────────────────────────

/// 敏感键后的值是否应吞到行尾（Authorization 的 `Bearer xyz` 整段都是凭据）
fn marker_eats_to_line_end(marker: &str) -> bool {
    marker == "authorization"
}

fn is_value_delimiter(b: u8) -> bool {
    matches!(b, b';' | b',' | b'&' | b' ' | b'\t' | b'\r' | b'\n')
}

/// 消息脱敏：`Cookie` / `Authorization` / `Password`（含 `proxy_password`）键的
/// 值替换为 `***`。纯函数，写入日志缓冲与网络记录前调用。
///
/// - `Cookie: abc=def` → `Cookie: ***`
/// - `Authorization: Bearer xyz` → `Authorization: ***`（整段吞掉）
/// - `proxy_password=secret` → `proxy_password=***`
/// - 无敏感键的消息原样返回（快速路径）
pub fn sanitize_message(msg: &str) -> String {
    const MARKERS: [&str; 3] = ["cookie", "authorization", "password"];

    let bytes = msg.as_bytes();
    let mut out = String::with_capacity(msg.len());
    let mut i = 0;
    while i < bytes.len() {
        let matched = MARKERS.iter().find_map(|m| {
            let ml = m.len();
            if i + ml <= bytes.len() && bytes[i..i + ml].eq_ignore_ascii_case(m.as_bytes()) {
                Some(*m)
            } else {
                None
            }
        });

        if let Some(marker) = matched {
            // 键与分隔符之间允许空格
            let mut sep = i + marker.len();
            while sep < bytes.len() && (bytes[sep] == b' ' || bytes[sep] == b'\t') {
                sep += 1;
            }
            if sep < bytes.len() && (bytes[sep] == b':' || bytes[sep] == b'=') {
                // 复制「键 + 分隔符」
                out.push_str(&msg[i..=sep]);
                // 跳过值前导空白，确定值起点
                let mut j = sep + 1;
                while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                    j += 1;
                }
                let value_start = j;
                let stop = if marker_eats_to_line_end(marker) {
                    while j < bytes.len() && !matches!(bytes[j], b';' | b',' | b'&' | b'\r' | b'\n')
                    {
                        j += 1;
                    }
                    j
                } else {
                    while j < bytes.len() && !is_value_delimiter(bytes[j]) {
                        j += 1;
                    }
                    j
                };
                if stop > value_start {
                    // 有值才脱敏：整段（前导空白 + 值）替换为 `***`；
                    // `=` 后直接接 `***`，`:` 后补一个空格保持可读性
                    if bytes[sep] == b'=' {
                        out.push_str("***");
                    } else {
                        out.push_str(" ***");
                    }
                    i = stop;
                } else {
                    // 无值：原样保留（键 + 分隔符已复制，此处补上前导空白）
                    out.push_str(&msg[sep + 1..j]);
                    i = j;
                }
                continue;
            }
        }

        // 未匹配：按 UTF-8 字符边界复制一个字符
        let ch_len = utf8_char_len(bytes[i]);
        out.push_str(&msg[i..i + ch_len]);
        i += ch_len;
    }
    out
}

/// 首字节推算 UTF-8 字符长度（ASCII 1 / 2 字节 2 / 3 字节 3 / 4 字节 4）
fn utf8_char_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(level: &str, module: &str, message: &str) -> LogEntry {
        LogEntry {
            timestamp: "2026-08-01T00:00:00Z".to_string(),
            level: level.to_string(),
            module: module.to_string(),
            message: message.to_string(),
        }
    }

    #[test]
    fn buffer_capacity_drops_oldest() {
        let buf = LogBuffer::new(3);
        buf.push(entry("info", "a", "1"));
        buf.push(entry("info", "b", "2"));
        buf.push(entry("info", "c", "3"));
        buf.push(entry("info", "d", "4"));
        let all = buf.all();
        assert_eq!(all.len(), 3);
        // 最新在前
        assert_eq!(all[0].message, "4");
        assert_eq!(all[2].message, "2");
    }

    #[test]
    fn buffer_filter_by_level() {
        let buf = LogBuffer::new(10);
        buf.push(entry("info", "m", "a"));
        buf.push(entry("error", "m", "b"));
        buf.push(entry("warn", "m", "c"));
        let errors = buf.filter(Some("error"), None);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "b");
    }

    #[test]
    fn buffer_filter_by_source_prefix() {
        let buf = LogBuffer::new(10);
        buf.push(entry("info", "domain::detector::loop", "d"));
        buf.push(entry("info", "domain::spider", "s"));
        buf.push(entry("info", "infrastructure::state::mock_store", "m"));
        let hits = buf.filter(None, Some("detector"));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message, "d");
        // 同时按级别 + 来源过滤
        let combined = buf.filter(Some("info"), Some("spider"));
        assert_eq!(combined.len(), 1);
        assert_eq!(combined[0].message, "s");
    }

    #[test]
    fn buffer_clear() {
        let buf = LogBuffer::new(10);
        buf.push(entry("info", "m", "a"));
        buf.clear();
        assert!(buf.is_empty());
    }

    #[test]
    fn sanitize_cookie_header() {
        assert_eq!(
            sanitize_message("Cookie: abc=def; ghi=jkl"),
            "Cookie: ***; ghi=jkl"
        );
        assert_eq!(sanitize_message("cookie=secret123"), "cookie=***");
        // 大小写不敏感
        assert_eq!(sanitize_message("cOoKiE: abc"), "cOoKiE: ***");
    }

    #[test]
    fn sanitize_authorization_bearer() {
        assert_eq!(
            sanitize_message("Authorization: Bearer eyJhbGciOiJIUzI1NiJ9"),
            "Authorization: ***"
        );
    }

    #[test]
    fn sanitize_password_key() {
        assert_eq!(
            sanitize_message("proxy_password=super-secret, ok=true"),
            "proxy_password=***, ok=true"
        );
        assert_eq!(sanitize_message("password: hunter2"), "password: ***");
    }

    #[test]
    fn sanitize_noop_without_sensitive_keys() {
        let msg = "检测完成: 主播 测试主播 已开播";
        assert_eq!(sanitize_message(msg), msg);
    }

    #[test]
    fn sanitize_handles_utf8_content_safely() {
        // 中文 + 敏感键混排，不 panic、不破坏 UTF-8
        let msg = "请求失败 Cookie: a1b2c3 主播：测试主播";
        let out = sanitize_message(msg);
        assert_eq!(out, "请求失败 Cookie: *** 主播：测试主播");
        assert!(out.is_char_boundary(out.len()));
    }

    #[test]
    fn sanitize_empty_value_keeps_text() {
        // 无值（冒号后直接结尾）不替换
        assert_eq!(sanitize_message("Cookie:"), "Cookie:");
        assert_eq!(sanitize_message("Cookie: "), "Cookie: ");
    }
}
