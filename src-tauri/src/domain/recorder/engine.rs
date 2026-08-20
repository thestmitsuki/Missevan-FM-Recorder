use tokio::process::Child;
use tokio_util::sync::CancellationToken;

use crate::domain::config::manager::ConfigManager;
use crate::infrastructure::error::types::AppError;
use std::sync::Arc;
use std::time::Duration;
use tauri::Emitter;
use tauri::WebviewWindow;
use tokio::task::JoinHandle;

use crate::domain::config::model::AnchorStatusUpdate;
use crate::domain::config::model::{AnchorConfig, GlobalConfig};
use crate::domain::recorder::builder::FfmpegCommandBuilder;
use crate::domain::recorder::disk::{check_disk_space, DiskSpaceStatus};
use crate::domain::recorder::monitor::monitor_recording;
use crate::domain::services::file_cache::FileCacheHandle;
use crate::domain::spider::MissevanClient;
use crate::infrastructure::notification::dispatcher::NotificationDispatcher;
use crate::infrastructure::state::app_state::{AppStateHandle, Task};

// ── B2：停止流程超时参数 ──
/// 优雅停止等待上限：ffmpeg 收到 stdin 'q' 后应秒级退出（flush 输出、写尾部
/// 元数据）；网络 IO 阻塞等卡死场景超过该上限后强制 kill。取值 8s：介于
/// 「正常退出等待（通常 <2s）」与「停录响应可接受上限（10s）」之间。
const STOP_GRACEFUL_TIMEOUT: Duration = Duration::from_secs(8);
/// 强制 kill 后的收割等待上限：kill 后必须 await wait() 回收（防僵尸进程）；
/// 再设上限兜底 kill 后 wait 仍悬挂的罕见场景（Linux 不可中断睡眠 D 状态等），
/// 再超时则放弃等待（进程已被杀，句柄 drop 由 kill_on_drop 兜底重试）。
const STOP_FORCE_KILL_WAIT: Duration = Duration::from_secs(3);

/// 录制子进程共享句柄：engine（spawn/stop）与 monitor（存活探测）经同一句柄
/// 访问 tokio::process::Child。内层 `std::sync::Mutex` 保证 `try_wait`/`wait`
/// 需要的 `&mut Child` 独占访问；外层进程表 `std::sync::Mutex` 保护表本身
/// （与既有架构一致；两把锁均不跨 await 持有）。
type SharedChild = Arc<std::sync::Mutex<Option<Child>>>;

/// 录制子进程存活探测结果（B1）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildProbe {
    /// 进程表无该主播条目，或句柄已被停止流程取走——不视为退出，跳过本轮探测
    Unknown,
    /// 子进程仍在运行
    Running,
    /// 子进程已退出（已被 try_wait 收割，退出状态可用）
    Exited(std::process::ExitStatus),
}

/// 异常退出判定（纯逻辑，便于单测；B1）：仅当「子进程已退出」且「取消令牌未
/// 触发」判定为异常退出——停止流程正在终止子进程时（cancel 已触发）子进程退出
/// 属正常停止，不得误判为崩溃。
pub fn is_abnormal_exit(probe: ChildProbe, cancel_requested: bool) -> bool {
    matches!(probe, ChildProbe::Exited(_)) && !cancel_requested
}

/// 停止流程动作（B2 超时决策）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopAction {
    /// 正常退出 / 已收割：无需强杀
    Reaped,
    /// wait 出错（进程状态未知）：不强杀（交由 kill_on_drop 兜底）
    SkipForceKill,
    /// 优雅等待超时：强制 kill
    ForceKill,
}

/// 停止超时决策（纯逻辑，便于单测；B2）：优雅等待超时 → 强制 kill（超时优先
/// 于 wait 出错）；wait 出错 → 不强杀（状态未知，强杀可能误伤）；否则正常退出。
pub fn decide_stop_action(timed_out: bool, wait_errored: bool) -> StopAction {
    if timed_out {
        StopAction::ForceKill
    } else if wait_errored {
        StopAction::SkipForceKill
    } else {
        StopAction::Reaped
    }
}

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
    // 同一锁区间内完成并发上限检查（max_concurrent_recordings，≥1 生效）。
    {
        let state = app_state.lock().await;
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

    // S2a：磁盘预检查（disk_space_limit_gb 接入录制主链路）——每次 ffmpeg 启动
    // （新录制/分段均为单次启动）前检查输出目录所在卷剩余空间，低于阈值**拒绝
    // 启动**（记录明确日志 + 节流 DISK_LOW 通知），而不是启动后磁盘写满 ENOSPC
    // 崩溃 → 检测循环立即重启的崩溃-重启循环。0 = 不限制（跳过，无系统调用）；
    // 查询失败按放行处理（与健康检查 Warning 同语义，探测失败不误伤录制）。
    // 检查发生在 create_dir_all 之后（目录已存在，fs2 跨平台可用）。
    if let DiskSpaceStatus::Low {
        available_gb,
        threshold_gb,
    } = check_disk_space(&output_dir, config.disk_space_limit_gb)
    {
        // 节流 DISK 通知（与 S3 定期预警共用 AppState 冷却；app_state 锁在
        // 同步作用域内短暂持有，不跨 await）
        let should_notify = app_state.lock().await.disk_notify_allowed();
        if should_notify {
            notifier
                .warning(
                    "DISK_LOW",
                    "磁盘空间不足，暂停新录制",
                    format!(
                        "剩余 {} GB，低于阈值 {} GB，拒绝启动录制: {}",
                        available_gb, threshold_gb, anchor_name
                    ),
                )
                .await;
        }
        tracing::error!(
            "[录制] 磁盘空间不足（剩余 {} GB < 阈值 {} GB），拒绝启动录制: {}",
            available_gb,
            threshold_gb,
            anchor_name
        );
        return Err(
            AppError::recording(
                crate::infrastructure::error::types::RC_DISK_LOW,
                format!(
                    "磁盘空间不足（剩余 {} GB < 阈值 {} GB），拒绝启动录制: {}",
                    available_gb, threshold_gb, anchor_name
                ),
            )
            .with_suggestion("请清理磁盘空间，或降低磁盘阈值设置（disk_space_limit_gb）"),
        );
    }

    // H3/H5：录制活动标记——ffmpeg 即将写入输出文件前创建 `{output_path}.recording`
    // 标记（内容含分段/格式规则，供启动清理精确还原产物范围）。正常收尾在
    // monitor.rs 统一出口移除；应用被强杀/断电时残留 → 下次启动据此识别并清理
    // 孤儿 ffmpeg 产物。写失败仅 warn（启动清理能力降级，不阻断录制）。
    write_recording_marker(&output_path, config.segment_seconds, ext);

    // 构建 FFmpeg 命令（注意 mut）
    let mut ffmpeg_cmd =
        FfmpegCommandBuilder::from_config(&config, &stream_url, &output_path).build();
    // B2 兜底：子进程句柄在任何路径被 drop（任务中止/应用退出）时强杀 ffmpeg，
    // 防孤儿进程（tokio 在子进程已被收割后自动禁用，无害）。
    ffmpeg_cmd.kill_on_drop(true);

    // 启动子进程
    let mut child: Child = ffmpeg_cmd.spawn().map_err(|e| {
        // spawn 失败：本次录制未开始，移除刚写入的活动标记（防残留）
        remove_recording_marker(&output_path);
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
        // 重复注册被拒：刚 spawn 的 child 已被终止，移除活动标记（无残留）
        remove_recording_marker(&output_path);
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
        // 克隆一份：下方「最终注册前锁内复检」拒绝路径还需使用 output_path
        // 移除活动标记（remove_recording_marker）
        output_path: output_path.clone(),
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
            // 录制从未真正开始：移除活动标记（本路径无 monitor 收尾）
            remove_recording_marker(&output_path);
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
            // 录制从未真正开始：移除活动标记（本路径无 monitor 收尾）
            remove_recording_marker(&output_path);
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
    child: SharedChild,
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
        // B2 兜底：kill_on_drop 已在 spawn 前的 Command 上设置（见
        // start_ffmpeg_recording）——句柄在任何路径被 drop 时强杀子进程。
        let shared: SharedChild = Arc::new(std::sync::Mutex::new(Some(child)));
        // 检查+插入在同一锁持有区间内原子完成（无 TOCTOU）；
        // 锁在块结束时释放，不在任何 await 点持有（Send 约束）
        {
            // 锁中毒（他处 panic 持锁）时恢复现场继续处理，不 panic
            let mut procs = match self.processes.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            if !procs.contains_key(&anchor_id) {
                procs.insert(anchor_id, ProcessInfo { child: shared });
                return Ok(());
            }
        }
        // 已存在：拒绝新进程（不覆盖），终止刚 spawn 的 child 后返回错误
        let child = {
            let mut guard = match shared.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            guard.take()
        };
        if let Some(mut child) = child {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        Err(already_recording_err(&anchor_id))
    }

    /// 探测录制子进程是否已退出（B1）。同步方法（try_wait 非异步），不阻塞
    /// 调用方；两把锁均只在同步作用域短暂持有，不跨 await（无死锁）。
    ///
    /// 与停止流程的并发：probe 与 stop 经共享句柄内锁互斥——stop 取出 child 后
    /// probe 返回 Unknown（不误判）；probe 先 try_wait 收割后 stop 的 wait() 拿到
    /// 的是 tokio 缓存的退出状态（tokio 1.53 源码核验：FusedChild::Done 分支返回
    /// 缓存值），不会 panic、不会重复收割。
    pub fn probe_process(&self, anchor_id: &str) -> ChildProbe {
        let shared = {
            let procs = match self.processes.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            match procs.get(anchor_id) {
                Some(info) => info.child.clone(),
                None => return ChildProbe::Unknown,
            }
        };
        let mut guard = match shared.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let Some(child) = guard.as_mut() else {
            return ChildProbe::Unknown;
        };
        match child.try_wait() {
            Ok(Some(status)) => ChildProbe::Exited(status),
            Ok(None) => ChildProbe::Running,
            Err(e) => {
                tracing::warn!(
                    "[录制] 探测子进程状态失败（按 Unknown 处理，不误判崩溃）: {}",
                    e
                );
                ChildProbe::Unknown
            }
        }
    }

    /// 当前进程表中的主播 id 快照（应用退出兜底用）
    pub fn active_anchor_ids(&self) -> Vec<String> {
        let procs = match self.processes.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        procs.keys().cloned().collect()
    }

    /// 强制终止全部活动录制子进程（应用退出兜底，B2）：逐个走带超时的优雅停止
    /// + 超时强杀；无条目 / 已停止的幂等跳过。
    pub async fn force_terminate_all(&self) {
        let ids = self.active_anchor_ids();
        for id in ids {
            if let Err(e) = self.stop(&id).await {
                tracing::warn!("[录制] 退出前强制终止失败: {}", e);
            }
        }
    }

    /// 停止流程内部实现（B2）：优雅终止（stdin 写 'q'）→ 等待上限 → 超时强制
    /// kill → 收割（防僵尸）。超时参数可注入（生产值见 RecorderEngine::stop，
    /// 测试用毫秒级值验证强制 kill 路径）。await 期间不持任何锁（两把锁均只在
    /// 同步作用域持有）。
    async fn stop_inner(
        &self,
        anchor_id: &str,
        graceful_timeout: Duration,
        force_kill_wait: Duration,
    ) -> Result<(), AppError> {
        // 同步作用域内取出共享句柄并释放外层锁。锁中毒（他处 panic 持锁）时
        // 恢复现场继续收尾——不得因锁异常而遗留进程条目（检测门控被占）。
        let shared = {
            let mut procs = match self.processes.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            procs.remove(anchor_id).map(|info| info.child)
        };
        let Some(shared) = shared else {
            // 无进程条目：幂等（任务已移除或从未注册）
            return Ok(());
        };
        // 内层锁同样只在同步作用域持有：取出 child 后释放
        let child = {
            let mut guard = match shared.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            guard.take()
        };
        let Some(mut child) = child else {
            // 句柄内已无 child（并发 stop 已取走）——幂等返回
            return Ok(());
        };

        // 优雅退出：stdin 写 'q'（ffmpeg 收到后结束当前段并退出；管道写失败——
        // 进程已死——忽略）。写完释放 stdin 句柄关闭管道。
        if let Some(stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let mut stdin = stdin;
            let _ = stdin.write_all(b"q\n").await;
            let _ = stdin.flush().await;
        }

        let pid = child.id();
        // 等待退出（设上限）。wait() 对已被 try_wait 收割的子进程返回 tokio
        // 缓存的退出状态（不会 panic）。动作统一经 decide_stop_action 决策：
        // 超时 → 强杀；wait 出错 → 不强杀；正常 → 已退出。
        let outcome = tokio::time::timeout(graceful_timeout, child.wait()).await;
        let action = match &outcome {
            Ok(Ok(_)) => decide_stop_action(false, false),
            Ok(Err(_)) => decide_stop_action(false, true),
            Err(_) => decide_stop_action(true, false),
        };
        match (action, outcome) {
            (StopAction::Reaped, Ok(Ok(status))) => {
                tracing::info!(
                    "[录制] FFmpeg 进程已退出 (pid={:?}, status={})",
                    pid,
                    status
                );
            }
            // wait 出错（罕见，句柄状态异常）：不强杀（进程状态未知，强杀可能
            // 误伤；交由 kill_on_drop 兜底）。
            (StopAction::SkipForceKill, Ok(Err(e))) => {
                tracing::warn!(
                    "[录制] 等待 FFmpeg 进程退出失败 (pid={:?}): {}（不强杀，kill_on_drop 兜底）",
                    pid,
                    e
                );
            }
            // 优雅等待超时 → 强制 kill。平台语义：Windows 无 SIGTERM，tokio
            // kill/start_kill = TerminateProcess（硬终止）；Unix = SIGKILL
            //（tokio 不暴露 SIGTERM——'q' 已覆盖优雅路径，剩余悬挂场景多为
            // IO 阻塞，SIGTERM 同样无效）。
            (StopAction::ForceKill, Err(_elapsed)) => {
                tracing::warn!(
                    "[录制] 优雅退出超时（{}ms），强制终止 FFmpeg (pid={:?})",
                    graceful_timeout.as_millis(),
                    pid
                );
                match child.start_kill() {
                    Ok(()) => {
                        // kill 后必须 await 收割（防僵尸）；再设上限兜底 kill 后
                        // wait 仍悬挂的罕见场景，超时则放弃（句柄 drop 由
                        // kill_on_drop 兜底重试）。
                        let _ = tokio::time::timeout(force_kill_wait, child.wait()).await;
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[录制] 强制终止 FFmpeg 失败（kill_on_drop 兜底）: {}",
                            e
                        );
                    }
                }
            }
            // 其余组合不可达（决策函数与等待结果一一对应）；防御性记录，不 panic
            (a, o) => {
                tracing::warn!("[录制] 停止决策与等待结果不一致（action={:?}），忽略", a);
                let _ = o;
            }
        }
        Ok(())
    }
}

/// 校验录制流地址（M6 + M2 对抗审查修复）：仅允许 http/https scheme，且拒绝
/// 回环地址（localhost / 127.0.0.0/8 / ::1）与私网/链路本地字面地址（10.* /
/// 192.168.* / 172.16-31.* / 169.254.* / 100.64.*、IPv6 ULA fc00::/7 与链路
/// 本地 fe80::/10）。域名不做 DNS 解析（避免阻塞检测循环），非字面 IP 的域名
/// 仅做 scheme 校验。
///
/// 绕过防护（M2）：
/// - 数值形态 IP（127.1 / 0x7f000001 / 2130706433 / 0177.0.0.1 等）由
///   url::Host 归一化为 `Ipv4Addr` 后统一按 IPv4 规则校验（url 2.5.8 实测
///   全部解析为 Ipv4，不会落进 Domain 分支）；
/// - IPv4 内嵌 IPv6 变体（IPv4-mapped `::ffff:a.b.c.d`、IPv4-compatible
///   `::a.b.c.d`、NAT64 知名前缀 `64:ff9b::/96`、RFC 6052 IPv4-translated
///   `::ffff:0:0:0/96`）先还原内嵌 IPv4 再按 IPv4 规则校验，防
///   `::ffff:127.0.0.1` 直连回环绕过（IPv6 本身不是 loopback，旧实现放行）；
/// - IPv6 链路本地 fe80::/10 显式拦截（std 无现成方法，手动按前 10 位判定）。
///
/// 额外收紧（不影响合法公网地址）：255.255.255.255 广播与 100.64.0.0/10
/// 运营商级 NAT（阿里云元数据服务 100.100.100.200 所在段）。
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
    // 私网/回环判定（IPv4 与 IPv6 内嵌 IPv4 共用）。is_cgnat 未稳定（Rust
    // 1.96 仍为 nightly API），100.64.0.0/10 手动判定：首字节 100 且次字节
    // 高 2 位为 01
    let is_blocked_v4 = |v4: &std::net::Ipv4Addr| {
        let o = v4.octets();
        let cgnat = o[0] == 100 && (o[1] & 0xc0) == 0x40;
        v4.is_loopback()
            || v4.is_private()
            || v4.is_link_local()
            || v4.is_unspecified()
            || v4.is_broadcast()
            || cgnat
    };
    match host {
        url::Host::Ipv4(v4) => {
            if is_blocked_v4(&v4) {
                return Err(AppError::config("流地址不允许使用回环/私网地址"));
            }
        }
        url::Host::Ipv6(v6) => {
            let octets = v6.octets();
            // 先还原 IPv4 内嵌变体，再按 IPv4 规则校验——否则 `::ffff:127.0.0.1`
            // 等会绕过 IPv4 检查（IPv6 视角下它既非 loopback 也非 ULA）
            let embedded_v4 = v6
                .to_ipv4_mapped()
                .or_else(|| v6.to_ipv4())
                // NAT64 知名前缀 64:ff9b::/96：内嵌 IPv4 为最后 32 位
                .or_else(|| {
                    (octets[0..4] == [0x00, 0x64, 0xff, 0x9b] && octets[8..12] == [0; 4]).then(
                        || std::net::Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]),
                    )
                })
                // RFC 6052 IPv4-translated ::ffff:0:0:0/96：内嵌 IPv4 为最后 32 位
                .or_else(|| {
                    (octets[0..8] == [0; 8]
                        && octets[8..10] == [0xff, 0xff]
                        && octets[10..12] == [0; 2])
                    .then(|| {
                        std::net::Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15])
                    })
                });
            if let Some(v4) = embedded_v4 {
                if is_blocked_v4(&v4) {
                    return Err(AppError::config("流地址不允许使用回环/私网地址"));
                }
            }
            // IPv6 原生特殊段：回环 ::1 / 未指定 :: / ULA fc00::/7 / 链路本地
            // fe80::/10（前 10 位 1111111010）
            let link_local = octets[0] == 0xfe && (octets[1] & 0xc0) == 0x80;
            if v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || link_local
            {
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
///
/// 跨平台说明（P1-11）：保持单一声明不做平台分支——Windows 字符集消毒在
/// Linux 上无害（Linux 仅 `/` 与 NUL 为非法字符，消毒结果必然合法且更保守），
/// 避免双平台行为分叉（同主播名在两平台产出不同目录名）；`:`/`\` 在 Linux
/// 上被替换属过度消毒，可接受。
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
/// best-effort：检查与 ffmpeg 实际创建之间仍有极小竞态窗口；默认模板已不含
/// 录制序号（`{index}` 已移除），常规「同秒重录 / 上次残留」与跨主播同秒同名
/// 均由本函数兜底。
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
/// `{anchor_name}/{date}_{time}_{anchor_name}.{ext}` → 子目录
/// `主播A/` + 文件名 `2026-08-07_12-30-45_主播A.m4a`）；本函数只做三段收尾：
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

// ── H3/H5：孤儿 ffmpeg 产物启动清理 + 录制半成品命名规则 ─────────────────────
//
// 背景（docs/audit/02-静态长值守审查.md F-11/G5）：应用被强杀（kill -9 / 断电 /
// Windows 更新自动重启）时，kill_on_drop 不触发——已 spawn 的 ffmpeg 子进程
// 成为孤儿继续向原输出文件写入；重启后若无清理，新旧实例可能双写同一文件
// （数据损坏/双录），或留下半成品文件永久滞留。
//
// 策略：ffmpeg 即将写入前创建「活动录制标记」`{output_path}.recording`（内容含
// 分段开关与格式——启动清理据此精确还原本次录制可能产生的全部产物）；正常收尾
// 由 monitor.rs 统一出口移除。启动时（录制调度器初始化前，单实例锁已持有，见
// lib.rs setup）扫描输出目录：
//   - 残留 `.recording` 标记 → 上次异常退出 → 按标记内容清理对应产物并删除标记；
//   - 无标记但存在 `.part` 半成品（用户手工改名等）→ 仅记录告警，**不删除**
//     （无规则可循时绝不误删用户文件，与审查建议一致）；
//   - 其余文件一律不动。
// 清理失败静默（仅 warn），不阻断启动。

/// 录制活动标记文件名后缀：`{output_path}.recording`（文件系统保留字段外的
/// 普通后缀；与可能存在的同名音频文件互不冲突——录音文件扩展名固定为 m4a/mp3）。
pub const RECORDING_MARKER_SUFFIX: &str = ".recording";
/// 录制半成品文件后缀：`{base}.part`（崩溃产物重命名标记；同时是启动清理可识别
/// 的半成品规则）。
pub const PARTIAL_SUFFIX: &str = ".part";

/// 标记内容（serde 序列化；写入失败仅 warn，不阻断录制）：
/// 记录输出路径与产物生成规则，启动清理据此还原本次录制可能产生的全部文件，
/// 避免「只删主文件、留下 ffmpeg 分段残留」。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct RecordingMarker {
    /// 主输出路径（模板渲染 + 去重后的最终路径）
    output: String,
    /// 分段模式（segment_seconds > 0）：ffmpeg 按 `{base}_%03d.{ext}` pattern 写段
    segmented: bool,
    /// 录制格式扩展名（m4a / mp3）
    ext: String,
}

/// 写入活动录制标记（`{output}.recording`，内容含产物范围签名）。
fn write_recording_marker(output_path: &str, segment_seconds: u64, ext: &str) {
    let marker = RecordingMarker {
        output: output_path.to_string(),
        segmented: segment_seconds > 0,
        ext: ext.to_string(),
    };
    let path = format!("{output_path}{RECORDING_MARKER_SUFFIX}");
    let Ok(text) = serde_json::to_string(&marker) else {
        return;
    };
    match std::fs::write(&path, text) {
        Ok(_) => tracing::debug!("[录制] 已创建活动标记: {}", path),
        Err(e) => tracing::warn!("[录制] 创建活动标记失败（启动清理将无法识别本次产物）: {}: {}", path, e),
    }
}

/// 移除活动录制标记（幂等；正常收尾与录制未真正开始的失败路径共用）。
pub fn remove_recording_marker(output_path: &str) {
    let marker_path = format!("{output_path}{RECORDING_MARKER_SUFFIX}");
    let path = std::path::Path::new(&marker_path);
    match std::fs::remove_file(path) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!("[录制] 移除活动标记失败: {}: {}", path.display(), e),
    }
}

/// 清理单个文件条目（H3/H5 共用；S1 兜底）：
/// - 符号链接 / junction 一律跳过（不跟随链接，杜绝触碰输出目录外文件）；
/// - 删除失败仅 warn（不阻断）；返回是否已删除。
fn remove_file_entry(path: &std::path::Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if meta.file_type().is_symlink() {
        tracing::warn!("[孤儿清理] 跳过符号链接/junction 项（不跟随链接）: {}", path.display());
        return false;
    }
    match std::fs::remove_file(path) {
        Ok(_) => {
            tracing::info!("[孤儿清理] 已删除: {}", path.display());
            true
        }
        Err(e) => {
            tracing::warn!("[孤儿清理] 删除失败（跳过）: {}: {}", path.display(), e);
            false
        }
    }
}

/// 计算本次录制（非分段）可能产生的产物路径：
/// - 主输出文件（存在才产出；可能已被孤儿 ffmpeg 写出一部分）；
/// - 同名半成品 `{output}.part`（H5 崩溃产物重命名规则，崩溃改名/删除的目标）。
/// 仅收集**实际存在**的文件，其余一律不动。
fn collect_non_segmented_products(output: &str, candidates: &mut Vec<std::path::PathBuf>) {
    let output_path = std::path::Path::new(output);
    let Some(parent) = output_path.parent() else {
        return;
    };
    let Some(file_name) = output_path.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    // 主输出文件（可能已被孤儿 ffmpeg 写出一部分）
    if output_path.exists() {
        candidates.push(output_path.to_path_buf());
    }
    // 同名半成品 `{output}.part`（H5 崩溃产物重命名规则）
    let self_partial = parent.join(format!("{file_name}{PARTIAL_SUFFIX}"));
    if self_partial.exists() {
        candidates.push(self_partial);
    }
}

/// 计算本次录制（分段模式）可能产生的产物路径：
/// 分段录制按 `{base}_%03d.{ext}` pattern 写文件（builder.rs），崩溃时可能残留
/// 已写出的若干段。无命名规则可精确圈定段范围，仅收集「文件名 = `{输出文件名}_`
/// + 纯数字序号 + `.{ext}`（或崩溃改名后的 `.{ext}.part`）」的段文件——精确匹配
/// 前缀，不误删同前缀用户文件。
fn collect_segmented_products(
    output: &str,
    ext: &str,
    candidates: &mut Vec<std::path::PathBuf>,
) {
    let output_path = std::path::Path::new(output);
    let Some(parent) = output_path.parent() else {
        return;
    };
    let Some(file_name) = output_path.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let pattern_prefix = format!("{file_name}_");
    let ext_suffix = format!(".{ext}");
    let ext_part_suffix = format!(".{ext}{PARTIAL_SUFFIX}");
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.file_type().is_file() || meta.file_type().is_symlink() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with(&pattern_prefix) {
            continue;
        }
        // 段文件名 = `{base}_{NNN}.{ext}`，或 H5 崩溃改名后的 `{base}_{NNN}.{ext}.part`
        let seq_ok = name
            .strip_prefix(&pattern_prefix)
            .and_then(|s| {
                s.strip_suffix(&ext_suffix)
                    .or_else(|| s.strip_suffix(&ext_part_suffix))
            })
            .is_some_and(|seq| !seq.is_empty() && seq.bytes().all(|b| b.is_ascii_digit()));
        if seq_ok {
            candidates.push(path);
        }
    }
}

/// 清理单个录制标记对应的残留产物（H3/H5 共用）：
/// 按标记内容精确匹配候选（分段 / 非分段），删除全部命中项；随后删除标记文件。
/// 返回删除的文件数（测试断言用；生产路径忽略）。删除失败仅 warn，不 panic。
fn cleanup_marker_products(marker: &RecordingMarker, marker_path: &std::path::Path) -> usize {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if marker.segmented {
        collect_segmented_products(&marker.output, &marker.ext, &mut candidates);
    } else {
        collect_non_segmented_products(&marker.output, &mut candidates);
    }
    let mut removed = 0usize;
    for path in &candidates {
        if remove_file_entry(path) {
            removed += 1;
        }
    }
    // 最后删除标记本身（产物清理完毕后再删——启动时若中途失败，标记仍在，
    // 下次启动可继续清理；且残留标记在录制开始前会被下面的 start 清理覆盖）
    if remove_file_entry(marker_path) {
        removed += 1;
    }
    removed
}

/// 启动时清理上次异常退出的孤儿 ffmpeg 产物（H3）：
/// 递归扫描输出目录（不跟随链接），对每个 `.recording` 标记按内容清理对应产物；
/// 无标记但存在 `.part` 文件（无规则可循）→ 仅记录告警，**不删除**。
/// 目录不存在 / 读取失败 → 静默成功（幂等，不阻断启动）。
/// 返回（删除文件数, 清理的标记数, 仅告警的半成品数）；测试断言用，生产路径忽略。
pub fn cleanup_orphan_recordings(
    output_dir: &str,
    root: &std::path::Path,
) -> (usize, usize, usize) {
    let Ok(_entries) = std::fs::read_dir(root) else {
        return (0, 0, 0);
    };
    let mut removed = 0usize;
    let mut markers_cleaned = 0usize;
    let mut warnings = 0usize;
    // 输出目录前缀（归一化 '/'；用于标记内容校验与相对路径还原）
    let dir_prefix = output_dir.trim_end_matches(['/', '\\']).replace('\\', "/");
    // 递归栈：目录（symlink_metadata 校验，不跟随链接）
    let mut dir_stack: Vec<std::path::PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = dir_stack.pop() {
        let Ok(dir_entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in dir_entries.flatten() {
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            let ft = meta.file_type();
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                dir_stack.push(path);
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if let Some(stripped) = name.strip_suffix(RECORDING_MARKER_SUFFIX) {
                // 标记对应的 output 完整路径 = 输出目录 + 相对子路径 + 标记文件名
                //（去掉 .recording 后缀；与引擎构造的 output 路径一致）
                let rel = dir
                    .strip_prefix(root)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                let marker_output = if rel.is_empty() {
                    format!("{}/{}", dir_prefix, stripped)
                } else {
                    format!("{}/{}/{}", dir_prefix, rel, stripped)
                };
                // 读取标记内容（普通文件校验：symlink_metadata 已确认是文件且
                // 非链接）；内容损坏/不一致仅告警不删除
                let Ok(text) = std::fs::read_to_string(&path) else {
                    tracing::warn!(
                        "[孤儿清理] 发现无法读取的录制标记（跳过，不删除）: {}",
                        path.display()
                    );
                    warnings += 1;
                    continue;
                };
                let Ok(parsed) = serde_json::from_str::<RecordingMarker>(&text) else {
                    tracing::warn!(
                        "[孤儿清理] 发现无法解析的录制标记（跳过，不删除）: {}",
                        path.display()
                    );
                    warnings += 1;
                    continue;
                };
                // 内容校验：标记内的 output 必须与所在位置一致且落在输出目录内
                //（防内容伪造/错位标记指向目录外文件）
                let norm = parsed.output.replace('\\', "/");
                if norm != marker_output.replace('\\', "/") || !norm.starts_with(&dir_prefix) {
                    tracing::warn!(
                        "[孤儿清理] 录制标记内容与所在位置不一致（跳过，不删除）: {}",
                        path.display()
                    );
                    warnings += 1;
                    continue;
                }
                tracing::info!(
                    "[孤儿清理] 检测到上次异常退出的录制标记: {}（segmented={}），清理残留产物",
                    marker_output,
                    parsed.segmented
                );
                removed += cleanup_marker_products(&parsed, &path);
                markers_cleaned += 1;
                continue;
            }
            // 无标记上下文：`.part` 文件可能是崩溃残留（H5 改名规则）或用户自己的
            // 文件——仅记录告警，绝不删除（无规则可循时不误删用户文件）。
            // 若该 .part 已有对应的 `.recording` 标记覆盖（崩溃残留路径：标记
            // 保留至下次启动清理），由标记清理处理，不重复告警。
            if name.ends_with(PARTIAL_SUFFIX) {
                let covered_by_marker = name
                    .strip_suffix(PARTIAL_SUFFIX)
                    .map(|stem| {
                        path.with_file_name(format!("{stem}{RECORDING_MARKER_SUFFIX}"))
                            .exists()
                    })
                    .unwrap_or(false);
                if !covered_by_marker {
                    tracing::warn!(
                        "[孤儿清理] 发现无标记的半成品文件（仅告警，不删除——无规则可循）: {}",
                        path.display()
                    );
                    warnings += 1;
                }
                continue;
            }
        }
    }
    if removed > 0 || markers_cleaned > 0 {
        tracing::info!(
            "[孤儿清理] 启动清理完成: 删除 {} 个文件 / 清理 {} 个录制标记 / 仅告警 {} 个",
            removed,
            markers_cleaned,
            warnings
        );
    } else if warnings > 0 {
        tracing::warn!(
            "[孤儿清理] 启动清理完成: 仅发现 {} 个无标记半成品（已告警，未删除）",
            warnings
        );
    }
    (removed, markers_cleaned, warnings)
}

// ── R4：孤儿 ffmpeg 进程终止（占位）──────────────────────────────────────────
//
// 背景（docs/audit/04-修复复核审查.md R4 / L1）：H3 启动清理只处理**产物文件**
//（{output}.recording 标记对应的残留文件），不终止**孤儿 ffmpeg 进程本身**。
// Windows 上孤儿进程持有输出文件句柄时 remove_file 触发共享冲突 → 跳过+告警，
// 新录制走 `_2` 后缀——数据不再双写同一文件（防损坏目标达成，见 04 报告），
// 但孤儿进程继续占盘写入直至流结束。
//
// 实现方案评估（选型：**占位不实装**，理由见下）：
// - Windows Job Object（推荐方案）：启动时创建带 JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
//   的 Job 对象，ffmpeg spawn 后 AssignProcessToJobObject——应用进程被强杀时
//   系统随 Job 句柄关闭终止 Job 内全部进程，无枚举/误杀面。需要 windows crate
//   新增 Win32_System_JobObjects / Win32_System_Threading feature（改 Cargo.toml），
//   且 Job 只覆盖「本实例 spawn 的子进程」——旧版本实例/用户手工启动的同路径
//   ffmpeg 不受控。
// - 按命令行匹配枚举（Windows：WMI/CIM Win32_Process CommandLine；Linux：
//   /proc/<pid>/cmdline）：启动时（单实例锁已持有、录制未开始）终止命令行含
//   当前输出路径的 ffmpeg。实现成本低（std::process::Command 调系统工具 + 字符串
//   匹配），但有**误杀面**：用户自己手工运行的同路径 ffmpeg 会被杀掉；Windows
//   依赖 PowerShell/CIM 系统组件，与组 C/3「通知不依赖 PowerShell」的工程纪律
//   冲突；匹配逻辑（路径转义/大小写/相对路径归一化）易出边界 bug。
// - Linux PDEATHSIG：需子进程配合 prctl(PR_SET_PDEATHSIG)（libc，新增依赖），
//   对 ffmpeg 这类外部可执行文件无法注入——不适用。
//
// 决策：**占位**。R4 属「提示」级、非回归（数据损坏风险已由 H3 产物清理 + 输出
// 路径去重消除，剩余是资源占用残留）；三个平台方案均需新增依赖/系统组件依赖或
// 有误杀面，收益（占盘回收）与成本/风险不成比例。占位函数保留完整签名与启动
// 路径接线，后续在独立迭代中按上述方案之一实装。
/// 终止上次异常退出的孤儿 ffmpeg 进程（R4，**占位实现**：当前不做事）。
///
/// 实装时应在启动清理 `cleanup_orphan_recordings` 之后、检测循环启动之前调用
///（单实例锁已持有、无并发录制，命令行匹配不会误杀本实例进程）。平台方案与
/// 限制见上方模块注释（Windows Job Object / 命令行匹配；Linux /proc cmdline；
/// PDEATHSIG 不适用）。不 panic，失败静默（与 H3 启动清理同语义，不阻断启动）。
pub fn terminate_orphan_ffmpeg() {
    // TODO(R4)：孤儿 ffmpeg 进程终止未实装（见上方模块注释的方案评估与决策）。
    // 实装要点：
    //   1. Windows：优先 Job Object（KILL_ON_JOB_CLOSE）——需在 Cargo.toml 的
    //      windows 依赖追加 Win32_System_JobObjects / Win32_System_Threading
    //      feature，并在 FfmpegRecorder::insert_process 处 AssignProcessToJobObject；
    //      或降级方案：枚举 Win32_Process CommandLine 匹配输出路径后 TerminateProcess
    //      （注意只匹配本应用 ffmpeg，避免误杀用户手工进程）。
    //   2. Linux：枚举 /proc/<pid>/cmdline 匹配输出路径后 kill（SIGTERM→SIGKILL）；
    //      无 libc 依赖（std::fs 读 /proc）。
    //   3. 匹配前先归一化路径（大小写/分隔符/相对绝对），输出目录参数需传入。
}

/// 录制异常退出（REC_CRASH）路径的半成品处置（H5）：
/// 崩溃产生的半成品文件（无 `-y` 覆盖保护、未写入完整元数据的 `{output}` /
/// `{output}.part` / 分段残留）**不随用户 auto_cleanup 语义保留**——auto_cleanup
/// 控制的是「录制结束是否执行保留期/总量清理」（cleanup.rs），与「崩溃残留垃圾」
/// 无关。此函数把本次崩溃产物改名标记为 `.part`（改名失败时直接删除），并记录
/// 告警日志告知用户；**保留活动录制标记**（不删除）——下次启动
/// （cleanup_orphan_recordings，H3）据此识别并清理 `.part` 残留。
/// 绝不触碰其他文件（不误删用户数据）。返回删除/改名的文件数。
pub fn mark_crash_partials(output_path: &str, segmented: bool, ext: &str) -> usize {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if segmented {
        collect_segmented_products(output_path, ext, &mut candidates);
    } else {
        collect_non_segmented_products(output_path, &mut candidates);
    }
    let mut handled = 0usize;
    for path in candidates {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // 已经带 .part 后缀（旧崩溃残留的再次崩溃）→ 直接删除
        if name.ends_with(PARTIAL_SUFFIX) {
            if remove_file_entry(&path) {
                handled += 1;
            }
            continue;
        }
        let new_path = path.with_file_name(format!("{name}{PARTIAL_SUFFIX}"));
        match std::fs::rename(&path, &new_path) {
            Ok(_) => {
                tracing::warn!(
                    "[录制] 录制崩溃产物已改名标记（.part，稍后自动清理）: {} → {}",
                    path.display(),
                    new_path.display()
                );
                handled += 1;
            }
            Err(e) => {
                tracing::warn!(
                    "[录制] 崩溃产物改名失败，直接删除: {}: {}",
                    path.display(),
                    e
                );
                if remove_file_entry(&path) {
                    handled += 1;
                }
            }
        }
    }
    // 保留活动录制标记：下次启动（cleanup_orphan_recordings，H3）据此识别并
    // 清理改名后的 .part 残留（若在此删除，启动清理只能告警、无法自动回收）
    if handled > 0 {
        tracing::warn!(
            "[录制] 崩溃半成品处置完成：{} 个文件已改名标记（.part）或删除；auto_cleanup 语义不适用于崩溃残留，用户可手动删除 .part 文件，下次启动将自动清理",
            handled
        );
    }
    handled
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
        // B2：优雅终止 → 等待上限（8s）→ 超时强制 kill → 收割；超时参数生产值
        // 见文件顶部常量（测试经 stop_inner 注入毫秒级值验证强制 kill 路径）。
        self.stop_inner(anchor_id, STOP_GRACEFUL_TIMEOUT, STOP_FORCE_KILL_WAIT)
            .await
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

    // ── M2：SSRF 绕过向量（数值形态 IP / IPv6 内嵌 IPv4 / 链路本地）──

    #[test]
    fn validate_stream_url_rejects_ipv4_obfuscations() {
        // 数值/十六进制/八进制形态 IP：url 归一化为 Ipv4 后必须按 IPv4 规则拦截
        for bad in [
            "http://127.1/live",
            "http://127.0.0.2/live",
            "http://0x7f000001/live",
            "http://0x7f.0.0.1/live",
            "http://2130706433/live",
            "http://0177.0.0.1/live",
            "http://127.0.0.1./live",
            "http://2130706434/live", // 127.0.0.2
            "http://169.254.0.1/live",
            "http://100.100.100.200/live", // CGNAT（阿里云元数据服务所在段）
            "http://255.255.255.255/live", // 广播地址
        ] {
            assert!(validate_stream_url(bad).is_err(), "应拒绝: {}", bad);
        }
    }

    #[test]
    fn validate_stream_url_rejects_ipv6_embedded_and_special() {
        for bad in [
            // IPv4-mapped IPv6：IPv6 视角既非 loopback 也非 ULA，旧实现会放行
            "http://[::ffff:127.0.0.1]/live",
            "http://[::ffff:7f00:1]/live",
            // RFC 6052 IPv4-translated ::ffff:0:0:0/96
            "http://[::ffff:0:127.0.0.1]/live",
            // IPv4-compatible ::a.b.c.d
            "http://[::127.0.0.1]/live",
            // NAT64 知名前缀 64:ff9b::/96 内嵌回环
            "http://[64:ff9b::7f00:1]/live",
            // IPv6 原生特殊段
            "http://[::1]/live",
            "http://[::]/live",
            "http://[fe80::1]/live",
            "http://[fe80::dead:beef::1]/live",
            "http://[fdb8:85a3::1]/live", // ULA fc00::/7
        ] {
            assert!(validate_stream_url(bad).is_err(), "应拒绝: {}", bad);
        }
    }

    #[test]
    fn validate_stream_url_still_accepts_public_addresses() {
        // M2 不得误伤合法公网流地址：IPv4 / IPv6 / IPv4-mapped 公网 IPv4 /
        // NAT64 内嵌公网 IPv4
        for good in [
            "https://stream.example.com/live.flv",
            "http://stream.example.com/live",
            "https://8.8.8.8/live",
            "http://114.114.114.114/live",
            "https://100.63.0.1/live",      // CGNAT 段外（100.64/10 之前）
            "http://[2001:4860:4860::8888]/live", // 公网 IPv6
            "http://[::ffff:8.8.8.8]/live",  // IPv4-mapped 的公网 IPv4
            "http://[64:ff9b::808:808]/live", // NAT64 内嵌公网 IPv4（8.8.8.8）
        ] {
            assert!(validate_stream_url(good).is_ok(), "应放行: {}", good);
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

    // ── B1：进程存活探测 / 异常退出判定 ──

    /// 获取一个真实「已退出」状态的 ExitStatus（ExitStatus 无公开构造器，用
    /// 真实子进程收割获得；仅作为纯逻辑判定的状态载体）
    fn exited_status() -> std::process::ExitStatus {
        #[cfg(windows)]
        let status = std::process::Command::new("cmd")
            .args(["/c", "exit"])
            .status()
            .expect("获取退出状态失败");
        #[cfg(not(windows))]
        let status = std::process::Command::new("true")
            .status()
            .expect("获取退出状态失败");
        status
    }

    #[test]
    fn abnormal_exit_requires_exited_and_no_cancel() {
        let exited = ChildProbe::Exited(exited_status());
        assert!(
            is_abnormal_exit(exited, false),
            "已退出且取消未触发 → 异常退出"
        );
        assert!(
            !is_abnormal_exit(exited, true),
            "已退出但取消已触发（停止流程正在终止）→ 正常停止，不得误判崩溃"
        );
        assert!(!is_abnormal_exit(ChildProbe::Running, false));
        assert!(!is_abnormal_exit(ChildProbe::Unknown, false));
    }

    #[test]
    fn stop_action_decision_matrix() {
        // 优雅等待超时 → 强制 kill（超时优先于 wait 出错）
        assert_eq!(decide_stop_action(true, false), StopAction::ForceKill);
        assert_eq!(decide_stop_action(true, true), StopAction::ForceKill);
        // wait 出错（状态未知）→ 不强杀
        assert_eq!(decide_stop_action(false, true), StopAction::SkipForceKill);
        // 正常退出 → 无需强杀
        assert_eq!(decide_stop_action(false, false), StopAction::Reaped);
    }

    #[tokio::test]
    async fn probe_detects_exited_child_and_removal() {
        let recorder = FfmpegRecorder::new();
        let child = spawn_fake_child().await;
        recorder.insert_process("a".into(), child).await.unwrap();
        // 轮询：真实子进程退出需要一点时间（tokio try_wait 收割）
        let mut exited = false;
        for _ in 0..100 {
            if matches!(recorder.probe_process("a"), ChildProbe::Exited(_)) {
                exited = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(exited, "立即退出的假子进程应被探测到已退出");
        // 停止后进程条目移除 → Unknown（检测门控随之释放）
        recorder.stop("a").await.unwrap();
        assert_eq!(recorder.probe_process("a"), ChildProbe::Unknown);
        assert!(!recorder.is_recording("a"));
    }

    #[tokio::test]
    async fn probe_reports_running_for_live_child() {
        let recorder = FfmpegRecorder::new();
        let child = spawn_stuck_child().await;
        recorder.insert_process("a".into(), child).await.unwrap();
        assert_eq!(recorder.probe_process("a"), ChildProbe::Running);
        // 清理：毫秒级注入超时强制终止（生产值 8s 会拖慢测试）
        recorder
            .stop_inner("a", Duration::from_millis(100), Duration::from_secs(3))
            .await
            .unwrap();
        assert_eq!(recorder.probe_process("a"), ChildProbe::Unknown);
    }

    // ── B2：停止流程（优雅 'q' → 超时强杀 → 收割）──

    /// 长时间运行且不读 stdin 的假子进程（验证强制 kill 路径；Windows 下
    /// ping.exe / Unix 下 sleep 均为单进程，强杀不留孤儿）
    async fn spawn_stuck_child() -> Child {
        #[cfg(windows)]
        let mut cmd = tokio::process::Command::new("ping");
        #[cfg(windows)]
        cmd.args(["-n", "30", "127.0.0.1"]);
        #[cfg(not(windows))]
        let mut cmd = tokio::process::Command::new("sleep");
        #[cfg(not(windows))]
        cmd.arg("30");
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn 卡死测试子进程失败")
    }

    /// 读 stdin 到 EOF 即退出的假子进程（验证优雅路径；Windows 下 findstr /
    /// Unix 下 cat 均为单进程 exe，EOF 触发退出，确定性高；不用 more.com——
    /// CreateProcess 不经 PATHEXT 解析 .com，直接 NotFound）
    async fn spawn_stdin_eof_child() -> Child {
        #[cfg(windows)]
        let mut cmd = tokio::process::Command::new("findstr");
        #[cfg(windows)]
        cmd.arg(".");
        #[cfg(not(windows))]
        let mut cmd = tokio::process::Command::new("cat");
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn 优雅退出测试子进程失败")
    }

    #[tokio::test]
    async fn stop_graceful_path_exits_on_q() {
        let recorder = FfmpegRecorder::new();
        let child = spawn_stdin_eof_child().await;
        recorder.insert_process("a".into(), child).await.unwrap();
        assert!(recorder.is_recording("a"));
        let t0 = std::time::Instant::now();
        recorder.stop("a").await.unwrap();
        let elapsed = t0.elapsed();
        assert!(!recorder.is_recording("a"), "停止后进程条目必须移除");
        assert_eq!(recorder.probe_process("a"), ChildProbe::Unknown);
        assert!(
            elapsed < Duration::from_secs(6),
            "优雅 'q' 退出应远快于 8s 超时上限（无需强杀），实际 {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn stop_force_kills_stuck_child() {
        let recorder = FfmpegRecorder::new();
        let child = spawn_stuck_child().await;
        recorder.insert_process("a".into(), child).await.unwrap();
        let t0 = std::time::Instant::now();
        // 毫秒级优雅上限注入（生产值 8s）：验证「超时 → 强制 kill → 收割」路径
        recorder
            .stop_inner("a", Duration::from_millis(200), Duration::from_secs(3))
            .await
            .unwrap();
        let elapsed = t0.elapsed();
        assert!(!recorder.is_recording("a"), "强制终止后进程条目必须移除");
        assert_eq!(recorder.probe_process("a"), ChildProbe::Unknown);
        // 应确实等待了优雅上限（≥200ms）才强杀（非误杀），且总时长有界
        //（远小于子进程 30s 自然寿命）
        assert!(
            elapsed >= Duration::from_millis(150),
            "应先等待优雅上限再强杀，实际 {:?}",
            elapsed
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "强杀路径应快速完成，实际 {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn force_terminate_all_clears_all_entries() {
        let recorder = FfmpegRecorder::new();
        // 用立即退出的假子进程（优雅路径快速完成，避免 8s×N 拖慢测试）
        for id in ["a", "b"] {
            recorder
                .insert_process(id.into(), spawn_fake_child().await)
                .await
                .unwrap();
        }
        assert_eq!(recorder.active_anchor_ids().len(), 2);
        recorder.force_terminate_all().await;
        assert!(
            recorder.active_anchor_ids().is_empty(),
            "退出兜底应终止并移除全部进程条目"
        );
        assert!(!recorder.is_recording("a"));
        assert!(!recorder.is_recording("b"));
        // 幂等：重复 stop 不报错、不 panic
        recorder.stop("a").await.unwrap();
        recorder.stop("b").await.unwrap();
    }

    // ── H3/H5：孤儿 ffmpeg 产物启动清理 / 崩溃半成品处置 ──

    /// 唯一临时输出目录（并行测试隔离）
    fn orphan_test_dir(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "missevan-orphan-{}-{}-{}",
            tag,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ))
    }

    #[test]
    fn startup_cleanup_removes_marker_products_only() {
        let dir = orphan_test_dir("h3");
        let _ = std::fs::remove_dir_all(&dir);
        let out_dir = dir.join("主播A");
        std::fs::create_dir_all(&out_dir).unwrap();
        // 孤儿 ffmpeg 残留：半成品主文件 + 活动标记（模拟强杀后重启）
        let output = out_dir.join("2026-08-07_12-30-45_主播A_001.m4a");
        std::fs::write(&output, b"partial").unwrap();
        let marker_path = out_dir.join("2026-08-07_12-30-45_主播A_001.m4a.recording");
        let marker = RecordingMarker {
            output: output.to_string_lossy().into_owned(),
            segmented: false,
            ext: "m4a".into(),
        };
        std::fs::write(&marker_path, serde_json::to_string(&marker).unwrap()).unwrap();
        // 无关文件：正常完成的录制 + 用户文件（必须保留）
        let done = out_dir.join("2026-08-07_09-00-00_主播A_002.m4a");
        std::fs::write(&done, b"done").unwrap();
        let notes = dir.join("notes.txt");
        std::fs::write(&notes, b"user").unwrap();

        let (removed, markers, warned) = cleanup_orphan_recordings(&dir.to_string_lossy(), &dir);
        assert_eq!(removed, 2, "应删除半成品 + 标记");
        assert_eq!(markers, 1, "应清理 1 个标记");
        assert_eq!(warned, 0);
        assert!(!output.exists(), "半成品必须删除");
        assert!(!marker_path.exists(), "标记必须删除");
        assert!(done.exists(), "正常录制不得删除");
        assert!(notes.exists(), "用户文件不得删除");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn startup_cleanup_warns_but_keeps_orphan_partial() {
        // 无标记上下文的 .part：无规则可循 → 仅告警，绝不删除（用户文件保护）
        let dir = orphan_test_dir("h3-warn");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let partial = dir.join("foo.m4a.part");
        std::fs::write(&partial, b"x").unwrap();

        let (removed, markers, warned) = cleanup_orphan_recordings(&dir.to_string_lossy(), &dir);
        assert_eq!(removed, 0);
        assert_eq!(markers, 0);
        assert_eq!(warned, 1, "无标记半成品应记录告警");
        assert!(partial.exists(), "无标记半成品不得删除");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn startup_cleanup_segmented_marker_removes_segments_only() {
        let dir = orphan_test_dir("h3-seg");
        let _ = std::fs::remove_dir_all(&dir);
        let out_dir = dir.join("主播A");
        std::fs::create_dir_all(&out_dir).unwrap();
        let base = out_dir.join("2026-08-07_12-30-45_主播A_001");
        // 崩溃残留：已写出的段 + H5 已改名标记的段（.m4a.part）
        std::fs::write(out_dir.join("2026-08-07_12-30-45_主播A_001_000.m4a"), b"s0").unwrap();
        std::fs::write(out_dir.join("2026-08-07_12-30-45_主播A_001_001.m4a.part"), b"s1")
            .unwrap();
        // 无关：同前缀非纯数字序号、其他主播的段（必须保留）
        std::fs::write(out_dir.join("2026-08-07_12-30-45_主播A_001_extra.m4a"), b"x").unwrap();
        std::fs::write(out_dir.join("2026-08-07_12-30-45_主播B_001_000.m4a"), b"x").unwrap();
        let marker = RecordingMarker {
            output: base.to_string_lossy().into_owned(),
            segmented: true,
            ext: "m4a".into(),
        };
        std::fs::write(
            out_dir.join("2026-08-07_12-30-45_主播A_001.recording"),
            serde_json::to_string(&marker).unwrap(),
        )
        .unwrap();

        let (removed, markers, warned) = cleanup_orphan_recordings(&dir.to_string_lossy(), &dir);
        assert_eq!(markers, 1);
        assert_eq!(removed, 3, "应删除 2 段 + 标记");
        assert_eq!(warned, 0);
        assert!(!out_dir.join("2026-08-07_12-30-45_主播A_001_000.m4a").exists());
        assert!(!out_dir.join("2026-08-07_12-30-45_主播A_001_001.m4a.part").exists());
        assert!(out_dir.join("2026-08-07_12-30-45_主播A_001_extra.m4a").exists());
        assert!(out_dir.join("2026-08-07_12-30-45_主播B_001_000.m4a").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn startup_cleanup_skips_mismatched_marker() {
        // 标记内容与所在位置不一致（伪造/错位）→ 跳过不删除（数据保护）
        let dir = orphan_test_dir("h3-bad");
        let _ = std::fs::remove_dir_all(&dir);
        let out_dir = dir.join("主播A");
        std::fs::create_dir_all(&out_dir).unwrap();
        let target = out_dir.join("evil.m4a");
        std::fs::write(&target, b"user-data").unwrap();
        let marker = RecordingMarker {
            output: dir.join("其他目录").join("evil.m4a").to_string_lossy().into_owned(),
            segmented: false,
            ext: "m4a".into(),
        };
        std::fs::write(
            out_dir.join("evil.m4a.recording"),
            serde_json::to_string(&marker).unwrap(),
        )
        .unwrap();

        let (removed, markers, warned) = cleanup_orphan_recordings(&dir.to_string_lossy(), &dir);
        assert_eq!(removed, 0, "不一致标记不得删除任何文件");
        assert_eq!(markers, 0);
        assert!(warned >= 1, "应记录告警");
        assert!(target.exists(), "用户文件必须保留");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn startup_cleanup_missing_dir_is_noop() {
        let dir = orphan_test_dir("h3-missing");
        let _ = std::fs::remove_dir_all(&dir);
        let (removed, markers, warned) = cleanup_orphan_recordings(&dir.to_string_lossy(), &dir);
        assert_eq!((removed, markers, warned), (0, 0, 0), "目录不存在应静默无操作");
    }

    #[test]
    fn crash_partials_rename_or_delete_products() {
        // H5 非分段：崩溃主文件改名为 .part（内容保留），活动标记**保留**（供下次
        // 启动清理识别），无关文件不动
        let dir = orphan_test_dir("h5");
        let _ = std::fs::remove_dir_all(&dir);
        let out_dir = dir.join("主播A");
        std::fs::create_dir_all(&out_dir).unwrap();
        let output = out_dir.join("2026-08-07_12-30-45_主播A_001.m4a");
        std::fs::write(&output, b"partial").unwrap();
        let marker_path = out_dir.join("2026-08-07_12-30-45_主播A_001.m4a.recording");
        let marker = RecordingMarker {
            output: output.to_string_lossy().into_owned(),
            segmented: false,
            ext: "m4a".into(),
        };
        std::fs::write(&marker_path, serde_json::to_string(&marker).unwrap()).unwrap();
        let unrelated = out_dir.join("2026-08-07_09-00-00_主播A_002.m4a");
        std::fs::write(&unrelated, b"done").unwrap();

        let handled = mark_crash_partials(&output.to_string_lossy(), false, "m4a");
        assert_eq!(handled, 1);
        let renamed = out_dir.join("2026-08-07_12-30-45_主播A_001.m4a.part");
        assert!(!output.exists(), "主文件应被改名");
        assert!(renamed.exists(), "崩溃产物应改名为 .part");
        assert_eq!(std::fs::read(&renamed).unwrap(), b"partial", "内容必须保留");
        assert!(marker_path.exists(), "崩溃收尾应保留活动标记（下次启动清理用）");
        assert!(unrelated.exists(), "无关文件不得触碰");

        // 模拟下次启动：cleanup_orphan_recordings 识别标记 → 清理 .part + 标记
        let (removed, markers, warned) = cleanup_orphan_recordings(&dir.to_string_lossy(), &dir);
        assert_eq!(removed, 2, "下次启动应删除 .part + 标记");
        assert_eq!(markers, 1);
        assert_eq!(warned, 0);
        assert!(!renamed.exists(), ".part 残留应在下次启动被清理");
        assert!(!marker_path.exists());
        assert!(unrelated.exists(), "无关文件不得删除");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crash_partials_segmented_renames_segments_and_deletes_old_partial() {
        // H5 分段：段文件改名为 .part；已带 .part 的旧残留直接删除
        let dir = orphan_test_dir("h5-seg");
        let _ = std::fs::remove_dir_all(&dir);
        let out_dir = dir.join("主播A");
        std::fs::create_dir_all(&out_dir).unwrap();
        let base = out_dir.join("2026-08-07_12-30-45_主播A_001");
        let seg0 = out_dir.join("2026-08-07_12-30-45_主播A_001_000.m4a");
        let seg1_old_part = out_dir.join("2026-08-07_12-30-45_主播A_001_001.m4a.part");
        std::fs::write(&seg0, b"s0").unwrap();
        std::fs::write(&seg1_old_part, b"old").unwrap();

        let handled = mark_crash_partials(&base.to_string_lossy(), true, "m4a");
        assert_eq!(handled, 2, "新段改名 + 旧 .part 删除");
        assert!(!seg0.exists());
        assert!(out_dir.join("2026-08-07_12-30-45_主播A_001_000.m4a.part").exists());
        assert!(!seg1_old_part.exists(), "已带 .part 的旧残留应直接删除");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
