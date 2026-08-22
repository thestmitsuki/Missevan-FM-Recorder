use tauri::State;

use crate::infrastructure::error::types::AppError;
use crate::infrastructure::state::app_state::RecorderState;
use crate::tr;

/// 启动录制任务 —— 占位命令（L5 审查跟进）
///
/// **当前版本未使用**：本命令为早期占位实现（曾注册一个无真实 ffmpeg 的空
/// 任务，插入空 output_path 且停不下来时占用活跃任务数/并发上限，属误用
/// 陷阱）。现按项目惯例保留为**占位 + 禁用语义**：注册保持（命令存在），
/// 调用一律返回明确错误，不注册任何任务、不占用任何运行时状态。
///
/// 实际录制入口：
/// - 自动录制：检测循环（detector/loop.rs）→ `engine::start_ffmpeg_recording`
///   （含双录防御/并发上限/路径模板渲染的完整实现）；
/// - 手动停止：`stop_recording`（前端「停止录制」按钮调用）。
///
/// TODO（未来需要「手动指定流地址录制」时再实现）：对接录制引擎
///（engine + monitor），至少需校验 stream_url / output_path 非空并走
/// `start_ffmpeg_recording` 同款防线；不得恢复旧的空任务占位行为。
#[tauri::command]
pub async fn start_recording(
    _state: State<'_, RecorderState>,
    _anchor_id: String,
    _stream_url: String,
    _output_path: String,
) -> Result<(), AppError> {
    Err(AppError::config(tr!(
        "recorder.start_placeholder"
    )))
}

/// 停止录制任务
#[tauri::command]
pub async fn stop_recording(
    state: State<'_, RecorderState>,
    anchor_id: String,
) -> Result<(), AppError> {
    let mut app_state = state.state.lock().await;

    match app_state.remove_task(&anchor_id) {
        Some(task) => {
            task.cancel_token.cancel();
            // 不 await handle，避免阻塞命令返回
            let _ = task.handle;
            tracing::info!("{}", tr!("recorder.stopped", anchor_id = anchor_id));
            Ok(())
        }
        None => {
            // pre_record_delay 延迟窗口内的启动尚未注册进 tasks——
            // 从 pending_starts 取消（取消令牌触发后 lib.rs 的 select! 放弃启动）
            if app_state.cancel_pending_start(&anchor_id) {
                tracing::info!(
                    "{}",
                    tr!("recorder.cancelled_pending_start", anchor_id = anchor_id)
                );
                Ok(())
            } else {
                Err(AppError::recording(
                    crate::infrastructure::error::types::RC_STREAM_UNAVAILABLE,
                    tr!("recorder.not_recording", anchor_id = anchor_id),
                ))
            }
        }
    }
}
