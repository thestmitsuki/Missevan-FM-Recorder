use tokio::process::Child;
use tokio_util::sync::CancellationToken;

use crate::domain::config::manager::ConfigManager;
use crate::infrastructure::error::types::AppError;
use std::sync::Arc;
use tauri::Emitter;
use tauri::WebviewWindow;
use tokio::task::JoinHandle;

use crate::domain::config::model::AnchorStatusUpdate;
use crate::domain::config::model::{AnchorConfig, GlobalConfig};
use crate::domain::recorder::builder::FfmpegCommandBuilder;
use crate::domain::recorder::monitor::monitor_recording;
use crate::domain::services::file_cache::FileCacheHandle;
use crate::domain::spider::MissevanClient;
use crate::infrastructure::notification::dispatcher::NotificationDispatcher;
use crate::infrastructure::state::app_state::{AppStateHandle, Task};

/// 启动 FFmpeg 录制的核心函数
pub async fn start_ffmpeg_recording(
    anchor: AnchorConfig,
    stream_url: String,
    cancel_token: CancellationToken,
    config: GlobalConfig,
    recorder: Arc<FfmpegRecorder>,
    client: MissevanClient,
    notifier: Arc<NotificationDispatcher>,
    app_state: AppStateHandle,
    window: WebviewWindow,       // 新增
    file_cache: FileCacheHandle, // 新增
    config_manager: Arc<ConfigManager>,
) -> Result<(), AppError> {
    if stream_url.trim().is_empty() {
        return Err(AppError::config("流地址为空，无法启动录制"));
    }
    // M6（SSRF 纵深）：stream_url 来自外部 API（或 mock 配置），作为 ffmpeg
    // `-i` 输入前校验 scheme 为 http/https 且非回环/私网地址——拒绝 file:、
    // ftp: 等其它 scheme 与内网探测面。
    validate_stream_url(&stream_url)?;
    // 双录防御 #3：入口双保险——共享任务表已登记 或 共享进程表已存在 → 拒绝启动。
    // 检测循环的门控检查（loop.rs:355-358）发生在 spawn 调度前，此处关闭
    // spawn → 实际执行 间隙的竞态窗口（双录根因候选③）；
    // 与 insert_process 的锁内检查（防御 #2）构成注册前/注册时两道防线。
    // 同一锁区间内完成并发上限检查（max_concurrent_recordings，≥1 生效）
    // 与录制序号分配（filename_template {index}）。
    let recording_seq: u32;
    {
        let mut state = app_state.lock().await;
        if state.tasks.contains_key(&anchor.id) {
            return Err(already_recording_err(&anchor.id));
        }
        // 并发录制上限：活跃任务数 ≥ 上限时拒绝（0 = 不限制）。
        // 检查点设在任务注册前的最后一道门（与已录制检查同锁），
        // 检测循环的触发点到实际 spawn 之间的并发启动在此收敛。
        check_concurrency_limit(
            state.active_count(),
            config.max_concurrent_recordings,
            &anchor.name,
        )?;
        recording_seq = state.next_recording_seq(&anchor.id);
    }
    if recorder.is_recording(&anchor.id) {
        return Err(already_recording_err(&anchor.id));
    }
    let anchor_id = anchor.id.clone();
    let anchor_id_for_monitor = anchor_id.clone(); // 给监控闭包
    let anchor_id_for_insert = anchor_id.clone();
    let anchor_name = anchor.name.clone();
    let room_id = anchor.room_id.clone();

    // 输出路径（filename_template 渲染，H1：主播名/房间号来自外部 API/用户输入/
    // 导入配置，渲染后逐路径组件消毒——剔除 Windows 非法字符、控制字符与路径穿越段；
    // 模板含子目录时自动创建）
    let output_dir = config.output_dir.trim_end_matches(['/', '\\']);
    let ext = &config.record_format;
    let rendered = crate::domain::recorder::template::render_filename_template(
        &config.filename_template,
        &crate::domain::recorder::template::TemplateContext {
            anchor_name: &anchor_name,
            room_id: &room_id,
            now: chrono::Local::now(),
            index: recording_seq,
            ext,
        },
    );
    let output_path =
        build_recording_output_path(output_dir, &rendered, ext, config.segment_seconds);
    if let Some(parent) = std::path::Path::new(&output_path).parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            AppError::system(
                "DIR_CREATE_FAIL",
                format!("创建输出目录失败: {}", parent.display()),
            )
            .with_technical(format!("{}", e))
        })?;
    }
    tracing::info!("[录制] 文件路径: {}", output_path);
    // 如果需要打印绝对路径（解决相对路径问题）
    if let Ok(abs_path) = std::path::absolute(&output_path) {
        tracing::info!("[录制] 绝对路径: {}", abs_path.display());
    }

    // 构建 FFmpeg 命令（注意 mut）
    let mut ffmpeg_cmd =
        FfmpegCommandBuilder::from_config(&config, &stream_url, &output_path).build();

    // 启动子进程
    let mut child: Child = ffmpeg_cmd.spawn().map_err(|e| {
        AppError::system("FFMPEG_SPAWN_FAIL", "启动 FFmpeg 进程失败")
            .with_technical(format!("{}", e))
    })?;

    // 添加 stderr 读取
    if let Some(stderr) = child.stderr.take() {
        let anchor_name = anchor_name.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let reader = tokio::io::BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                // L2：ffmpeg 首行会回显输入 URL（CDN 地址常带签名 query）——
                // 截断到 256 字符，避免完整签名 URL 落入日志/诊断导出
                let clipped: String = line.chars().take(256).collect();
                tracing::info!("[FFmpeg stderr][{}] {}", anchor_name, clipped);
            }
        });
    }

    // 调试页展示所需（Task 15）：PID + 任务元数据
    let pid = child.id();

    // 存入 FfmpegRecorder（双录防御 #2：锁内重复检查——同 id 已有进程则拒绝
    // 注册并终止刚 spawn 的 child，消除「同 id 第二次注册覆盖条目但旧进程
    // 仍在跑」的隐患；失败路径在此返回：监控任务与任务表均未登记，无残留）
    if let Err(e) = recorder.insert_process(anchor_id.clone(), child).await {
        tracing::warn!("拒绝重复录制启动（进程表已存在同主播）: {}", e);
        return Err(e);
    }

    // 创建监控 task
    let monitor_cancel = cancel_token.clone();
    let recorder_clone = recorder.clone();
    let client_clone = client.clone();
    let notifier_clone = notifier.clone();
    let config_clone = config.clone();
    let window_for_monitor = window.clone();
    let file_cache_for_monitor = file_cache.clone();
    let config_manager_for_monitor = config_manager.clone();

    let app_state_for_monitor = app_state.clone();

    // 任务元数据（调试页「录制引擎」模块展示所需；async block 移走前克隆）
    let anchor_name_for_monitor = anchor_name.clone();
    let room_id_for_monitor = room_id.clone();
    let output_path_for_monitor = output_path.clone();
    let task_started_at = std::time::Instant::now();

    let monitor_handle: JoinHandle<()> = tokio::spawn(async move {
        if let Ok(meta) = tokio::fs::metadata(&output_path_for_monitor).await {
            tracing::info!("[录制] 当前文件大小: {} 字节", meta.len());
        } else {
            tracing::warn!("[录制] 文件尚未创建: {}", output_path_for_monitor);
        }
        monitor_recording(
            anchor_id_for_monitor, // 直接移动
            anchor_name_for_monitor,
            room_id_for_monitor,
            output_path_for_monitor,
            monitor_cancel,
            recorder_clone,
            client_clone,
            notifier_clone,
            config_clone,
            app_state_for_monitor,
            window_for_monitor,
            file_cache_for_monitor,
            config_manager_for_monitor,
        )
        .await;
    });

    // 注册任务
    // 最终注册复检（下方）需要主播名拼错误信息——Task 会移走 anchor_name，
    // 此处先克隆一份（其余克隆见上：anchor_id_for_insert / anchor_id_for_event）
    let anchor_name_for_recheck = anchor_name.clone();
    let task = Task {
        anchor_id: anchor_id.clone(), // 或 anchor_id_for_insert.clone()
        cancel_token,
        handle: monitor_handle,
        anchor_name,
        room_id,
        output_path,
        started_at: task_started_at,
        pid,
    };

    // 注册任务
    // 在 insert_task 之前克隆一份用于事件
    let anchor_id_for_event = anchor_id_for_insert.clone();

    // 最终注册前锁内复检（并发上限 TOCTOU 修复）：入口并发检查（函数开头）
    // 发生在 ffmpeg spawn 之前——跨主播并发启动时，B 可能在 A 注册进任务表
    // 之前通过上限检查，双双越过 max_concurrent_recordings。此处与 insert_task
    // 同一锁区间内重新核对：
    //   1. 同主播已注册（双录防御 #3 的注册时复检：spawn 窗口内检测循环重入 /
    //      手动触发都可能重复启动）
    //   2. 活跃任务数已达并发上限（拒绝）
    // 拒绝路径复用双录防御的拒绝模式：终止已 spawn 的进程（recorder.stop 发 q
    // 优雅退出）并 abort monitor 任务——录制从未真正开始，monitor 的清理流程
    // （通知/历史摘要/录制后动作）不应执行。监控任务与任务表均未登记，无残留。
    {
        let mut state = app_state.lock().await;
        if state.tasks.contains_key(&anchor_id_for_insert) {
            drop(state);
            task.handle.abort();
            let _ = recorder.stop(&anchor_id).await;
            return Err(already_recording_err(&anchor_id_for_insert));
        }
        if let Err(err) = check_concurrency_limit(
            state.active_count(),
            config.max_concurrent_recordings,
            &anchor_name_for_recheck,
        ) {
            drop(state);
            task.handle.abort();
            let _ = recorder.stop(&anchor_id).await;
            return Err(err);
        }
        state.insert_task(anchor_id_for_insert, task);
    }

    let update = AnchorStatusUpdate {
        anchor_id: anchor_id_for_event, // 使用预先克隆的副本
        is_live: true,
        is_recording: true,
    };
    let _ = window.emit("recording_status_changed", &update);

    Ok(())
}

/// 录制引擎 trait——真实 FFmpeg 和 Mock 实现此接口
///
/// start/is_recording 是设计文档为「Mock 引擎接线」（可选）预留的接口面，
/// 当前实际录制路径直连 start_ffmpeg_recording + stop（monitor.rs），
/// 故这两个方法标 allow(dead_code)；stop 经 monitor.rs 以 trait 方法调用。
#[async_trait::async_trait]
pub trait RecorderEngine: Send + Sync {
    /// 开始录制
    #[allow(dead_code)]
    async fn start(
        &self,
        config: &GlobalConfig,
        stream_url: &str,
        output_path: &str,
        cancel: CancellationToken,
    ) -> Result<(), AppError>;
    /// 停止录制（发送 q 信号）
    async fn stop(&self, anchor_id: &str) -> Result<(), AppError>;
    /// 是否正在录制
    #[allow(dead_code)]
    fn is_recording(&self, anchor_id: &str) -> bool;
}

/// FFmpeg 录制引擎——管理 FFmpeg 子进程生命周期
pub struct FfmpegRecorder {
    processes: std::sync::Mutex<std::collections::HashMap<String, ProcessInfo>>,
}

struct ProcessInfo {
    child: Option<Child>,
}

impl FfmpegRecorder {
    pub fn new() -> Self {
        Self {
            processes: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// 注册 FFmpeg 子进程（双录防御 #2）。
    ///
    /// 锁内检查 `procs.contains_key(anchor_id)`：已存在则拒绝新进程（**不覆盖**），
    /// 终止刚 spawn 的 child 后返回 `Err(RC_ALREADY_RECORDING)`——消除「同 id
    /// 第二次注册覆盖条目但旧进程仍在跑」的隐患。检查+插入在同一锁持有区间内
    /// 原子完成（无 TOCTOU）；调用方（start_ffmpeg_recording）收到 Err 后直接
    /// 失败返回（child 已在此处终止）。
    pub async fn insert_process(&self, anchor_id: String, child: Child) -> Result<(), AppError> {
        // 检查+插入在同一锁持有区间内原子完成（无 TOCTOU）；
        // 锁在块结束时释放，不在任何 await 点持有（Send 约束）
        {
            let mut procs = self.processes.lock().unwrap();
            if !procs.contains_key(&anchor_id) {
                procs.insert(anchor_id, ProcessInfo { child: Some(child) });
                return Ok(());
            }
        }
        // 已存在：拒绝新进程（不覆盖），终止刚 spawn 的 child 后返回错误
        let mut child = child;
        let _ = child.kill().await;
        let _ = child.wait().await;
        Err(already_recording_err(&anchor_id))
    }
}

/// 校验录制流地址（M6）：仅允许 http/https scheme，且拒绝回环地址
/// （localhost / 127.* / ::1）与私网字面 IP（10.* / 192.168.* / 172.16-31.* /
/// 169.254.*）。域名不做 DNS 解析（避免阻塞检测循环），非字面 IP 的域名
/// 仅做 scheme 校验。
fn validate_stream_url(url: &str) -> Result<(), AppError> {
    let parsed = url::Url::parse(url)
        .map_err(|_| AppError::config("流地址不是有效 URL"))?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(AppError::config(format!(
            "流地址 scheme 不支持: {}（仅允许 http/https）",
            scheme
        )));
    }
    let Some(host) = parsed.host() else {
        return Err(AppError::config("流地址缺少主机名"));
    };
    match host {
        url::Host::Ipv4(v4) => {
            if v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
            {
                return Err(AppError::config("流地址不允许使用回环/私网地址"));
            }
        }
        url::Host::Ipv6(v6) => {
            if v6.is_loopback() || v6.is_unspecified() || v6.is_unique_local() {
                return Err(AppError::config("流地址不允许使用回环/私网地址"));
            }
        }
        url::Host::Domain(domain) => {
            let d = domain.to_ascii_lowercase();
            if d == "localhost" || d.ends_with(".localhost") {
                return Err(AppError::config("流地址不允许使用 localhost"));
            }
        }
    }
    Ok(())
}

/// 路径组件消毒（H1）：把不可信字符串（主播名/房间号——来自外部 API、用户
/// 输入或导入配置）转为安全的单个路径组件，防止路径穿越与非法文件名字符。
///
/// 规则：
/// - Windows 非法字符 `<>:"/\|?*` 与控制字符（`\x00`-`\x1F`）替换为 `_`
///   （路径分隔符被替换后，`..` 无法再构成上级目录段）
/// - `..` 子串替换为 `_`（纵深防御，防 `.`/`..` 段残留）
/// - 去除首尾空白
/// - 结果为 `.`/`..`/空时返回占位 `_`
pub fn sanitize_path_component(raw: &str) -> String {
    let mut out: String = raw
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    out = out.replace("..", "_");
    let trimmed = out.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        "_".to_string()
    } else {
        trimmed.to_string()
    }
}

/// 构造「已在录制」错误（RC_ALREADY_RECORDING，录制类；入口检查与进程表
/// 去重共用，保证错误语义一致）
fn already_recording_err(anchor_id: &str) -> AppError {
    AppError::recording(
        crate::infrastructure::error::types::RC_ALREADY_RECORDING,
        format!("主播 {} 已在录制中", anchor_id),
    )
}

/// 输出路径去重（实装审查跟进，数据丢失风险兜底）：目标文件已存在（如同秒
/// 碰撞——两个同名主播同时录制，或上次录制残留）时在扩展名前追加 `_2`、`_3`
/// …（最多 100 次，之后放弃追加原样返回）。
///
/// best-effort：检查与 ffmpeg 实际创建之间仍有极小竞态窗口，但默认模板含
/// `{index}`（每主播单调递增）已消除常规碰撞；此处主要兜底跨主播同秒同名
/// 与用户自定模板不含序号的情况。
fn deduplicate_output_path(path: &str) -> String {
    if !std::path::Path::new(path).exists() {
        return path.to_string();
    }
    // 最后一个 `.` 作为扩展名分隔（目录名含点不影响：rsplit_once 取最后一段）
    let (stem, ext) = match path.rsplit_once('.') {
        Some((s, e)) if !e.is_empty() => (s, e),
        _ => (path, ""),
    };
    for n in 2..=100u32 {
        let candidate = if ext.is_empty() {
            format!("{}_{}", stem, n)
        } else {
            format!("{}_{}.{}", stem, n, ext)
        };
        if !std::path::Path::new(&candidate).exists() {
            return candidate;
        }
    }
    path.to_string()
}

/// 构造录制输出路径（纯函数，便于单测）：`{output_dir}/{模板渲染结果}`。
///
/// 模板渲染结果（render_filename_template）含**完整**相对路径——模板的子目录
/// 部分与音频文件名部分都来自用户模板（如默认模板
/// `{anchor_name}/{date}_{time}_{anchor_name}_{index}.{ext}` → 子目录
/// `主播A/` + 文件名 `2026-08-07_12-30-45_主播A_001.m4a`）；本函数只做三段收尾：
/// - 拼接输出目录；
/// - 分段模式：去掉尾部 `.{ext}`，由 builder 追加 `_%03d.{ext}` 输出 pattern
///   （builder 契约：分段模式 output_path 不带扩展名，避免 `xxx.m4a_000.m4a`）；
/// - 非分段模式：模板文件名部分未写 `{ext}` 时补上真实格式扩展名——否则
///   ffmpeg 无法从扩展名推断封装格式直接失败，且文件缓存/清理服务的扩展名
///   白名单（m4a/aac/mp3/flac）扫不到该文件（与分段模式 builder 恒追加
///   `_%03d.{ext}` 的行为对齐）；随后目标文件已存在（同秒碰撞——两个同名
///   主播同时录制 / 上次残留）时自动追加序号，防 ffmpeg `-y` 覆盖已有录制。
///   分段模式由 ffmpeg 自带 `%03d` 序号管理，不在此去重。
fn build_recording_output_path(
    output_dir: &str,
    rendered: &str,
    ext: &str,
    segment_seconds: u64,
) -> String {
    let mut output_path = format!("{}/{}", output_dir, rendered);
    let suffix = format!(".{}", ext);
    if segment_seconds > 0 {
        if let Some(stripped) = output_path.strip_suffix(&suffix) {
            output_path = stripped.to_string();
        }
    } else {
        if !output_path.ends_with(&suffix) {
            output_path.push_str(&suffix);
        }
        output_path = deduplicate_output_path(&output_path);
    }
    output_path
}

/// 并发录制上限检查（纯函数，便于单测）：`max_concurrent > 0` 且活跃数
/// 达到上限 → `Err(RC_CONCURRENCY_LIMIT)`（记录日志）；`0` = 不限制。
fn check_concurrency_limit(
    active_count: usize,
    max_concurrent: u32,
    anchor_name: &str,
) -> Result<(), AppError> {
    if max_concurrent > 0 && active_count >= max_concurrent as usize {
        let err = AppError::recording(
            crate::infrastructure::error::types::RC_CONCURRENCY_LIMIT,
            format!(
                "已达并发录制上限（{} 个），拒绝启动新录制: {}",
                max_concurrent, anchor_name
            ),
        );
        tracing::warn!("[录制] {}", err.message);
        return Err(err);
    }
    Ok(())
}

#[async_trait::async_trait]
impl RecorderEngine for FfmpegRecorder {
    async fn start(
        &self,
        _config: &GlobalConfig,
        _stream_url: &str,
        _output_path: &str,
        _cancel: CancellationToken,
    ) -> Result<(), AppError> {
        // 此方法被 design 为从 detector 调用，它 spawn 监控循环
        // 实际子进程启动在 builder.rs + monitor.rs 中完成
        Ok(())
    }

    async fn stop(&self, anchor_id: &str) -> Result<(), AppError> {
        // 在同步作用域内取出 child，然后释放锁再 await
        let child = {
            let mut procs = self
                .processes
                .lock()
                .map_err(|_| AppError::internal("锁获取失败"))?;
            procs.remove(anchor_id).and_then(|info| info.child)
        }; // MutexGuard 在此处被 drop，确保不会跨 await 持有

        if let Some(mut child) = child {
            // 发送 'q' 到 stdin 优雅退出
            if let Some(stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                let mut stdin = stdin;
                let _ = stdin.write_all(b"q\n").await;
                let _ = stdin.flush().await;
            }
            // 等待子进程退出
            let _ = child.wait().await;
        }
        Ok(())
    }

    fn is_recording(&self, anchor_id: &str) -> bool {
        self.processes
            .lock()
            .map(|p| p.contains_key(anchor_id))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用假子进程：Windows 下 cmd /c exit（立即退出），其他平台 true
    async fn spawn_fake_child() -> Child {
        #[cfg(windows)]
        let mut cmd = tokio::process::Command::new("cmd");
        #[cfg(windows)]
        cmd.args(["/c", "exit"]);
        #[cfg(not(windows))]
        let mut cmd = tokio::process::Command::new("true");
        cmd.spawn().expect("spawn 测试子进程失败")
    }

    // ── 双录防御 #2：insert_process 锁内重复检查 ──

    #[tokio::test]
    async fn insert_process_rejects_duplicate_anchor_id() {
        let recorder = FfmpegRecorder::new();
        let child = spawn_fake_child().await;
        assert!(
            recorder.insert_process("a".into(), child).await.is_ok(),
            "首次注册应成功"
        );
        // 同 id 第二次注册 → 拒绝（不覆盖，返回 RC_ALREADY_RECORDING）
        let child2 = spawn_fake_child().await;
        let err = recorder
            .insert_process("a".into(), child2)
            .await
            .expect_err("同 id 重复注册必须被拒绝");
        assert_eq!(
            err.code, crate::infrastructure::error::types::RC_ALREADY_RECORDING,
            "错误码应为 RC_ALREADY_RECORDING"
        );
        // 不同 id → 放行（互不影响）
        let child3 = spawn_fake_child().await;
        assert!(
            recorder.insert_process("b".into(), child3).await.is_ok(),
            "不同主播的注册应互不影响"
        );
        // 原注册未被覆盖
        assert!(recorder.is_recording("a"));
        assert!(recorder.is_recording("b"));
    }

    // ── M6：流地址校验 ──

    #[test]
    fn validate_stream_url_accepts_public_http_https() {
        assert!(validate_stream_url("https://stream.example.com/live.flv").is_ok());
        assert!(validate_stream_url("http://stream.example.com/live").is_ok());
        assert!(validate_stream_url("https://8.8.8.8/live").is_ok());
    }

    #[test]
    fn validate_stream_url_rejects_bad_schemes_and_private_hosts() {
        for bad in [
            "file:///tmp/evil.m4a",
            "ftp://example.com/a.flv",
            "rtmp://example.com/live",
            "https://localhost:8080/live",
            "http://127.0.0.1:8080/live",
            "http://10.0.0.5/live",
            "http://192.168.1.1/live",
            "http://172.16.0.1/live",
            "http://172.31.255.255/live",
            "http://169.254.169.254/latest/meta-data",
            "http://[::1]:8080/live",
            "http://0.0.0.0/live",
            "not a url",
        ] {
            assert!(
                validate_stream_url(bad).is_err(),
                "应拒绝: {}",
                bad
            );
        }
    }

    // ── H1：路径组件消毒 ──

    #[test]
    fn sanitize_path_component_strips_traversal() {
        // 路径穿越尝试（Windows 反斜杠 / POSIX 斜杠）
        assert_eq!(sanitize_path_component(r"..\..\..\Users\admin\evil"), "______Users_admin_evil");
        assert_eq!(sanitize_path_component("../../evil"), "____evil");
        // 单独的点段 / 空串 → 占位
        assert_eq!(sanitize_path_component(".."), "_");
        assert_eq!(sanitize_path_component("."), "_");
        assert_eq!(sanitize_path_component(""), "_");
        assert_eq!(sanitize_path_component("   "), "_");
    }

    #[test]
    fn sanitize_path_component_replaces_windows_invalid_chars_and_controls() {
        // Windows 非法字符全部替换为 _
        assert_eq!(
            sanitize_path_component("a<b>c:d\"e/f\\g|h?i*j"),
            "a_b_c_d_e_f_g_h_i_j"
        );
        // 控制字符
        assert_eq!(sanitize_path_component("abc\x00def\x1f"), "abc_def_");
        // 首尾空白去除，内部空白保留
        assert_eq!(sanitize_path_component("  主播名  "), "主播名");
        assert_eq!(sanitize_path_component("主 播 名"), "主 播 名");
    }

    #[test]
    fn sanitize_path_component_keeps_normal_names() {
        assert_eq!(sanitize_path_component("主播A"), "主播A");
        assert_eq!(sanitize_path_component("123456"), "123456");
        assert_eq!(sanitize_path_component("主播名.D"), "主播名.D");
        // 与目录命名约定兼容：`{safe_name}-{safe_room}` 不产生分隔符
        let dir = format!(
            "{}-{}",
            sanitize_path_component("主播A"),
            sanitize_path_component("100000001")
        );
        assert_eq!(dir, "主播A-100000001");
    }

    // ── 并发录制上限（max_concurrent_recordings 接线）──

    #[test]
    fn concurrency_limit_rejects_when_at_or_over_cap() {
        // 上限 2：活跃 2（等于上限）→ 拒绝；上限 0 = 不限制
        let err = check_concurrency_limit(2, 2, "主播A").unwrap_err();
        assert_eq!(
            err.code, crate::infrastructure::error::types::RC_CONCURRENCY_LIMIT,
            "达到上限必须拒绝: {}",
            err
        );
        let err = check_concurrency_limit(5, 2, "主播A").unwrap_err();
        assert_eq!(err.code, crate::infrastructure::error::types::RC_CONCURRENCY_LIMIT);
        assert!(err.message.contains("2"), "错误信息应含上限值: {}", err.message);
    }

    #[test]
    fn concurrency_limit_passes_below_cap_and_when_disabled() {
        // 上限 2、活跃 1 → 放行
        assert!(check_concurrency_limit(1, 2, "主播A").is_ok());
        // 上限 0 = 不限制（默认配置）
        assert!(check_concurrency_limit(10, 0, "主播A").is_ok());
    }

    // ── 输出路径去重（防 ffmpeg -y 覆盖已有文件）──

    /// 唯一临时目录（并行测试隔离）
    fn unique_dir(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "missevan-dedup-{}-{}-{}",
            tag,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ))
    }

    #[test]
    fn dedup_appends_suffix_when_target_exists() {
        let dir = unique_dir("exists");
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("live.m4a");
        std::fs::write(&base, b"x").unwrap();
        let base_s = base.to_string_lossy().into_owned();
        let expect_2 = dir.join("live_2.m4a").to_string_lossy().into_owned();
        assert_eq!(deduplicate_output_path(&base_s), expect_2, "已存在 → 追加 _2");
        // _2 也被占用 → _3
        std::fs::write(&expect_2, b"x").unwrap();
        let expect_3 = dir.join("live_3.m4a").to_string_lossy().into_owned();
        assert_eq!(deduplicate_output_path(&base_s), expect_3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dedup_returns_path_unchanged_when_free() {
        let dir = unique_dir("free");
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("never-exists.m4a");
        assert_eq!(
            deduplicate_output_path(&base.to_string_lossy()),
            base.to_string_lossy().into_owned(),
            "目标不存在 → 原样返回"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dedup_handles_no_extension_and_dotted_dirs() {
        let dir = unique_dir("noext");
        std::fs::create_dir_all(&dir).unwrap();
        // 无扩展名
        let base = dir.join("clip");
        std::fs::write(&base, b"x").unwrap();
        assert_eq!(
            deduplicate_output_path(&base.to_string_lossy()),
            dir.join("clip_2").to_string_lossy().into_owned()
        );
        // 目录名含点：最后一个 . 是扩展名分隔符，_2 追加在文件名上
        let nested = dir.join("with.dot").join("live.m4a");
        std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
        std::fs::write(&nested, b"x").unwrap();
        assert_eq!(
            deduplicate_output_path(&nested.to_string_lossy()),
            dir.join("with.dot")
                .join("live_2.m4a")
                .to_string_lossy()
                .into_owned()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── 输出路径构造（模板渲染结果 → 录制输出路径；音频文件名按模板渲染）──

    #[test]
    fn output_path_keeps_template_directory_and_filename() {
        // 模板含子目录：目录与音频文件名都来自模板渲染结果，不收尾改动
        let p = build_recording_output_path(
            "D:/rec",
            "主播A/2026-08-07_12-30-45_主播A_001.m4a",
            "m4a",
            0,
        );
        assert_eq!(p, "D:/rec/主播A/2026-08-07_12-30-45_主播A_001.m4a");
    }

    #[test]
    fn output_path_no_subdir_template_stays_single_component() {
        // 模板无子目录（渲染结果单组件）：输出路径不产生多余目录
        let p = build_recording_output_path(
            "D:/rec",
            "2026-08-07_12-30-45_主播A_001.m4a",
            "m4a",
            0,
        );
        assert_eq!(p, "D:/rec/2026-08-07_12-30-45_主播A_001.m4a");
        // 模板文件名部分未写 {ext} → 补上真实格式扩展名（否则 ffmpeg 无法
        // 推断封装格式，且扩展名白名单扫不到）
        let p2 = build_recording_output_path("D:/rec", "主播A/001", "m4a", 0);
        assert_eq!(p2, "D:/rec/主播A/001.m4a");
        let p3 = build_recording_output_path("D:/rec", "主播A/001", "mp3", 0);
        assert_eq!(p3, "D:/rec/主播A/001.mp3");
    }

    #[test]
    fn output_path_segment_strips_extension_for_builder_pattern() {
        // 分段模式：去掉尾部 .{ext}（builder 追加 _%03d.{ext}，不产生
        // `xxx.m4a_000.m4a`）
        let p = build_recording_output_path(
            "D:/rec",
            "主播A/2026-08-07_12-30-45_主播A_001.m4a",
            "m4a",
            600,
        );
        assert_eq!(p, "D:/rec/主播A/2026-08-07_12-30-45_主播A_001");
        // 模板未写 {ext} 的分段输出：没有可剥离的扩展名，原样保留——
        // builder 追加 _%03d.{ext} 仍产出真实扩展名
        let p2 = build_recording_output_path("D:/rec", "主播A/001", "m4a", 600);
        assert_eq!(p2, "D:/rec/主播A/001");
    }

    #[test]
    fn output_path_dedup_still_applied_in_non_segment() {
        // deduplicate_output_path 保留：非分段且目标已存在 → 追加 _2
        let dir = unique_dir("build-path");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("主播A.m4a"), b"x").unwrap();
        let p = build_recording_output_path(&dir.to_string_lossy(), "主播A.m4a", "m4a", 0);
        assert_eq!(
            p,
            format!("{}/主播A_2.m4a", dir.to_string_lossy()),
            "目标已存在 → 追加 _2"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn is_recording_reflects_registry_state() {
        let recorder = FfmpegRecorder::new();
        assert!(!recorder.is_recording("a"), "未注册时应为 false");
        let child = spawn_fake_child().await;
        recorder.insert_process("a".into(), child).await.unwrap();
        assert!(recorder.is_recording("a"), "注册后应为 true");
        // stop 移除后恢复 false（与 monitor.rs 停止路径一致）
        recorder.stop("a").await.unwrap();
        assert!(!recorder.is_recording("a"), "移除后应为 false");
        // 移除后同 id 可重新注册（录制结束 → 再次开播的场景）
        let child2 = spawn_fake_child().await;
        assert!(
            recorder.insert_process("a".into(), child2).await.is_ok(),
            "进程移除后应可重新注册"
        );
    }
}
