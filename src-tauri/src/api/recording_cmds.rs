use tauri::State;
use tokio_util::sync::CancellationToken;

use crate::infrastructure::error::types::AppError;
use crate::infrastructure::state::app_state::{RecorderState, Task};

/// 启动录制任务
///
/// 从 anchor_cmds 或 detector 层调用，传入主播 ID 和推流 URL。
/// 该命令会创建一个新的 tokio task 运行录制循环。
#[tauri::command]
pub async fn start_recording(
    state: State<'_, RecorderState>,
    anchor_id: String,
    stream_url: String,
    output_path: String,
) -> Result<(), AppError> {
    let mut app_state = state.state.lock().await;

    if app_state.is_recording(&anchor_id) {
        return Err(AppError::recording(
            crate::infrastructure::error::types::RC_STREAM_UNAVAILABLE,
            format!("主播 {} 已在录制中", anchor_id),
        ));
    }

    let cancel_token = CancellationToken::new();
    let cancel_clone = cancel_token.clone();
    let anchor_id_clone = anchor_id.clone();

    // 启动录制任务（占位——Phase 5 集成后对接录制引擎）
    let handle = tokio::spawn(async move {
        tracing::info!("录制任务启动: anchor_id={}", anchor_id_clone);
        loop {
            tokio::select! {
                _ = cancel_clone.cancelled() => {
                    tracing::info!("录制任务取消: anchor_id={}", anchor_id_clone);
                    break;
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {
                    // 占位：实际录制循环在 monitor.rs 中
                }
            }
        }
    });

    app_state.insert_task(
        anchor_id.clone(),
        Task {
            anchor_id,
            cancel_token,
            handle,
            anchor_name: String::new(),
            room_id: String::new(),
            output_path: String::new(),
            started_at: std::time::Instant::now(),
            pid: None,
        },
    );

    tracing::info!(
        "录制已开始: stream_url={}, output_path={}",
        stream_url,
        output_path
    );
    Ok(())
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
            tracing::info!("录制已停止: anchor_id={}", anchor_id);
            Ok(())
        }
        None => Err(AppError::recording(
            crate::infrastructure::error::types::RC_STREAM_UNAVAILABLE,
            format!("主播 {} 未在录制中", anchor_id),
        )),
    }
}
