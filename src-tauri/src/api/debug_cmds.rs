use chrono::Utc;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::State;
use tokio::sync::Mutex;

use crate::domain::config::manager::{redact_proxy_url, ConfigManager};
use crate::domain::config::model::GlobalConfig;
use crate::domain::detector::r#loop::DetectionLoop;
use crate::domain::detector::stats::DetectorStatsSnapshot;
use crate::domain::services::file_cache::{
    FileCacheHandle, FileCacheState, ScanLogEntry,
};
use crate::infrastructure::checker::checks::{DiskSpaceCheck, FfmpegCheck};
use crate::infrastructure::checker::report::{CheckResult, CheckStatus, DiagnosticReport};
use crate::infrastructure::checker::runner::CheckRunner;
use crate::infrastructure::error::types::AppError;
use crate::infrastructure::logging::buffer::{LogBuffer, LogEntry};
use crate::infrastructure::logging::network::{NetworkLog, NetworkLogStore};
use crate::infrastructure::notification::dispatcher::NotificationDispatcher;
use crate::infrastructure::state::app_state::{
    ActiveRecording, RecorderState, RecorderStateInfo, RecordingSummary,
};

/// 工具（FFmpeg / ffprobe）状态——`get_debug_info.ffmpeg_status` /
/// `ffprobe_status` 返回，前端运行概览「系统信息」区展示。
///
/// 解析语义与录制引擎一致（`domain::tools` 候选顺序：配置指定路径 →
/// `{exe_dir}/ffmpeg/<工具>.exe` → PATH），并试运行 `-version` 取版本号。
#[derive(Debug, Clone, Serialize)]
pub struct ToolStatus {
    /// 是否找到可执行的工具
    pub found: bool,
    /// 实际解析到的可执行文件路径（裸名走 PATH 时为工具名本身）
    pub path: String,
    /// `-version` 首行版本信息；找到但取版本失败时为 None
    pub version: Option<String>,
}

/// 工具探测缓存（TTL 60s）：前端运行概览每 2s 轮询 get_debug_info，
/// 每次跑两次 `-version` 子进程开销不必要；键 = 配置中的路径（候选
/// 解析仅依赖它），60s 内重复调用直接返回缓存结果。
static TOOL_CACHE: OnceLock<StdMutex<HashMap<String, (Instant, ToolStatus)>>> = OnceLock::new();

const TOOL_CACHE_TTL: Duration = Duration::from_secs(60);
/// `-version` 试运行超时（防损坏/挂起的可执行文件卡住 get_debug_info）
const TOOL_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// 依次尝试候选可执行文件：存在且 `-version` 执行成功 → 返回 found + 版本；
/// 裸名（如 "ffmpeg"，走 PATH）不检查 exists()，直接试运行。
async fn probe_tool(candidates: Vec<std::path::PathBuf>) -> ToolStatus {
    for cand in candidates {
        let is_bare = cand.components().count() <= 1;
        if !is_bare && !cand.exists() {
            continue;
        }
        // 隐藏控制台：发布构建无控制台，探测 spawn 的 ffmpeg/ffprobe（控制台
        // 子系统）缺 CREATE_NO_WINDOW 会闪现黑窗口——调试页每 60s 探测一次，
        // 是「黑窗口闪现」的最频繁来源（tools.rs::apply_create_no_window）
        let mut probe = tokio::process::Command::new(&cand);
        probe.arg("-version");
        #[cfg(windows)]
        crate::domain::tools::apply_create_no_window(probe.as_std_mut());
        let result = tokio::time::timeout(TOOL_PROBE_TIMEOUT, probe.output()).await;
        match result {
            Ok(Ok(out)) if out.status.success() => {
                let version = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .next()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                return ToolStatus {
                    found: true,
                    path: cand.to_string_lossy().into_owned(),
                    version,
                };
            }
            _ => continue,
        }
    }
    ToolStatus {
        found: false,
        path: String::new(),
        version: None,
    }
}

/// 带缓存的工具探测（TTL 60s；键 = 配置路径，配置变更后自动重新探测）
async fn cached_probe(key: &str, candidates: Vec<std::path::PathBuf>) -> ToolStatus {
    {
        let cache = TOOL_CACHE.get_or_init(|| StdMutex::new(HashMap::new()));
        let guard = cache.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((at, status)) = guard.get(key) {
            if at.elapsed() < TOOL_CACHE_TTL {
                return status.clone();
            }
        }
    }
    let status = probe_tool(candidates).await;
    let cache = TOOL_CACHE.get_or_init(|| StdMutex::new(HashMap::new()));
    cache
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(key.to_string(), (Instant::now(), status.clone()));
    status
}

/// 调试信息——返回给前端的状态概览。
///
/// 字段与前端 `api.ts` 的 `get_debug_info` 类型对齐（Task 15 统一：
/// `active_tasks` → `active_recordings`，语义 = 活跃录制任务数）。
#[derive(Debug, Clone, Serialize)]
pub struct DebugInfo {
    pub active_recordings: usize,
    pub mock_mode: bool,
    // —— 概览模块附加字段 ——
    pub app_version: String,
    pub rust_version: String,
    pub tauri_version: String,
    pub os: String,
    pub detector_running: bool,
    pub total_checks: u64,
    pub success_checks: u64,
    pub failed_checks: u64,
    pub enabled_anchors: usize,
    pub live_anchors: usize,
    pub recording_anchors: usize,
    pub file_count: usize,
    /// FFmpeg 可执行文件状态（found / path / version）
    pub ffmpeg_status: ToolStatus,
    /// ffprobe 可执行文件状态（found / path / version）
    pub ffprobe_status: ToolStatus,
}

/// 完整诊断报告（包含健康检查结果和配置状态）
#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticFullReport {
    pub health: DiagnosticReport,
    pub config_exists: bool,
    pub config_valid: bool,
    pub config_errors: Vec<String>,
}

// ========== 底层检查执行器（不加载配置，无通知） ==========
async fn perform_health_checks(config: &GlobalConfig, mock_mode: bool) -> DiagnosticReport {
    if mock_mode {
        let results = vec![
            CheckResult {
                check_name: "FFmpeg".into(),
                status: CheckStatus::Passed,
                message: "Mock 模式，忽略".into(),
                details: None,
                suggestion: None,
                duration_ms: 0,
            },
            CheckResult {
                check_name: "磁盘空间".into(),
                status: CheckStatus::Passed,
                message: "Mock 模式，忽略".into(),
                details: None,
                suggestion: None,
                duration_ms: 0,
            },
        ];

        let count = results.len();
        return DiagnosticReport {
            results, // 字段名与变量名相同，可简写
            total: count,
            passed: count,
            failed: 0,
            warnings: 0,
            timestamp: Utc::now().to_rfc3339(), // 模拟耗时设为 0
        };
    }

    let mut runner = CheckRunner::new();
    runner.register(Box::new(FfmpegCheck {
        ffmpeg_path: config.ffmpeg_path.clone(),
    }));
    runner.register(Box::new(DiskSpaceCheck {
        output_dir: config.output_dir.clone(),
        threshold_gb: config.disk_space_limit_gb,
    }));

    runner.run_all().await
}

// ========== 辅助函数：加载配置、验证、生成报告（无通知） ==========
async fn run_diagnostic(
    state: &State<'_, RecorderState>,
    config_manager: &State<'_, Arc<ConfigManager>>,
) -> DiagnosticFullReport {
    let mock_mode = state.mock_mode.load(Ordering::Relaxed);

    // 1. Mock 模式：直接返回模拟报告（不加载配置）
    if mock_mode {
        let dummy_config = GlobalConfig::default();
        let health = perform_health_checks(&dummy_config, mock_mode).await;
        return DiagnosticFullReport {
            health,
            config_exists: true,
            config_valid: true,
            config_errors: vec![],
        };
    }

    // 2. 加载配置
    let config = match config_manager.load() {
        Ok(c) => c,
        Err(e) => {
            let error_results = vec![CheckResult {
                check_name: "配置加载".into(),
                status: CheckStatus::Failed,
                message: format!("配置加载失败: {}", e),
                details: None,
                suggestion: Some("请检查配置文件是否存在，或使用 --mock 模式".into()),
                duration_ms: 0,
            }];
            let health = DiagnosticReport {
                results: error_results,
                total: 1,
                passed: 0,
                failed: 1,
                warnings: 0,
                timestamp: Utc::now().to_rfc3339(),
            };
            return DiagnosticFullReport {
                health,
                config_exists: false,
                config_valid: false,
                config_errors: vec![format!("配置加载失败: {}", e)],
            };
        }
    };

    // 3. 验证配置
    let (config_valid, config_errors) = match config.is_valid() {
        Ok(()) => (true, vec![]),
        Err(errs) => (false, errs),
    };

    // 4. 执行健康检查
    let health = perform_health_checks(&config.global, mock_mode).await;

    DiagnosticFullReport {
        health,
        config_exists: true,
        config_valid,
        config_errors,
    }
}

// ========== Tauri 命令：统一诊断入口（负责发送通知） ==========
#[tauri::command]
pub async fn run_health_check(
    state: State<'_, RecorderState>,
    config_manager: State<'_, Arc<ConfigManager>>,
    dispatcher: State<'_, Arc<NotificationDispatcher>>,
) -> Result<DiagnosticFullReport, AppError> {
    // 1. 调用辅助函数获取报告（无通知）
    let report = run_diagnostic(&state, &config_manager).await;

    // 2. 根据报告发送通知
    let health_ok = report.health.results.iter().all(|r| {
        matches!(
            r.status,
            CheckStatus::Passed | CheckStatus::Warning | CheckStatus::Skipped
        )
    });
    let all_ok = health_ok && report.config_valid;

    if all_ok {
        dispatcher
            .info(
                "diagnostic_ok",
                "诊断通过",
                "所有检查项均正常，系统准备就绪。",
            )
            .await;
    } else {
        let mut error_details = Vec::new();
        for r in &report.health.results {
            match r.status {
                // 如果是失败，记录错误
                CheckStatus::Failed => {
                    error_details.push(format!("{}: {}", r.check_name, r.message));
                }
                // 如果是警告，也记录（可以加个前缀标明是警告）
                CheckStatus::Warning => {
                    error_details.push(format!("{}: (警告) {}", r.check_name, r.message));
                }
                // 通过和跳过的情况，忽略，不做任何事
                CheckStatus::Passed | CheckStatus::Skipped => {}
            }
        }
        if !report.config_errors.is_empty() {
            error_details.extend(report.config_errors.clone());
        }
        let detail = if error_details.is_empty() {
            "未知错误".to_string()
        } else {
            error_details.join("; ")
        };
        dispatcher
            .error(
                "diagnostic_failed",
                "诊断发现问题",
                format!("检测到以下问题：{}", detail),
            )
            .await;
    }

    Ok(report)
}

/// 获取调试信息（活跃录制数、Mock 模式状态、版本/统计概览等）
#[tauri::command]
pub async fn get_debug_info(
    state: State<'_, RecorderState>,
    detection_loop: State<'_, Arc<DetectionLoop>>,
    config_manager: State<'_, Arc<ConfigManager>>,
    live_cache: State<'_, Arc<Mutex<HashMap<String, bool>>>>,
    file_cache: State<'_, FileCacheHandle>,
) -> Result<DebugInfo, AppError> {
    let app_state = state.state.lock().await;
    let active_recordings = app_state.active_count();
    let recording_anchors = app_state.tasks.len();
    let mock_mode = state.mock_mode.load(Ordering::Relaxed);
    // 双重验证归并口径：直播中 = API 判直播 ∪ 录制中（与前端展示一致）
    let live_cache_guard = live_cache.lock().await;
    let live_anchors = live_cache_guard
        .iter()
        .filter(|(id, v)| **v || app_state.tasks.contains_key(*id))
        .count();
    drop(live_cache_guard);
    let file_count = file_cache.lock().await.files.len();
    drop(app_state);

    let stats = detection_loop.stats.snapshot();
    // 配置只加载一次（enabled_anchors 计数 + ffmpeg/ffprobe 状态共用）
    let config = config_manager.load().ok();
    let enabled_anchors = config
        .as_ref()
        .map(|c| c.anchors.iter().filter(|a| a.enable_check).count())
        .unwrap_or(0);

    // FFmpeg / ffprobe 状态：候选顺序与录制引擎一致（tools.rs），
    // 探测结果 60s 缓存——前端 2s 轮询不会反复跑 `-version` 子进程。
    let ffmpeg_status = cached_probe(
        &format!("ffmpeg|{}", config.as_ref().and_then(|c| c.global.ffmpeg_path.clone()).unwrap_or_default()),
        crate::domain::tools::ffmpeg_candidates(
            config.as_ref().and_then(|c| c.global.ffmpeg_path.as_deref()),
        ),
    )
    .await;
    let ffprobe_status = cached_probe(
        &format!("ffprobe|{}", config.as_ref().map(|c| c.global.ffprobe_path.clone()).unwrap_or_default()),
        crate::domain::tools::ffprobe_candidates(
            config.as_ref().map(|c| c.global.ffprobe_path.as_str()).unwrap_or(""),
        ),
    )
    .await;

    Ok(DebugInfo {
        active_recordings,
        mock_mode,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        rust_version: env!("CARGO_PKG_RUST_VERSION").to_string(),
        tauri_version: tauri::VERSION.to_string(),
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        detector_running: stats.running,
        total_checks: stats.total_checks,
        success_checks: stats.success_checks,
        failed_checks: stats.failed_checks,
        enabled_anchors,
        live_anchors,
        recording_anchors,
        file_count,
        ffmpeg_status,
        ffprobe_status,
    })
}

// ========== 调试命令：实时日志 ==========

/// 获取日志（级别 / 来源过滤；来源 = module 子串，如 detector / recorder / spider / file_cache）
#[tauri::command]
pub async fn get_logs(
    log_buffer: State<'_, Arc<LogBuffer>>,
    level: Option<String>,
    source: Option<String>,
) -> Result<Vec<LogEntry>, AppError> {
    Ok(log_buffer.filter(level.as_deref(), source.as_deref()))
}

#[tauri::command]
pub async fn clear_logs(log_buffer: State<'_, Arc<LogBuffer>>) -> Result<(), AppError> {
    log_buffer.clear();
    Ok(())
}

// ========== 调试命令：网络请求 ==========

#[tauri::command]
pub async fn get_network_logs(
    network_store: State<'_, Arc<NetworkLogStore>>,
) -> Result<Vec<NetworkLog>, AppError> {
    Ok(network_store.all())
}

#[tauri::command]
pub async fn clear_network_logs(
    network_store: State<'_, Arc<NetworkLogStore>>,
) -> Result<(), AppError> {
    network_store.clear();
    Ok(())
}

// ========== 调试命令：检测循环 ==========

/// 聚合检测统计（运行状态 / 上次检测时间 / 总 / 成功 / 失败 / 启用主播数 / 直播中 / 录制中）
async fn collect_detector_stats(
    detection_loop: &DetectionLoop,
    config_manager: &ConfigManager,
    live_cache: &Mutex<HashMap<String, bool>>,
    recorder_state: &RecorderState,
) -> DetectorStatsSnapshot {
    let mut snap = detection_loop.stats.snapshot();
    snap.enabled_anchors = config_manager
        .load()
        .map(|c| c.anchors.iter().filter(|a| a.enable_check).count())
        .unwrap_or(0);
    // 双重验证归并口径：直播中 = API 判直播 ∪ 录制中（与前端展示一致）
    let tasks = recorder_state.state.lock().await;
    let cache = live_cache.lock().await;
    snap.live_anchors = cache
        .iter()
        .filter(|(id, v)| **v || tasks.tasks.contains_key(*id))
        .count();
    snap.recording_anchors = tasks.tasks.len();
    drop(cache);
    drop(tasks);
    snap
}

#[tauri::command]
pub async fn get_detector_stats(
    detection_loop: State<'_, Arc<DetectionLoop>>,
    config_manager: State<'_, Arc<ConfigManager>>,
    live_cache: State<'_, Arc<Mutex<HashMap<String, bool>>>>,
    recorder_state: State<'_, RecorderState>,
) -> Result<DetectorStatsSnapshot, AppError> {
    Ok(collect_detector_stats(
        &detection_loop,
        &config_manager,
        &live_cache,
        &recorder_state,
    )
    .await)
}

/// 立即触发一轮检测（唤醒检测循环）
#[tauri::command]
pub async fn trigger_detection_now(
    detection_loop: State<'_, Arc<DetectionLoop>>,
) -> Result<(), AppError> {
    detection_loop.trigger_now();
    Ok(())
}

/// 重置检测统计计数
#[tauri::command]
pub async fn reset_detector_stats(
    detection_loop: State<'_, Arc<DetectionLoop>>,
) -> Result<(), AppError> {
    detection_loop.stats.reset();
    Ok(())
}

// ========== 调试命令：录制引擎 ==========

/// 录制引擎状态：活跃任务列表 + 最近结束的录制历史（最新 20 条）
#[tauri::command]
pub async fn get_recorder_state(
    state: State<'_, RecorderState>,
) -> Result<RecorderStateInfo, AppError> {
    let app_state = state.state.lock().await;
    let active: Vec<ActiveRecording> = app_state
        .tasks
        .values()
        .map(|task| ActiveRecording {
            anchor_id: task.anchor_id.clone(),
            anchor_name: task.anchor_name.clone(),
            room_id: task.room_id.clone(),
            status: "recording".to_string(),
            duration_secs: task.started_at.elapsed().as_secs(),
            output_path: task.output_path.clone(),
            pid: task.pid,
        })
        .collect();
    let history: Vec<RecordingSummary> = app_state.history.iter().take(20).cloned().collect();
    Ok(RecorderStateInfo { active, history })
}

// ========== 调试命令：文件缓存 ==========

/// 文件缓存状态（上次扫描时间 / 文件数 / 分段组数 / 扫描日志）
#[tauri::command]
pub async fn get_file_cache_state(
    cache: State<'_, FileCacheHandle>,
) -> Result<FileCacheState, AppError> {
    let cache = cache.lock().await;
    Ok(cache.state())
}

/// 清除文件缓存（只清内存索引，不动磁盘文件）
#[tauri::command]
pub async fn clear_file_cache(cache: State<'_, FileCacheHandle>) -> Result<(), AppError> {
    let mut cache = cache.lock().await;
    let files_before = cache.files.len();
    let groups_before = cache.groups.len();
    cache.clear_cache();
    cache.push_scan_log(ScanLogEntry {
        timestamp: Utc::now().to_rfc3339(),
        kind: "clear".into(),
        duration_ms: 0,
        files_before,
        files_after: 0,
        groups: groups_before,
    });
    tracing::info!("文件缓存已清除（内存索引）");
    Ok(())
}

// ========== 调试命令：Mock 面板 ==========

/// Mock 面板状态（mock 模式开关 + 模拟主播数；Task 15 提前补全，供 Task 16 初始加载）
#[tauri::command]
pub async fn get_mock_state(
    state: State<'_, RecorderState>,
) -> Result<crate::api::mock_cmds::MockStatusChanged, AppError> {
    Ok(crate::api::mock_cmds::MockStatusChanged {
        enabled: state.mock_store.is_mock_mode(),
        count: state.mock_store.list().len(),
    })
}

// ========== 调试命令：诊断报告导出 ==========

/// 导出诊断报告：概览 + 配置（脱敏）+ 日志 + 网络记录 + 检测统计 + 录制状态 + 文件缓存。
/// 返回 JSON 字符串，前端保存为文件。
#[tauri::command]
pub async fn export_diagnostic_report(
    recorder_state: State<'_, RecorderState>,
    config_manager: State<'_, Arc<ConfigManager>>,
    log_buffer: State<'_, Arc<LogBuffer>>,
    network_store: State<'_, Arc<NetworkLogStore>>,
    detection_loop: State<'_, Arc<DetectionLoop>>,
    live_cache: State<'_, Arc<Mutex<HashMap<String, bool>>>>,
    file_cache: State<'_, FileCacheHandle>,
) -> Result<String, AppError> {
    use serde_json::json;

    let app_state = recorder_state.state.lock().await;
    let recorder = json!({
        "active": app_state
            .tasks
            .values()
            .map(|t| ActiveRecording {
                anchor_id: t.anchor_id.clone(),
                anchor_name: t.anchor_name.clone(),
                room_id: t.room_id.clone(),
                status: "recording".to_string(),
                duration_secs: t.started_at.elapsed().as_secs(),
                output_path: t.output_path.clone(),
                pid: t.pid,
            })
            .collect::<Vec<_>>(),
        "history": app_state.history.iter().take(20).cloned().collect::<Vec<_>>(),
    });
    let active_recordings = app_state.active_count();
    drop(app_state);

    let config_value = config_manager
        .load()
        .map(|c| redact_config(&c))
        .unwrap_or_else(|e| json!({ "error": format!("配置加载失败: {}", e) }));

    // FFmpeg / ffprobe 状态（探测结果带 60s 缓存，重复导出无额外开销）
    let loaded = config_manager.load().ok();
    let ffmpeg_status = cached_probe(
        &format!("ffmpeg|{}", loaded.as_ref().and_then(|c| c.global.ffmpeg_path.clone()).unwrap_or_default()),
        crate::domain::tools::ffmpeg_candidates(
            loaded.as_ref().and_then(|c| c.global.ffmpeg_path.as_deref()),
        ),
    )
    .await;
    let ffprobe_status = cached_probe(
        &format!("ffprobe|{}", loaded.as_ref().map(|c| c.global.ffprobe_path.clone()).unwrap_or_default()),
        crate::domain::tools::ffprobe_candidates(
            loaded.as_ref().map(|c| c.global.ffprobe_path.as_str()).unwrap_or(""),
        ),
    )
    .await;
    let overview = json!({
        "exported_at": Utc::now().to_rfc3339(),
        "app_version": env!("CARGO_PKG_VERSION"),
        "rust_version": env!("CARGO_PKG_RUST_VERSION"),
        "tauri_version": tauri::VERSION,
        "os": format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        "mock_mode": recorder_state.mock_mode.load(Ordering::Relaxed),
        "active_recordings": active_recordings,
        "ffmpeg_status": ffmpeg_status,
        "ffprobe_status": ffprobe_status,
    });

    let detector = collect_detector_stats(
        &detection_loop,
        &config_manager,
        &live_cache,
        &recorder_state,
    )
    .await;

    let report = json!({
        "overview": overview,
        "config": config_value,
        "logs": log_buffer.all().into_iter().take(200).collect::<Vec<_>>(),
        "network": network_store.all().into_iter().take(200).collect::<Vec<_>>(),
        "detector": detector,
        "recorder": recorder,
        "file_cache": {
            "state": file_cache.lock().await.state(),
        },
    });

    serde_json::to_string_pretty(&report)
        .map_err(|e| AppError::internal(format!("序列化诊断报告失败: {}", e)))
}

/// 配置脱敏：`global.proxy_password`、`anchor.cookie` → `***`，
/// 代理 URL 内嵌密码（`http://user:pass@host`）→ `http://user:***@host`
fn redact_config(config: &crate::domain::config::model::Config) -> serde_json::Value {
    let mut v = serde_json::to_value(config).unwrap_or(serde_json::Value::Null);
    if let Some(global) = v.get_mut("global").and_then(|g| g.as_object_mut()) {
        global.insert(
            "proxy_password".to_string(),
            serde_json::Value::String("***".to_string()),
        );
    }
    if let Some(anchors) = v.get_mut("anchors").and_then(|a| a.as_array_mut()) {
        for anchor in anchors.iter_mut() {
            if let Some(obj) = anchor.as_object_mut() {
                obj.insert(
                    "cookie".to_string(),
                    serde_json::Value::String("***".to_string()),
                );
                if let Some(proxy) = obj.get_mut("proxy") {
                    if let Some(p) = proxy.as_str() {
                        *proxy = serde_json::Value::String(redact_proxy_url(p));
                    }
                }
            }
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_config_blanks_sensitive_fields() {
        let mut config = crate::domain::config::model::Config::default();
        config.global.proxy_password = "secret".to_string();
        config.anchors.push(crate::domain::config::model::AnchorConfig {
            id: "a1".into(),
            name: "主播A".into(),
            url: "https://fm.missevan.com/live/1001".into(),
            room_id: "1001".into(),
            proxy: Some("http://user:pass@proxy.example.com:8080".into()),
            cookie: Some("ck=abc".into()),
            enable_check: true,
            avatar_url: None,
            tags: vec!["音乐".into()],
        });

        let v = redact_config(&config);
        assert_eq!(v["global"]["proxy_password"], "***");
        assert_eq!(v["anchors"][0]["cookie"], "***");
        let proxy = v["anchors"][0]["proxy"].as_str().unwrap();
        assert!(proxy.contains(":***@"));
        assert!(!proxy.contains("pass@"));
    }

    #[test]
    fn redact_proxy_url_without_password_unchanged() {
        assert_eq!(
            redact_proxy_url("http://proxy.example.com:8080"),
            "http://proxy.example.com:8080"
        );
    }
}
