use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use tauri::AppHandle;
use tracing::{Event, Subscriber};
use tracing::subscriber::Interest;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::layer::{Context, Filter, Layer, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::fmt;

use super::buffer::{LogBuffer, LogLayer, sanitize_message};

/// 日志级别白名单（与 `init_logging` 启动级别同规则）：仅接受
/// error/warn/info/debug/trace，非法值回退 `info`。纯函数，供启动与
/// 运行时热更新共用（U5：日志级别热更新）。
fn normalize_log_level(level: &str) -> &'static str {
    match level {
        "error" => "error",
        "warn" => "warn",
        "info" => "info",
        "debug" => "debug",
        "trace" => "trace",
        _ => "info",
    }
}

/// 可热更新过滤器（U5）：包装共享 `Arc<RwLock<EnvFilter>>`，实现
/// `tracing_subscriber::layer::Filter`，供控制台层与调试缓冲层**共享同一实例**
/// （`Clone` 只复制 Arc）——运行中替换锁内 EnvFilter 即同时对两层生效，与
/// 改造前「console 与 debug 同 filter」的行为一致。
///
/// 实现取舍：`tracing_subscriber::reload::Layer` 不实现 `Clone` 且其 `S` 泛型
/// 为匿名层组合类型，无法在 state 中持有多个句柄统一 reload；自实现 Filter
/// 仅 ~40 行，类型明确、两处共享一个锁，语义与官方 reload 等价。
///
/// callsite 缓存：`callsite_enabled` 委托锁内 EnvFilter（按当前级别返回
/// always/never/sometimes）——tracing-core 对每个调用点只注册一次 Interest 并
/// 缓存，因此**热更新时必须调用 `tracing::callsite::rebuild_interest_cache()`**
/// （见 `LogLevelReload::reload`）重建全部缓存，否则旧调用点仍按旧级别过滤。
#[derive(Clone)]
struct ReloadFilter {
    inner: Arc<RwLock<EnvFilter>>,
}

impl<S> Filter<S> for ReloadFilter {
    fn enabled(&self, meta: &tracing::Metadata<'_>, ctx: &Context<'_, S>) -> bool {
        self.inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .enabled(meta, ctx.clone())
    }

    fn callsite_enabled(&self, meta: &'static tracing::Metadata<'static>) -> Interest {
        // EnvFilter::register_callsite 是私有固有方法，经 Filter trait 公有方法
        // 委托（UFCS 消除 S 泛型歧义）——返回基于当前级别的 always/never/
        // sometimes；配合 LogLevelReload::reload 的 rebuild_interest_cache 重建
        // 调用点缓存，热更新对所有已注册调用点生效。
        let guard = self.inner.read().unwrap_or_else(|e| e.into_inner());
        <EnvFilter as Filter<S>>::callsite_enabled(&guard, meta)
    }

    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        self.inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .max_level_hint()
    }
}

/// 日志级别热更新句柄（U5）：持有与 `ReloadFilter` 共享的 `Arc<RwLock<EnvFilter>>`，
/// 由 `init_logging` 创建并随返回值交予 lib.rs 托管为 Tauri state；
/// `save_config` / `import_config` 在配置落盘成功后调用 `reload` 即时生效，
/// 不再需要重启应用。
///
/// 语义约定：
/// - 仅影响启动时带 filter 的层（控制台 + 调试环形缓冲）；文件层本就不过滤
///   （全级别落盘，见 `init_logging`），行为保持不变；
/// - `RUST_LOG` 环境变量仍优先于启动级别；运行中经本句柄 reload 后以配置
///   `log_level` 为准（白名单校验，非法回退 info，与启动规则一致）；
/// - reload 失败（锁中毒等极少数场景）静默降级：记 warn 并返回 false，
///   不影响配置保存路径。
#[derive(Clone)]
pub struct LogLevelReload {
    inner: Arc<RwLock<EnvFilter>>,
}

impl LogLevelReload {
    /// 热更新日志级别。返回是否成功（失败时保持原级别，仅记 warn）。
    pub fn reload(&self, level: &str) -> bool {
        let level = normalize_log_level(level);
        // 写锁作用域最小化：仅覆盖 EnvFilter 替换。绝不能持写锁调用
        // rebuild_interest_cache / tracing::info!——二者会经全局 subscriber
        // 回调本 filter 的 callsite_enabled / enabled（取读锁），同一线程
        // RwLock 非重入 → 必然死锁（对抗式验证实证：save_config 即挂死应用）。
        {
            match self.inner.write() {
                Ok(mut f) => *f = EnvFilter::new(level),
                Err(e) => {
                    tracing::warn!("日志级别热更新失败（保持原级别）: {}", e);
                    return false;
                }
            }
        } // 写锁在此释放

        // 重建 callsite interest 缓存：tracing-core 对每个调用点只注册一次
        // Interest（always/never 永久缓存），不重建则旧调用点仍按旧级别过滤、
        // 热更新不生效（U5 实测）。rebuild 遍历全部调用点并按当前 dispatcher
        // 重新注册——日志级别变更频率极低，开销可忽略（官方文档推荐此 API
        // 用于低频配置变化场景）。
        tracing::callsite::rebuild_interest_cache();
        tracing::info!("日志级别已热更新为 {}", level);
        true
    }
}

/// 日志守卫——持有 non_blocking 写入器的 Guard，防止日志丢失；同时持有周期
/// 日志清理线程的停止信号（R2：drop 时自动通知后台线程退出，测试/进程退出时
/// 不再遗留循环线程）。
pub struct LogGuard {
    _guard: WorkerGuard,
    /// 周期日志清理线程句柄（None = 线程 spawn 失败，已静默降级）
    cleanup: Option<LogCleanupHandle>,
}

impl Drop for LogGuard {
    fn drop(&mut self) {
        // 通知周期清理线程退出（不 join：进程退出场景下无需等待，避免任何挂起
        // 风险；线程收到信号后在 recv_timeout 下一次唤醒即退出，不阻塞退出路径）
        if let Some(c) = self.cleanup.take() {
            let _ = c.stop.send(());
        }
    }
}

/// 周期日志清理句柄（R2）：停止信号 + 线程 JoinHandle。
/// - 生产路径：由 `LogGuard` 持有，drop 时自动发送停止信号（线程随即退出）；
/// - 测试路径：显式 `stop.send(())` + `handle.join()`，测试结束不留后台线程。
pub struct LogCleanupHandle {
    stop: std::sync::mpsc::Sender<()>,
    handle: std::thread::JoinHandle<()>,
}

/// 日志文件前缀（tracing_appender::rolling::daily 的按日轮转文件名）：
/// 轮转文件命名为 `{LOG_FILE_PREFIX}.{YYYY-MM-DD}`（如
/// `missevan-recorder.log.2026-08-19`）；清理逻辑按此前缀匹配。
const LOG_FILE_PREFIX: &str = "missevan-recorder.log";

/// 日志文件保留天数（M5）：只保留最近 7 个自然日（含今天）的按日轮转文件。
/// 做成常量而非配置项：避免扩大改动面；7 天足够覆盖「诊断一周内问题」的常见
/// 需求，且 debug 级单日文件通常 <100MB，7 天总量可控。
const LOG_RETENTION_DAYS: i64 = 7;
/// 运行期间日志清理周期（H4）：每 24 小时执行一次 `clean_old_logs`，保证单次
/// 连续运行超过保留期时旧日志同样被清理（此前仅启动时执行一次，>7 天即失效）。
const LOG_CLEAN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// 判定按日轮转日志文件是否过期（纯逻辑，便于单测；M5）：
/// 保留窗口 = 最近 `retention_days` 个自然日（含今天）；文件日期早于
/// `today - (retention_days - 1)` 即过期。`retention_days >= 1`。
fn is_expired_log(
    file_date: chrono::NaiveDate,
    today: chrono::NaiveDate,
    retention_days: i64,
) -> bool {
    debug_assert!(retention_days >= 1);
    file_date < today - chrono::Duration::days(retention_days - 1)
}

/// 清理过期日志文件（M5/H4；启动初始化与运行期间周期任务共用）。
///
/// 按日轮转文件命名 `{prefix}.{YYYY-MM-DD}`：只删除「文件名 = 前缀 + `.` +
/// 合法日期」且日期早于保留窗口的文件；当前活动文件（无日期后缀）、其他
/// 文件、非法日期后缀一律不动。目录不存在 / 读取失败 → 静默成功（幂等，
/// 不阻断启动）。返回删除的文件数（测试断言用；生产路径忽略）。
fn clean_old_logs(
    log_dir: &std::path::Path,
    prefix: &str,
    retention_days: i64,
) -> usize {
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return 0;
    };
    let today = chrono::Local::now().date_naive();
    let mut removed = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // 匹配 `{prefix}.{YYYY-MM-DD}`；无日期后缀（活动文件）/ 其他文件跳过
        let Some(suffix) = name
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_prefix('.'))
        else {
            continue;
        };
        let Ok(date) = chrono::NaiveDate::parse_from_str(suffix, "%Y-%m-%d") else {
            continue;
        };
        if is_expired_log(date, today, retention_days) && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// 周期日志清理（H4）：后台线程每 `interval` 执行一次 `clean_old_logs`，保证
/// 单次连续运行超过保留期时旧日志仍被清理（30 天运行不再积累 30 个日文件）。
///
/// 实现取舍：用 `std::thread` 而非 tokio 定时任务——`init_logging` 在 tauri
/// runtime 建立前同步调用，无可靠异步上下文；清理是同步小操作（读目录 + 删
/// 过期文件），后台线程每 24h 唤醒一次，占用可忽略。清理失败静默（`clean_old_logs`
/// 内部已容错），不 panic、不阻断任何路径。
///
/// 停止机制（R2）：线程以 `mpsc::Receiver::recv_timeout(interval)` 等待——
/// 收到停止信号（Ok）或发送端全部 drop（Disconnected）即退出；超时则执行一次
/// 清理。返回 `Option<LogCleanupHandle>`：由调用方保管——init_logging 存入
/// LogGuard（drop 时自动发停止信号），测试中显式 stop + join；spawn 失败
/// （资源耗尽）返回 None（启动清理已在 init_logging 执行，静默降级）。
fn spawn_daily_log_cleanup(
    log_dir: std::path::PathBuf,
    prefix: &'static str,
    retention_days: i64,
    interval: std::time::Duration,
) -> Option<LogCleanupHandle> {
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let handle = std::thread::Builder::new()
        .name("log-cleanup".to_string())
        .spawn(move || loop {
            match stop_rx.recv_timeout(interval) {
                // 收到停止信号 / 发送端已全部 drop（调用方退出）→ 线程退出
                Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    let removed = clean_old_logs(&log_dir, prefix, retention_days);
                    if removed > 0 {
                        tracing::debug!(
                            "[日志] 周期清理过期日志文件 {} 个（保留 {} 天）",
                            removed,
                            retention_days
                        );
                    }
                }
            }
        })
        .ok()?;
    Some(LogCleanupHandle {
        stop: stop_tx,
        handle,
    })
}

/// 日志文件写入失败计数（G4/L8 审查跟进）：`CountingWriter` 统计持久化层
/// 写盘失败次数，供调用方查询（可接入调试面板/通知；当前版本以一次性 error
/// 日志 + 本计数器暴露，见 `CountingWriter::handle_write_error`）。
pub type LogWriteErrorCounter = Arc<AtomicU64>;

/// 包装滚动文件 appender 的计数写入器（G4/L8）：透传全部写入；写失败时——
/// 1. 失败计数 +1（`LogWriteErrorCounter` 可查询）；
/// 2. **每个失败阶段只记一次 `tracing::error!`**（`notified` 防刷屏：磁盘满
///    时每个日志事件都失败，若每次都发 error 会无限刷屏——首次失败提醒后
///    同阶段静默计数；某次写入成功后 `notified` 复位，下一失败阶段可再次
///    提醒）。
///
/// 该 error 事件仍走全部 tracing 层：控制台与调试环形缓冲（debug:log）可见；
/// 文件层尝试写这条 error 会再次失败——由本写入器静默计数，不再触发嵌套
/// error，无递归风险（`notified` 已置位）。
struct CountingWriter<W> {
    inner: W,
    errors: LogWriteErrorCounter,
    notified: AtomicBool,
}

impl<W> CountingWriter<W> {
    fn new(inner: W, errors: LogWriteErrorCounter) -> Self {
        Self {
            inner,
            errors,
            notified: AtomicBool::new(false),
        }
    }

    /// 记录一次写失败；返回累计失败次数（供 error 文案展示）
    fn note_failure(&self) -> u64 {
        self.errors.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// 是否应发出 error 提醒：每个失败阶段只记一次（成功写入后复位）
    fn should_notify(&self) -> bool {
        !self.notified.swap(true, Ordering::Relaxed)
    }

    /// 写入成功 → 复位提醒标记（下一失败阶段可再次提醒）
    fn mark_success(&self) {
        self.notified.store(false, Ordering::Relaxed);
    }

    /// 写失败统一处理：计数 + 阶段内一次性 error
    fn handle_write_error(&self) {
        let count = self.note_failure();
        if self.should_notify() {
            tracing::error!(
                "[日志] 日志文件写入失败（磁盘满或权限不足），错误日志不再落盘（已失败 {} 次）",
                count
            );
        }
    }
}

impl<W: io::Write> io::Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.inner.write(buf) {
            Ok(n) => {
                self.mark_success();
                Ok(n)
            }
            Err(e) => {
                self.handle_write_error();
                Err(e)
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.inner.flush() {
            Ok(()) => {
                self.mark_success();
                Ok(())
            }
            Err(e) => {
                self.handle_write_error();
                Err(e)
            }
        }
    }
}

/// 启动写测试（G4/L8）：验证当前日志文件可写（打开追加 + 确认元数据可读）。
/// 目录创建失败在 init_logging 开头已 expect；此处针对「目录可建但文件不可
/// 写」的启动场景（磁盘满 / 权限 / 只读目录）——失败时 eprintln 兜底 +
/// `tracing::error!`（控制台/调试缓冲可见；文件层写失败由 CountingWriter
/// 静默计数，不刷屏）。返回是否可写。
fn probe_log_file_writable(log_dir: &std::path::Path) -> bool {
    let today = chrono::Local::now().format("%Y-%m-%d");
    let path = log_dir.join(format!("{}.{}", LOG_FILE_PREFIX, today));
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(f) => f.metadata().is_ok(),
        Err(_) => false,
    }
}

/// 初始化结构化日志系统。
///
/// - 控制台输出：`RUST_LOG` 环境变量优先，其次 `level` 参数（来自
///   GlobalConfig.log_level，仅接受 error/warn/info/debug/trace，非法回退 info）
/// - 文件输出：`{app_data_dir}/logs/missevan-recorder.log`，按日轮转；
///   启动时清理 7 天前的轮转文件，并启动后台线程每 24h 周期清理一次
///   （M5/H4，见 `clean_old_logs` / `spawn_daily_log_cleanup`）
/// - 内存环形缓冲（Task 15）：捕获全部日志事件写入 `LogBuffer`（容量 1000）
///   并 emit `debug:log`（节流 100 条/秒）；app_handle 由 setup() 注入后事件生效
///
/// 脱敏（规格 5.2 / Task 18）：控制台与文件层的事件消息在写出前统一经
/// `sanitize_message` 过滤（Cookie / Authorization / Password 值 → `***`），
/// 与调试缓冲层（Task 15）同规则——所有日志出口共用同一脱敏函数。
///
/// `level` 为启动级别：lib.rs run() 读取上次保存的配置传入。运行中经
/// 返回的 `LogLevelReload` 句柄热更新（U5，见 `save_config` / `import_config`
/// 调用点），不再依赖重启。
///
/// # Panics
/// 如果日志系统初始化失败（例如无法创建日志目录），会 panic。
pub fn init_logging(
    app_data_dir: &std::path::Path,
    level: &str,
) -> (
    LogGuard,
    Arc<LogBuffer>,
    Arc<Mutex<Option<AppHandle>>>,
    LogWriteErrorCounter,
    LogLevelReload,
) {
    let log_dir = app_data_dir.join("logs");
    std::fs::create_dir_all(&log_dir).expect("Failed to create log directory");
    // M5 + H4：启动时清理过期日志（保留最近 7 天），并启动后台周期任务每 24h
    // 再清理一次——日志按日轮转但单次运行 >7 天时不再只靠重启清理（30 天运行
    // 不再积累 30 个日文件，debug 级可达 ~3GB）。清理只删符合
    // `{prefix}.{YYYY-MM-DD}` 命名且早于保留窗口的文件，失败静默跳过
    //（不阻断启动/运行）。周期任务的间隔注入（测试用短间隔验证触发，生产值
    // 见 LOG_CLEAN_INTERVAL）。
    clean_old_logs(&log_dir, LOG_FILE_PREFIX, LOG_RETENTION_DAYS);
    let log_cleanup = spawn_daily_log_cleanup(
        log_dir.clone(),
        LOG_FILE_PREFIX,
        LOG_RETENTION_DAYS,
        LOG_CLEAN_INTERVAL,
    );

    // G4/L8：文件层写入失败感知——用计数写入器包装滚动 appender（写失败
    // 计数 + 每失败阶段一次性 error，见 `CountingWriter`）。计数器随返回值
    // 暴露给调用方（lib.rs 持有，供未来接入调试面板/通知）。
    let log_write_errors: LogWriteErrorCounter = Arc::new(AtomicU64::new(0));
    let file_appender = CountingWriter::new(
        tracing_appender::rolling::daily(&log_dir, LOG_FILE_PREFIX),
        log_write_errors.clone(),
    );
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // 构建多层订阅者：控制台（脱敏文本）+ 文件（脱敏 JSON）+ 调试缓冲（内存环形缓冲 + 事件）
    // 级别来源：RUST_LOG（开发/诊断）> 配置 log_level（白名单校验）> info
    // U5：console 与 debug 层共享同一 ReloadFilter（内部 Arc<RwLock<EnvFilter>>），
    // 运行中经返回的 LogLevelReload 句柄替换锁内 filter 即对两层同时生效；
    // 文件层不挂 filter（全级别落盘），与改造前行为一致。
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(normalize_log_level(level)));
    let shared_filter = Arc::new(RwLock::new(env_filter));

    // Stdout 无直接 MakeWriter 实现：用零捕获闭包（FnMut() -> Stdout）包装
    let console_layer = SanitizedFmtLayer::new(|| std::io::stdout(), false)
        .with_filter(ReloadFilter {
            inner: shared_filter.clone(),
        });

    let file_layer = SanitizedFmtLayer::new(non_blocking, true);

    // 调试环形缓冲层：与 console 同级别过滤（RUST_LOG 一致性）
    let log_buffer = Arc::new(LogBuffer::new(1000));
    let app_handle_slot: Arc<Mutex<Option<AppHandle>>> = Arc::new(Mutex::new(None));
    let debug_layer = LogLayer::new(log_buffer.clone(), app_handle_slot.clone())
        .with_filter(ReloadFilter {
            inner: shared_filter.clone(),
        });

    tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .with(debug_layer)
        .init();

    // G4/L8：启动写测试——日志目录可创建但文件不可写（磁盘满/权限）时
    // 启动即提示一次，避免整个运行期静默丢日志。eprintln 兜底（不依赖
    // tracing 链路，控制台可见）；tracing::error! 经控制台/调试缓冲可见，
    // 文件层写失败由 CountingWriter 静默计数不刷屏。
    if !probe_log_file_writable(&log_dir) {
        eprintln!(
            "[日志] 警告：日志文件不可写（磁盘满或权限不足），本次运行日志不会落盘"
        );
        tracing::error!("[日志] 日志文件不可写（磁盘满或权限不足），本次运行日志不会落盘");
    }

    (
        LogGuard {
            _guard: guard,
            cleanup: log_cleanup,
        },
        log_buffer,
        app_handle_slot,
        log_write_errors,
        LogLevelReload {
            inner: shared_filter,
        },
    )
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

    // ── M5：日志保留策略（只删过期轮转文件）──

    #[test]
    fn expired_log_judgement_keeps_recent_week_only() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 8, 19).unwrap();
        // 保留窗口 = 最近 7 个自然日（含今天）
        for days_ago in 0..=6i64 {
            assert!(
                !is_expired_log(today - chrono::Duration::days(days_ago), today, 7),
                "{} 天前应保留",
                days_ago
            );
        }
        // 第 7 天起过期
        assert!(is_expired_log(today - chrono::Duration::days(7), today, 7));
        assert!(is_expired_log(today - chrono::Duration::days(30), today, 7));
        // 保留 1 天：仅今天保留，昨天即过期
        assert!(!is_expired_log(today, today, 1));
        assert!(is_expired_log(today - chrono::Duration::days(1), today, 1));
    }

    #[test]
    fn clean_old_logs_removes_expired_keeps_recent_and_unrelated() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "missevan-test-logs-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let today = chrono::Local::now().date_naive();
        let dated = |days_ago: i64| {
            dir.join(format!(
                "{}.{}",
                "missevan-recorder.log",
                today - chrono::Duration::days(days_ago)
            ))
        };
        // 过期：8 天前 / 10 天前
        std::fs::write(dated(8), "old").unwrap();
        std::fs::write(dated(10), "old").unwrap();
        // 保留：今天 / 3 天前 / 6 天前
        std::fs::write(dated(0), "cur").unwrap();
        std::fs::write(dated(3), "keep").unwrap();
        std::fs::write(dated(6), "keep").unwrap();
        // 非轮转文件不动：活动文件（无日期后缀）、非法日期后缀、无关文件
        std::fs::write(dir.join("missevan-recorder.log"), "active").unwrap();
        std::fs::write(dir.join("missevan-recorder.log.2026-08-1x"), "bad").unwrap();
        std::fs::write(dir.join("other.txt"), "other").unwrap();

        let removed = clean_old_logs(&dir, "missevan-recorder.log", 7);
        assert_eq!(removed, 2, "只应删除 2 个过期文件");
        assert!(!dated(8).exists() && !dated(10).exists(), "过期文件必须删除");
        assert!(
            dated(0).exists() && dated(3).exists() && dated(6).exists(),
            "保留窗口内文件必须保留"
        );
        assert!(dir.join("missevan-recorder.log").exists(), "活动文件不得删除");
        assert!(
            dir.join("missevan-recorder.log.2026-08-1x").exists(),
            "非法日期后缀不得删除"
        );
        assert!(dir.join("other.txt").exists(), "无关文件不得删除");

        // 目录不存在 → 静默成功（幂等，不 panic）
        assert_eq!(
            clean_old_logs(&dir.join("no-such-dir"), "missevan-recorder.log", 7),
            0
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── H4：日志保留周期化（单次长运行 >7 天仍清理）──

    #[test]
    fn periodic_log_cleanup_fires_and_removes_expired() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "missevan-test-logcycle-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let today = chrono::Local::now().date_naive();
        // 过期（8 天前）与保留窗口内（3 天前）文件
        let expired = dir.join(format!(
            "missevan-recorder.log.{}",
            today - chrono::Duration::days(8)
        ));
        let keep = dir.join(format!(
            "missevan-recorder.log.{}",
            today - chrono::Duration::days(3)
        ));
        std::fs::write(&expired, "old").unwrap();
        std::fs::write(&keep, "keep").unwrap();

        // 短间隔周期任务（生产值 24h，测试用 30ms 验证触发）：启动清理之外，
        // 运行期间周期执行也必须删除过期文件
        let cleanup = spawn_daily_log_cleanup(
            dir.clone(),
            LOG_FILE_PREFIX,
            LOG_RETENTION_DAYS,
            std::time::Duration::from_millis(30),
        )
        .expect("周期清理线程应能启动");
        // 等待至少数个周期（30ms × 10+；删除为微秒级小操作）
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(!expired.exists(), "周期清理应删除过期日志文件");
        assert!(keep.exists(), "保留窗口内文件不得删除");
        // R2：停止后台线程并 join——测试结束后不再遗留循环线程
        let _ = cleanup.stop.send(());
        cleanup
            .handle
            .join()
            .expect("周期清理线程应收到停止信号并正常退出");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── G4/L8：日志写入失败感知（计数 + 阶段内一次性 error）──

    /// 恒失败写入器（模拟磁盘满/权限拒绝）
    struct FailingWriter;

    impl io::Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::Other, "disk full"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::Other, "disk full"))
        }
    }

    #[test]
    fn counting_writer_counts_every_failed_write() {
        let errors = Arc::new(AtomicU64::new(0));
        let mut w = CountingWriter::new(FailingWriter, errors.clone());
        // 每次失败写入都计数（静默计数，不刷屏）
        assert!(w.write(b"a").is_err());
        assert!(w.write(b"b").is_err());
        assert_eq!(errors.load(Ordering::Relaxed), 2);
        assert!(w.flush().is_err());
        assert_eq!(errors.load(Ordering::Relaxed), 3, "flush 失败同样计数");
    }

    #[test]
    fn counting_writer_notifies_once_per_failure_episode() {
        let errors = Arc::new(AtomicU64::new(0));
        let w = CountingWriter::new(FailingWriter, errors.clone());
        // 首个失败阶段：should_notify 第一次返回 true（发 error），同阶段后续 false
        assert!(w.should_notify(), "首次失败应触发 error 提醒");
        assert!(!w.should_notify(), "同阶段不得重复提醒（防刷屏）");
        assert!(!w.should_notify());
        // 计数独立于提醒：无论是否提醒，失败都被计数
        w.note_failure();
        assert_eq!(errors.load(Ordering::Relaxed), 1);
        // 成功写入 → 复位 → 下一失败阶段可再次提醒
        w.mark_success();
        assert!(w.should_notify(), "成功写入后新失败阶段应能再次提醒");
        assert!(!w.should_notify());
    }

    #[test]
    fn counting_writer_recovers_after_success() {
        let errors = Arc::new(AtomicU64::new(0));
        let mut w = CountingWriter::new(io::sink(), errors.clone());
        // 写入成功：不计数、复位提醒标记
        assert_eq!(w.write(b"ok").unwrap(), 2);
        assert_eq!(errors.load(Ordering::Relaxed), 0);
        assert!(w.should_notify(), "成功后的下一次失败仍应触发提醒");
    }

    #[test]
    fn probe_log_file_writable_detects_unwritable_dir() {
        // 正常目录可写（临时目录中创建/追加日志文件）
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "missevan-test-probe-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(probe_log_file_writable(&dir), "可写目录探测应成功");
        assert!(
            dir.join(format!(
                "{}.{}",
                LOG_FILE_PREFIX,
                chrono::Local::now().format("%Y-%m-%d")
            ))
            .exists(),
            "探测应创建当日日志文件（与滚动 appender 命名一致）"
        );
        // 不存在的目录 → 不可写（打开失败）
        assert!(!probe_log_file_writable(&dir.join("no-such-dir")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── U5：日志级别热更新（白名单 + reload 句柄行为）──

    #[test]
    fn normalize_log_level_whitelist_falls_back_to_info() {
        assert_eq!(normalize_log_level("error"), "error");
        assert_eq!(normalize_log_level("warn"), "warn");
        assert_eq!(normalize_log_level("info"), "info");
        assert_eq!(normalize_log_level("debug"), "debug");
        assert_eq!(normalize_log_level("trace"), "trace");
        // 非法 / 空 / 大小写不匹配 → 一律回退 info（与启动规则一致）
        assert_eq!(normalize_log_level(""), "info");
        assert_eq!(normalize_log_level("verbose"), "info");
        assert_eq!(normalize_log_level("INFO"), "info");
        assert_eq!(normalize_log_level(" Debug "), "info");
    }

    /// 探针 callsite（兴趣切换验证用）：静态 Metadata，避开宏展开的 callsite
    /// interest 缓存（tracing-core 按调用点注册一次并永久缓存，测试中多次构造
    /// 订阅者会互相污染；生产路径由 `LogLevelReload::reload` 的
    /// `rebuild_interest_cache` 重建缓存，见其注释）。
    static PROBE_DEBUG_CS: std::sync::LazyLock<&'static tracing::callsite::DefaultCallsite> =
        std::sync::LazyLock::new(|| tracing::callsite! {
            name: "log_reload_probe_debug",
            kind: tracing::metadata::Kind::EVENT,
            target: "log_reload_probe",
            level: tracing::Level::DEBUG,
            fields: &[],
        });
    static PROBE_ERROR_CS: std::sync::LazyLock<&'static tracing::callsite::DefaultCallsite> =
        std::sync::LazyLock::new(|| tracing::callsite! {
            name: "log_reload_probe_error",
            kind: tracing::metadata::Kind::EVENT,
            target: "log_reload_probe",
            level: tracing::Level::ERROR,
            fields: &[],
        });

    #[test]
    fn log_level_reload_switches_filter_live() {
        use tracing::callsite::Callsite;
        // 静态探针 Metadata：验证 ReloadFilter::callsite_enabled 的兴趣随 reload
        // 切换（never → always → never）——这正是生产路径热更新 + rebuild 后
        // 各调用点重新注册得到的真实过滤语义。
        //
        // 说明：不做宏事件流断言——宏展开的调用点 interest 由 tracing-core 全局
        // 缓存且依赖全局 dispatcher 注册（with_default 线程局部不注册，rebuild
        // 不生效），单元测试环境不可控；生产路径由 `init_logging` 的全局
        // subscriber（.init() → set_global_default）保证 rebuild 生效，功能验证
        // 阶段用真实应用 + E2E 覆盖。
        let meta_debug: &'static tracing::Metadata<'static> = PROBE_DEBUG_CS.metadata();
        let meta_error: &'static tracing::Metadata<'static> = PROBE_ERROR_CS.metadata();
        let shared = Arc::new(RwLock::new(EnvFilter::new("info")));
        let filter = ReloadFilter {
            inner: shared.clone(),
        };
        let wrapper = LogLevelReload {
            inner: shared.clone(),
        };

        // info 级别：debug 调用点兴趣 never（将被缓存丢弃），error 兴趣 always
        //（Filter<S> 为泛型 trait，S 不参与 callsite_enabled 签名，UFCS 显式指定）
        type RF = ReloadFilter;
        let interest_debug = |f: &RF| {
            <RF as Filter<tracing_subscriber::Registry>>::callsite_enabled(f, meta_debug)
        };
        let interest_error = |f: &RF| {
            <RF as Filter<tracing_subscriber::Registry>>::callsite_enabled(f, meta_error)
        };
        assert!(interest_debug(&filter).is_never(), "info 级别下 debug 调用点应无兴趣");
        assert!(interest_error(&filter).is_always(), "error 调用点应始终感兴趣");

        // 热更新到 debug：debug 调用点兴趣变为 always（rebuild 后立即可达）
        assert!(wrapper.reload("debug"));
        assert!(interest_debug(&filter).is_always(), "reload 到 debug 后 debug 调用点应感兴趣");

        // 降级到 error：debug 调用点兴趣回到 never，error 保持 always
        assert!(wrapper.reload("error"));
        assert!(interest_debug(&filter).is_never(), "降级到 error 后 debug 调用点应再次无兴趣");
        assert!(interest_error(&filter).is_always());
    }

    #[test]
    fn log_level_reload_whitelist_via_wrapper() {
        // LogLevelReload 包装层：非法值回退 info（与启动同规则），reload 成功
        let shared = Arc::new(RwLock::new(EnvFilter::new("info")));
        let wrapper = LogLevelReload {
            inner: shared.clone(),
        };
        assert!(wrapper.reload("debug"), "合法级别应热更新成功");
        assert!(wrapper.reload("trace"));
        // 非法值不报错——按白名单回退 info（reload 本身仍成功）
        assert!(wrapper.reload("verbose"), "非法值回退 info 后仍应成功");
        assert!(wrapper.reload(""));
        // 共享同一锁的 ReloadFilter 能读到更新后的级别（间接验证已生效）
        let read = shared.read().unwrap();
        assert_eq!(
            read.max_level_hint(),
            Some(tracing::level_filters::LevelFilter::INFO)
        );
    }

    #[test]
    fn log_level_reload_no_deadlock_on_global_subscriber() {
        // 回归测试（对抗式验证发现 S1）：reload 曾在持有写锁时调用
        // rebuild_interest_cache——该 API 遍历全部已注册 callsite 并回调本 filter
        // 的 callsite_enabled（取读锁），同一线程 RwLock 非重入 → 必然死锁。
        //
        // 本测试复刻生产注册路径：带 ReloadFilter 的 subscriber 经
        // set_global_default 注册为全局默认（生产 init_logging 的 .init() 同路径），
        // 使 tracing 宏的真实 callsite 注册 / rebuild 回调均触达 ReloadFilter，
        // 若死锁回归则本测试超时挂起（cargo test 超时失败）。
        use std::sync::Mutex;
        use tracing::subscriber::set_global_default;

        // 收集层：统计到达的日志事件（验证 reload 后 debug 事件真实可观测）
        #[derive(Clone)]
        struct CollectLayer(Arc<Mutex<usize>>);
        impl<S> Layer<S> for CollectLayer
        where
            S: Subscriber + for<'a> LookupSpan<'a>,
        {
            fn on_event(&self, _event: &Event<'_>, _ctx: Context<'_, S>) {
                *self.0.lock().unwrap() += 1;
            }
        }

        let shared = Arc::new(RwLock::new(EnvFilter::new("info")));
        let count = Arc::new(Mutex::new(0usize));
        let subscriber = tracing_subscriber::registry().with(
            CollectLayer(count.clone()).with_filter(ReloadFilter {
                inner: shared.clone(),
            }),
        );
        // 全局 default 进程内只允许注册一次；测试并行下其余测试均用
        // with_default 局部覆盖不受影响，重复注册（Err）静默忽略即可。
        let _ = set_global_default(subscriber);

        tracing::info!("info 级事件");
        tracing::debug!("debug 级事件（info filter 下应被过滤）");
        assert_eq!(*count.lock().unwrap(), 1, "info 级别下仅 info 事件到达");

        let wrapper = LogLevelReload {
            inner: shared.clone(),
        };
        // 若死锁回归：此调用永不返回（测试挂起直至超时）
        assert!(wrapper.reload("debug"), "reload 应正常返回（不死锁）");
        // reload 内自身 info!（「日志级别已热更新为 debug」）在 debug filter 下
        // 放行，也到达收集层——此时 count = 1（前 info）+ 1（reload 内 info）
        assert_eq!(*count.lock().unwrap(), 2, "reload 内 info 事件到达");

        tracing::debug!("reload 后 debug 级事件可见");
        assert_eq!(
            *count.lock().unwrap(),
            3,
            "reload 后 debug 事件应真实到达收集层"
        );

        // 降级到 error：info/debug 全部过滤，error 可达
        assert!(wrapper.reload("error"));
        // reload 内 info! 在 error filter 下被过滤，count 不变
        assert_eq!(*count.lock().unwrap(), 3, "error filter 下 info 被过滤");
        tracing::error!("error 级事件可见");
        assert_eq!(
            *count.lock().unwrap(),
            4,
            "降级到 error 后 error 事件到达"
        );
    }
}
