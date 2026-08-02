use std::sync::{Arc, Mutex};

use tauri::AppHandle;
use tracing::{Event, Subscriber};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::fmt;

use super::buffer::{LogBuffer, LogLayer, sanitize_message};

/// 日志守卫——持有 non_blocking 写入器的 Guard，防止日志丢失
pub struct LogGuard {
    _guard: WorkerGuard,
}

/// 初始化结构化日志系统。
///
/// - 控制台输出：`RUST_LOG` 环境变量控制级别，默认 `info`
/// - 文件输出：`{app_data_dir}/logs/missevan-recorder.log`，按日轮转（需外部清理旧日志）
/// - 内存环形缓冲（Task 15）：捕获全部日志事件写入 `LogBuffer`（容量 1000）
///   并 emit `debug:log`（节流 100 条/秒）；app_handle 由 setup() 注入后事件生效
///
/// 脱敏（规格 5.2 / Task 18）：控制台与文件层的事件消息在写出前统一经
/// `sanitize_message` 过滤（Cookie / Authorization / Password 值 → `***`），
/// 与调试缓冲层（Task 15）同规则——所有日志出口共用同一脱敏函数。
///
/// # Panics
/// 如果日志系统初始化失败（例如无法创建日志目录），会 panic。
pub fn init_logging(
    app_data_dir: &std::path::Path,
) -> (LogGuard, Arc<LogBuffer>, Arc<Mutex<Option<AppHandle>>>) {
    let log_dir = app_data_dir.join("logs");
    std::fs::create_dir_all(&log_dir).expect("Failed to create log directory");

    let file_appender = tracing_appender::rolling::daily(&log_dir, "missevan-recorder.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // 构建多层订阅者：控制台（脱敏文本）+ 文件（脱敏 JSON）+ 调试缓冲（内存环形缓冲 + 事件）
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // Stdout 无直接 MakeWriter 实现：用零捕获闭包（FnMut() -> Stdout）包装
    let console_layer = SanitizedFmtLayer::new(|| std::io::stdout(), false)
        .with_filter(filter.clone());

    let file_layer = SanitizedFmtLayer::new(non_blocking, true);

    // 调试环形缓冲层：与 console 同级别过滤（RUST_LOG 一致性）
    let log_buffer = Arc::new(LogBuffer::new(1000));
    let app_handle_slot: Arc<Mutex<Option<AppHandle>>> = Arc::new(Mutex::new(None));
    let debug_layer = LogLayer::new(log_buffer.clone(), app_handle_slot.clone())
        .with_filter(filter);

    tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .with(debug_layer)
        .init();

    (LogGuard { _guard: guard }, log_buffer, app_handle_slot)
}

// ── 脱敏格式化层（规格 5.2：文件日志与实时日志同规则脱敏） ──────────────────

/// 提取事件全部字段（name, Debug 文本）的 visitor
struct FieldsVisitor<'a>(&'a mut Vec<(String, String)>);

impl<'a> tracing::field::Visit for FieldsVisitor<'a> {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.push((field.name().to_string(), value.to_string()));
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push((field.name().to_string(), format!("{:?}", value)));
    }
}

/// 剥掉 Debug 格式化的外层引号（`"msg"` → `msg`；JSON 层输出与 fmt::json 一致）
fn unquote(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .unwrap_or(s)
}

/// 消息字段脱敏（Debug 文本 → 去引号 → sanitize_message）
fn sanitize_field_value(v: &str) -> String {
    sanitize_message(unquote(v))
}

/// 纯渲染函数：组装单行日志输出（消息已脱敏；测试直接调用）
fn render_line(
    json: bool,
    timestamp: &str,
    level: &str,
    target: &str,
    thread: &str,
    message: &str,
) -> String {
    if json {
        let mut obj = serde_json::Map::new();
        obj.insert("timestamp".into(), serde_json::Value::String(timestamp.into()));
        obj.insert("level".into(), serde_json::Value::String(level.into()));
        obj.insert("target".into(), serde_json::Value::String(target.into()));
        obj.insert("thread".into(), serde_json::Value::String(thread.into()));
        obj.insert("message".into(), serde_json::Value::String(message.into()));
        serde_json::Value::Object(obj).to_string()
    } else {
        format!("{} {} {}: {}", timestamp, level, target, message)
    }
}

/// 脱敏格式化层：提取事件字段 → message 脱敏 → JSON（文件）或纯文本（控制台）写出。
/// 与调试缓冲层共用 `sanitize_message`，保证文件日志与实时日志脱敏规则一致。
struct SanitizedFmtLayer<W> {
    writer: W,
    json: bool,
}

impl<W> SanitizedFmtLayer<W> {
    fn new(writer: W, json: bool) -> Self {
        Self { writer, json }
    }
}

impl<S, W> Layer<S> for SanitizedFmtLayer<W>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: for<'a> fmt::MakeWriter<'a> + Send + Sync + 'static,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut fields: Vec<(String, String)> = Vec::new();
        let mut visitor = FieldsVisitor(&mut fields);
        event.record(&mut visitor);

        let message = fields
            .iter()
            .find(|(k, _)| k == "message")
            .map(|(_, v)| sanitize_field_value(v))
            .unwrap_or_default();

        let line = render_line(
            self.json,
            &chrono::Utc::now().to_rfc3339(),
            level_str(event.metadata().level()),
            event.metadata().target(),
            std::thread::current().name().unwrap_or("main"),
            &message,
        );

        let mut w = self.writer.make_writer();
        use std::io::Write;
        let _ = writeln!(w, "{}", line);
    }
}

fn level_str(level: &tracing::Level) -> &'static str {
    match *level {
        tracing::Level::TRACE => "TRACE",
        tracing::Level::DEBUG => "DEBUG",
        tracing::Level::INFO => "INFO",
        tracing::Level::WARN => "WARN",
        tracing::Level::ERROR => "ERROR",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_json_line_sanitizes_cookie_in_message() {
        // 真实调用链：on_event 先 sanitize_field_value 再 render_line
        let msg = sanitize_field_value("请求失败 Cookie: a1b2c3");
        let line = render_line(
            true,
            "2026-08-01T00:00:00Z",
            "INFO",
            "domain::spider",
            "main",
            &msg,
        );
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["level"], "INFO");
        assert_eq!(v["target"], "domain::spider");
        assert_eq!(v["thread"], "main");
        assert_eq!(v["message"], "请求失败 Cookie: ***");
    }

    #[test]
    fn render_text_line_sanitizes_password_value() {
        let msg = sanitize_field_value("proxy_password=super-secret, ok=true");
        let line = render_line(
            false,
            "2026-08-01T00:00:00Z",
            "WARN",
            "config_manager",
            "main",
            &msg,
        );
        assert!(line.contains("proxy_password=***"), "{}", line);
        assert!(!line.contains("super-secret"), "{}", line);
    }

    #[test]
    fn render_text_line_noop_without_sensitive_keys() {
        let msg = "检测完成: 主播 测试主播 已开播";
        let line = render_line(false, "t", "INFO", "detector", "main", msg);
        assert!(line.ends_with(&format!(": {}", msg)));
    }

    #[test]
    fn sanitize_field_value_unquotes_and_sanitizes() {
        // Debug 格式化的字符串带引号：先剥引号再脱敏
        assert_eq!(sanitize_field_value(r#""Cookie: abc=def""#), "Cookie: ***");
        assert_eq!(sanitize_field_value("Cookie: abc=def"), "Cookie: ***");
        assert_eq!(sanitize_field_value(r#""普通消息""#), "普通消息");
    }

    #[test]
    fn render_json_line_is_valid_json() {
        let line = render_line(true, "t", "INFO", "m", "main", "hi");
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["message"], "hi");
    }
}
