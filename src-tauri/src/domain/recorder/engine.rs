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
    {
        let state = app_state.lock().await;
        if state.tasks.contains_key(&anchor.id) {
            return Err(already_recording_err(&anchor.id));
        }
    }
    if recorder.is_recording(&anchor.id) {
        return Err(already_recording_err(&anchor.id));
    }
    let anchor_id = anchor.id.clone();
    let anchor_id_for_monitor = anchor_id.clone(); // 给监控闭包
    let anchor_id_for_insert = anchor_id.clone();
    let anchor_name = anchor.name.clone();
    let room_id = anchor.room_id.clone();

    // 输出路径（H1：主播名/房间号来自外部 API/用户输入/导入配置，拼接路径前
    // 必须消毒——剔除 Windows 非法字符、控制字符与路径穿越段）
    let output_dir = &config.output_dir;
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let ext = &config.record_format;
    let safe_name = sanitize_path_component(&anchor_name);
    let safe_room = sanitize_path_component(&room_id);
    let anchor_dir = format!("{}-{}", safe_name, safe_room);
    let full_output_dir = format!("{}/{}", output_dir, anchor_dir);
    std::fs::create_dir_all(&full_output_dir)?;
    let output_path = format!("{}/{}_{}.{}", full_output_dir, safe_name, timestamp, ext);
    tracing::info!("[录制] 文件路径: {}", output_path);
    // 如果需要打印绝对路径（解决相对路径问题）
    if let Ok(abs_path) = std::path::absolute(&output_path) {
        tracing::info!("[录制] 绝对路径: {}", abs_path.display());
    }

    std::fs::create_dir_all(output_dir).map_err(|e| {
        AppError::system(
            "DIR_CREATE_FAIL",
            format!("创建输出目录失败: {}", output_dir),
        )
        .with_technical(format!("{}", e))
    })?;

    let output_dir = &config.output_dir;
    // 确保目录存在
    std::fs::create_dir_all(output_dir).map_err(|e| {
        AppError::system(
            "DIR_CREATE_FAIL",
            format!("创建输出目录失败: {}", output_dir),
        )
        .with_technical(format!("{}", e))
    })?;

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

    let mut state = app_state.lock().await;
    state.insert_task(anchor_id_for_insert, task);
    drop(state);

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
