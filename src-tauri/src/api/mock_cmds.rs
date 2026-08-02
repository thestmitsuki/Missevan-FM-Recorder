use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

use crate::infrastructure::error::types::AppError;
use crate::infrastructure::notification::dispatcher::NotificationDispatcher;
use crate::infrastructure::state::app_state::RecorderState;
pub use crate::infrastructure::state::mock_store::MockLiveData;

/// `mock:status_changed` 事件负载（前端 Mock 面板据此刷新状态）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockStatusChanged {
    pub enabled: bool,
    pub count: usize,
}

/// 设置模拟直播数据，用于前端调试录制流程
///
/// 启用 mock 模式后，直播检测将使用模拟数据而非真实 API。
/// 设置 `is_live: true` 可触发模拟开播检测。
#[tauri::command]
pub async fn set_mock_live_data(
    state: State<'_, RecorderState>,
    data: MockLiveData,
) -> Result<(), AppError> {
    // 写入 MockStore（DetectionLoop 在 mock 模式下读取该数据源）
    state.mock_store.upsert(data.clone());

    // 首次设置时自动启用 mock 模式
    if !state.mock_store.is_mock_mode() {
        state.mock_store.set_mode(true);
        state.mock_mode.store(true, Ordering::Relaxed);
        tracing::info!("Mock 模式已启用");
    }

    tracing::info!(
        "模拟直播数据已更新: name={}, is_live={}, stream_url={}",
        data.name,
        data.is_live,
        data.stream_url
    );

    emit_mock_status_changed(&state).await;

    Ok(())
}

/// 切换模拟模式开关
#[tauri::command]
pub async fn set_mock_mode(
    state: State<'_, RecorderState>,
    enable: bool,
    dispatcher: State<'_, Arc<NotificationDispatcher>>,
) -> Result<(), AppError> {
    let old = state.mock_store.is_mock_mode();
    if old == enable {
        // 状态未变化，无需通知
        return Ok(());
    }

    state.mock_store.set_mode(enable);
    state.mock_mode.store(enable, Ordering::Relaxed);
    tracing::info!("模拟模式已{}", if enable { "启用" } else { "禁用" });

    // 发送系统通知
    dispatcher
        .info(
            "mock_mode_toggle",
            &format!("模拟模式已{}", if enable { "启用" } else { "禁用" }),
            if enable {
                "现在检测循环将使用模拟数据，请通过模拟控制面板调整主播信息。"
            } else {
                "已切回真实直播检测。"
            },
        )
        .await;

    // 通知前端 mock 状态变更
    emit_mock_status_changed(&state).await;

    Ok(())
}

/// 向所有窗口广播 `mock:status_changed`
async fn emit_mock_status_changed(state: &RecorderState) {
    let payload = MockStatusChanged {
        enabled: state.mock_store.is_mock_mode(),
        count: state.mock_store.list().len(),
    };
    if let Some(handle) = state.app_handle.lock().await.as_ref() {
        let _ = handle.emit("mock:status_changed", &payload);
    }
}

// ========== Mock 面板 CRUD（Task 16：调试页 Mock 控制面板） ==========

/// 校验模拟主播条目（room_id 必填）
fn validate_mock_anchor(anchor: &MockLiveData) -> Result<(), AppError> {
    if anchor.room_id.trim().is_empty() {
        return Err(AppError::config("模拟主播房间号不能为空"));
    }
    Ok(())
}

/// 列出全部模拟主播
#[tauri::command]
pub async fn list_mock_anchors(
    state: State<'_, RecorderState>,
) -> Result<Vec<MockLiveData>, AppError> {
    Ok(state.mock_store.list())
}

/// 新增模拟主播（room_id 已存在则覆盖）
#[tauri::command]
pub async fn add_mock_anchor(
    state: State<'_, RecorderState>,
    anchor: MockLiveData,
) -> Result<(), AppError> {
    validate_mock_anchor(&anchor)?;
    state.mock_store.upsert(anchor);
    tracing::info!("[Mock] 已新增模拟主播");
    emit_mock_status_changed(&state).await;
    Ok(())
}

/// 更新模拟主播（按 room_id 定位；room_id 不可改名）
#[tauri::command]
pub async fn update_mock_anchor(
    state: State<'_, RecorderState>,
    anchor: MockLiveData,
) -> Result<(), AppError> {
    let room_id = anchor.room_id.clone();
    validate_mock_anchor(&anchor)?;
    state.mock_store.upsert(anchor);
    tracing::info!("[Mock] 已更新模拟主播: {}", room_id);
    emit_mock_status_changed(&state).await;
    Ok(())
}

/// 删除模拟主播
#[tauri::command]
pub async fn remove_mock_anchor(
    state: State<'_, RecorderState>,
    room_id: String,
) -> Result<(), AppError> {
    state.mock_store.remove(&room_id);
    tracing::info!("[Mock] 已删除模拟主播: {}", room_id);
    emit_mock_status_changed(&state).await;
    Ok(())
}

/// 一键设置所有模拟主播的直播状态（全部开播 / 全部下播）
#[tauri::command]
pub async fn set_all_mock_live(
    state: State<'_, RecorderState>,
    live: bool,
) -> Result<(), AppError> {
    state.mock_store.set_all_live(live);
    tracing::info!(
        "[Mock] 全部模拟主播已{}",
        if live { "开播" } else { "下播" }
    );
    emit_mock_status_changed(&state).await;
    Ok(())
}

/// 重置所有模拟数据（清空模拟主播表；模式开关保持不变）
#[tauri::command]
pub async fn reset_mock(state: State<'_, RecorderState>) -> Result<(), AppError> {
    state.mock_store.reset();
    tracing::info!("[Mock] 模拟数据已重置");
    emit_mock_status_changed(&state).await;
    Ok(())
}
