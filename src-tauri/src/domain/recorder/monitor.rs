use std::time::Duration;
use tauri::WebviewWindow;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use crate::domain::config::manager::ConfigManager;
use crate::domain::config::model::AnchorStatusUpdate;
use crate::domain::recorder::engine::{FfmpegRecorder, RecorderEngine};
use crate::domain::services::file_cache::{FileCacheHandle, FileCacheManager};
use crate::domain::spider::MissevanClient;
use crate::infrastructure::state::app_state::{AppStateHandle, RecordingSummary};
use std::sync::Arc;
use tauri::Emitter;

pub async fn monitor_recording(
    anchor_id: String,
    anchor_name: String,
    room_id: String,
    output_path: String,
    cancel_token: CancellationToken,
    recorder: Arc<FfmpegRecorder>,
    client: MissevanClient,
    notifier: Arc<crate::infrastructure::notification::dispatcher::NotificationDispatcher>,
    // 预留：后续按需使用（如录制时长上限）；当前录制参数已由 engine 侧消费
    _config: crate::domain::config::model::GlobalConfig,
    app_state: AppStateHandle,
    window: WebviewWindow,              // 用于推送事件
    file_cache: FileCacheHandle,        // 文件缓存
    config_manager: Arc<ConfigManager>, // 配置管理器（用于刷新缓存时获取主播列表）
) {
    let start_time = std::time::Instant::now();
    let max_duration = Duration::from_secs(24 * 60 * 60);
    let mut consecutive_api_failures = 0;
    const MAX_API_FAILURES: u32 = 3;
    // 最近一次 API 直播判定（默认 true：录制启动时流存在）；
    // 录制结束时推送该值（而非硬编码 false），避免直播实际仍在时误显离线
    let mut last_api_live = true;

    notifier
        .info(
            "REC_START",
            format!("开始录制: {}", anchor_name),
            format!("主播 {} 的直播正在录制", anchor_name),
        )
        .await;

    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => {
                notifier.info("REC_STOP", format!("录制已取消: {}", anchor_name), "用户取消或直播结束".to_string()).await;
                break;
            }
            _ = sleep(Duration::from_secs(10)) => {
                if start_time.elapsed() > max_duration {
                    notifier.error("REC_TIMEOUT", format!("录制超时: {}", anchor_name), "超过 24 小时安全阀".to_string()).await;
                    break;
                }

                // 兜底（方案 C 防漏）：录制期间主播的「检测与自动录制」被关闭
                //（update_anchor 保存即停为主路径，此处低频兜底覆盖竞态/其他写
                // 路径）。monitor 持有的是录制启动时的主播快照，enable_check 的
                // 最新值须实时读配置；配置读取失败时跳过（保持录制，避免误停）。
                match config_manager.load() {
                    Ok(cfg) => {
                        let check_disabled = cfg
                            .anchors
                            .iter()
                            .find(|a| a.id == anchor_id)
                            .is_some_and(|a| !a.enable_check);
                        if check_disabled {
                            notifier.info("REC_STOP_CHECK_DISABLED", format!("已停止录制: {}", anchor_name), "主播的「启用检测与自动录制」已关闭".to_string()).await;
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::warn!("[录制] 读取配置失败（跳过检测开关兜底检查）: {}", e);
                    }
                }

                match client.check_live(&room_id, None).await {
                    Ok(result) => {
                        consecutive_api_failures = 0;
                        last_api_live = result.is_live;
                        if !result.is_live {
                            notifier.info("REC_ENDED", format!("直播结束: {}", anchor_name), "API 返回未直播状态".to_string()).await;
                            break;
                        }
                    }
                    Err(e) => {
                        // 错误分类（规格「直播状态异常修复」）：
                        // Server/Network/Format 为瞬时错误（5XX/429/网络抖动/格式变化），
                        // 不判离线也不计入失败阈值——FFmpeg 正在录说明流存在，避免风控
                        // 误报中断进行中的录制；仅「明确离线」（Other，如 404）计失败。
                        if e.is_transient() {
                            tracing::warn!(
                                "[录制] API 瞬时错误（不影响录制，保持直播判定）: {}: {}",
                                anchor_name,
                                e
                            );
                        } else {
                            consecutive_api_failures += 1;
                            notifier.warning("REC_API_ERR", format!("API 检测失败 ({}/{}): {}", consecutive_api_failures, MAX_API_FAILURES, anchor_name), e.message().to_string()).await;
                            if consecutive_api_failures >= MAX_API_FAILURES {
                                notifier.error("REC_API_FAILED", format!("API 连续失败，停止录制: {}", anchor_name), "连续 3 次 API 调用失败".to_string()).await;
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    // 停止 FFmpeg
    let _ = recorder.stop(&anchor_id).await;

    // 从 AppState 中移除录制任务
    app_state.lock().await.remove_task(&anchor_id);

    // 记录录制历史摘要（调试页「录制引擎」模块；最新在前）
    {
        let duration = start_time.elapsed().as_secs();
        let ended_at = chrono::Local::now();
        let started_at = ended_at - chrono::Duration::seconds(duration as i64);
        app_state.lock().await.record_history(RecordingSummary {
            anchor_id: anchor_id.clone(),
            anchor_name: anchor_name.clone(),
            room_id: room_id.clone(),
            output_path: output_path.clone(),
            started_at: started_at.to_rfc3339(),
            duration_secs: duration,
            ended_at: ended_at.to_rfc3339(),
        });
    }

    // 🔔 推送录制状态变为 false；is_live 用最近一次 API 判定（双重验证语义下
    // 录制中一直保持「直播中」；结束时的直播状态由下一轮检测循环校正）
    let update = AnchorStatusUpdate {
        anchor_id: anchor_id.clone(),
        is_live: last_api_live,
        is_recording: false,
    };
    let _ = window.emit("recording_status_changed", &update);
    tracing::info!("录制任务已从状态中移除: {}", anchor_id);

    // 刷新文件缓存，让前端立刻看到新文件
    // （任务已从 AppState 移除，刷新时该文件不会再被标记为「录制中」）
    let cache_manager = FileCacheManager::new(window, file_cache);
    if let Err(e) = cache_manager.refresh(&config_manager, &app_state).await {
        tracing::error!("文件缓存刷新失败: {}", e);
    }
}
