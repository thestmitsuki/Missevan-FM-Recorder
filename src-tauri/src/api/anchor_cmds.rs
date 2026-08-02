use tauri::State;
use tracing::info;

use crate::domain::config::manager::ConfigManager;
use crate::domain::config::model::AnchorConfig;
use crate::domain::config::model::RecordingStatus;
use crate::domain::detector::merge_live_state;
use crate::domain::spider::{AnchorProfile, MissevanClient};
use crate::infrastructure::error::types::{AppError, ErrorCategory, ErrorSeverity};
use crate::infrastructure::notification::dispatcher::NotificationDispatcher;
use crate::infrastructure::state::app_state::AppStateHandle;
use crate::infrastructure::state::app_state::AvatarCache;
use crate::infrastructure::state::app_state::RecorderState;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[tauri::command]
pub async fn get_anchors(
    config_manager: State<'_, Arc<ConfigManager>>,
    avatar_cache: State<'_, AvatarCache>,
) -> Result<Vec<AnchorConfig>, AppError> {
    let mut config = config_manager.load()?;
    let client = MissevanClient::new()?; // 保留 client，用于缓存缺失时请求

    for anchor in &mut config.anchors {
        // 1. 检查缓存
        let cached = avatar_cache.lock().await.get(&anchor.id).cloned();
        if let Some(avatar) = cached {
            anchor.avatar_url = Some(avatar);
            continue;
        }

        // 2. 缓存未命中，主动请求（仅一次）
        let room_id = if !anchor.room_id.is_empty() {
            anchor.room_id.clone()
        } else if let Some(rid) = MissevanClient::extract_room_id(&anchor.url) {
            rid
        } else {
            continue; // 无法提取 room_id，跳过
        };

        match client.get_anchor_profile(&room_id).await {
            Ok(profile) => {
                anchor.avatar_url = Some(profile.avatar_url.clone());
                avatar_cache
                    .lock()
                    .await
                    .insert(anchor.id.clone(), profile.avatar_url);
            }
            Err(e) => {
                tracing::warn!("获取主播 {} 头像失败: {}", anchor.id, e);
            }
        }
    }

    Ok(config.anchors)
}

#[tauri::command]
pub async fn get_recording_status(
    state: tauri::State<'_, RecorderState>,
    live_cache: tauri::State<'_, Arc<Mutex<HashMap<String, bool>>>>,
    config_manager: tauri::State<'_, Arc<ConfigManager>>, // 新增
) -> Result<Vec<RecordingStatus>, AppError> {
    info!("[get_recording_status] 开始执行");

    // 1. 获取配置中的所有主播
    let config = match config_manager.load() {
        Ok(c) => c,
        Err(e) => {
            info!("[get_recording_status] 加载配置失败: {}", e);
            return Ok(vec![]); // 配置失败时返回空
        }
    };
    info!(
        "[get_recording_status] 配置中主播数: {}",
        config.anchors.len()
    );

    // 2. 获取录制状态（tasks）和直播缓存
    let app_state = state.state.lock().await;
    let cache = live_cache.lock().await;

    let mut statuses = Vec::new();
    for anchor in &config.anchors {
        let is_recording = app_state.tasks.contains_key(&anchor.id);
        // 双重验证归并：API 判定 || 录制中（API 判离线但录制进行中 → 保持直播中）
        let is_live = merge_live_state(
            cache.get(&anchor.id).copied().unwrap_or(false),
            is_recording,
        );

        statuses.push(RecordingStatus {
            anchor_id: anchor.id.clone(),
            is_recording,
            is_live,
        });
    }

    info!("[get_recording_status] 即将返回 {} 条状态", statuses.len());
    Ok(statuses)
}

#[tauri::command]
pub async fn stop_anchors_recording(
    state: State<'_, RecorderState>,
    anchor_id: String,
) -> Result<(), AppError> {
    let mut guard = state.state.lock().await;
    if let Some(task) = guard.remove_task(&anchor_id) {
        // 取消任务并等待结束（可选）
        task.cancel_token.cancel();
        // 如果想等待任务结束，可以 task.handle.await，但这里避免阻塞，仅取消
        tracing::info!("已停止录制: {}", anchor_id);
        Ok(())
    } else {
        Err(AppError {
            code: "RECORDING_NOT_FOUND",
            category: ErrorCategory::Recording, // 使用 Recording 类别
            severity: ErrorSeverity::Warning,   // 假定 Warning 存在（如果不存在，可用 Error）
            message: format!("未找到录制任务: {}", anchor_id),
            technical: None,
            suggestion: Some("请确认主播ID是否正确，或任务是否已结束。".to_string()),
            source: Some("stop_recording".to_string()),
        })
    }
}

#[tauri::command]
pub async fn add_anchor(
    config_manager: State<'_, Arc<ConfigManager>>,
    dispatcher: State<'_, Arc<NotificationDispatcher>>,
    mut anchor: AnchorConfig,
) -> Result<(), AppError> {
    // 1. 补全 room_id（如果前端未传）
    if anchor.room_id.is_empty() {
        if let Some(room_id) = MissevanClient::extract_room_id(&anchor.url) {
            anchor.room_id = room_id;
        } else {
            let err = AppError {
                code: "INVALID_URL",
                category: ErrorCategory::Config,
                severity: ErrorSeverity::Error,
                message: "无法从主页地址提取房间号".to_string(),
                technical: None,
                suggestion: Some("请确保URL格式为 https://fm.missevan.com/live/数字".to_string()),
                source: Some("add_anchor".to_string()),
            };
            dispatcher
                .error("anchor_add_failed", "URL解析失败", err.message.clone())
                .await;
            return Err(err);
        }
    } else if !anchor.room_id.bytes().all(|b| b.is_ascii_digit()) {
        // M5：前端传入的 room_id 也必须是纯数字（与 update_anchor 强制
        // extract_room_id 的路径一致）——room_id 会拼入 URL 路径与输出目录
        let err = AppError {
            code: "INVALID_ROOM_ID",
            category: ErrorCategory::Config,
            severity: ErrorSeverity::Error,
            message: "房间号必须是纯数字".to_string(),
            technical: None,
            suggestion: Some("请填写 https://fm.missevan.com/live/数字 中的数字部分".to_string()),
            source: Some("add_anchor".to_string()),
        };
        dispatcher
            .error("anchor_add_failed", "房间号无效", err.message.clone())
            .await;
        return Err(err);
    }

    // 2. 调用 API 获取主播真实名称（自定义名称非空时保留，留空则自动获取）
    let client = MissevanClient::new()?;
    let profile = match client.get_anchor_profile(&anchor.room_id).await {
        Ok(p) => p,
        Err(e) => {
            dispatcher
                .error("anchor_fetch_failed", "获取主播信息失败", e.to_string())
                .await;
            return Err(e);
        }
    };
    if anchor.name.trim().is_empty() {
        anchor.name = profile.name;
    }

    // 3. 加载已有配置（用于重复检查）
    let config = config_manager.load()?;

    // 4. 重复检查（ID / URL / 房间号）
    // 房间号去重（双录防御 #1：同一房间号两个主播条目 → 两个独立进程稳定录
    // 同一流——双录根因候选①；trim 后比较，容忍空白差异）
    if room_id_already_exists(&config.anchors, &anchor.room_id) {
        let err = AppError {
            code: "ANCHOR_ROOM_EXISTS",
            category: ErrorCategory::Config,
            severity: ErrorSeverity::Warning,
            message: format!("该主播已添加（房间号 {} 已存在）", anchor.room_id.trim()),
            technical: None,
            suggestion: Some("请检查主播列表，避免为同一直播间添加多个条目".to_string()),
            source: Some("add_anchor".to_string()),
        };
        dispatcher
            .error("anchor_add_failed", "添加主播失败", err.message.clone())
            .await;
        return Err(err);
    }
    if config.anchors.iter().any(|a| a.id == anchor.id) {
        let err = AppError {
            code: "ANCHOR_EXISTS",
            category: ErrorCategory::Config,
            severity: ErrorSeverity::Warning,
            message: format!("主播ID '{}' 已存在", anchor.id),
            technical: None,
            suggestion: Some("请刷新列表或使用不同的ID".to_string()),
            source: Some("add_anchor".to_string()),
        };
        dispatcher
            .error("anchor_add_failed", "添加主播失败", err.message.clone())
            .await;
        return Err(err);
    }
    if config.anchors.iter().any(|a| a.url == anchor.url) {
        let err = AppError {
            code: "ANCHOR_URL_EXISTS",
            category: ErrorCategory::Config,
            severity: ErrorSeverity::Warning,
            message: format!("主播URL '{}' 已被占用", anchor.url),
            technical: None,
            suggestion: Some("请检查URL是否正确".to_string()),
            source: Some("add_anchor".to_string()),
        };
        dispatcher
            .error("anchor_add_failed", "添加主播失败", err.message.clone())
            .await;
        return Err(err);
    }

    // 5. 确保 avatar_url 为 None（不存储到配置文件）
    anchor.avatar_url = None;

    // 6. 保存主播文件（只存 name，不存 avatar_url）
    config_manager.add_anchor(&anchor)?;

    // 7. 成功通知
    dispatcher
        .info(
            "anchor_add_ok",
            "添加成功",
            format!("主播 “{}” 已添加", anchor.name),
        )
        .await;

    tracing::info!("添加主播: {}", anchor.name);
    Ok(())
}

#[tauri::command]
pub async fn remove_anchor(
    state: State<'_, RecorderState>,
    config_manager: State<'_, Arc<ConfigManager>>,
    dispatcher: State<'_, Arc<NotificationDispatcher>>,
    live_cache: State<'_, Arc<Mutex<HashMap<String, bool>>>>,
    id: String,
) -> Result<(), AppError> {
    // 1. 检查主播是否存在于配置中（避免误删）
    let config = config_manager.load()?;
    let anchor = config.anchors.iter().find(|a| a.id == id);
    if anchor.is_none() {
        let err = AppError::config(format!("主播 {} 不存在", id));
        dispatcher
            .error("anchor_remove_failed", "删除失败", err.message.clone())
            .await;
        return Err(err);
    }
    let anchor = anchor.unwrap();

    // 2. 如果正在录制，先停止录制任务
    let mut app_state = state.state.lock().await;
    if let Some(task) = app_state.remove_task(&id) {
        task.cancel_token.cancel();
        tracing::info!("已停止录制任务: {}", id);
        // 释放锁（因为 remove_task 可能已修改状态，我们继续持有锁）
    }
    drop(app_state); // 尽早释放锁，避免后续阻塞

    // 3. 清理直播状态缓存
    {
        let mut cache = live_cache.lock().await;
        cache.remove(&id);
    }

    // 4. 从配置文件中删除主播
    if let Err(e) = config_manager.remove_anchor(&id) {
        dispatcher
            .error(
                "anchor_remove_failed",
                "删除主播配置失败",
                e.message.clone(),
            )
            .await;
        return Err(e);
    }

    // 5. 发送成功通知
    dispatcher
        .info(
            "anchor_remove_ok",
            "删除成功",
            format!("主播 “{}” 已移除", anchor.name),
        )
        .await;

    tracing::info!("删除主播: {} ({})", id, anchor.name);
    Ok(())
}

//刷新主播
#[tauri::command]
pub async fn refresh_anchor(
    config_manager: State<'_, Arc<ConfigManager>>,
    dispatcher: State<'_, Arc<NotificationDispatcher>>,
    anchor_id: String,
    avatar_cache: State<'_, AvatarCache>,
) -> Result<AnchorConfig, AppError> {
    // 1. 加载配置，找到该主播
    let mut config = config_manager.load()?;
    let pos = config
        .anchors
        .iter()
        .position(|a| a.id == anchor_id)
        .ok_or_else(|| AppError::config(format!("未找到主播ID: {}", anchor_id)))?;
    let anchor = &mut config.anchors[pos];

    // 2. 确保有 room_id（若无，则从 url 提取）
    let room_id = if anchor.room_id.is_empty() {
        if let Some(rid) = MissevanClient::extract_room_id(&anchor.url) {
            anchor.room_id = rid.clone();
            rid
        } else {
            return Err(AppError::config("无法从 URL 提取房间号"));
        }
    } else {
        anchor.room_id.clone()
    };

    // 3. 调用猫耳 API 获取最新信息
    let client = MissevanClient::new()?;
    let profile = client.get_anchor_profile(&room_id).await?;

    // 4. 更新主播配置（名称和头像 URL）
    anchor.name = profile.name;
    avatar_cache
        .lock()
        .await
        .insert(anchor.id.clone(), profile.avatar_url.clone());

    // 5. 保存更改（覆盖原主播文件）——保存前不设 avatar_url（头像不落盘约定，
    //    与 update_anchor 一致）；保存后单独装配返回体，保证前端拿到最新头像。
    config_manager.add_anchor(anchor)?; // 使用现有方法覆盖写入

    // 返回体必须携带最新头像：avatar_cache 缓存的是「新」URL，若返回体不设置
    // avatar_url，前端会用 avatar_url=None 覆盖列表项 → 头像变默认占位图，
    // 仅重载页面（get_anchors 命中缓存）才恢复。
    anchor.avatar_url = Some(profile.avatar_url);

    // 6. 发送通知
    dispatcher
        .info(
            "anchor_refresh_ok",
            "刷新成功",
            format!("主播 “{}” 信息已更新", anchor.name),
        )
        .await;

    Ok(anchor.clone())
}

#[tauri::command]
pub async fn update_anchor(
    state: State<'_, RecorderState>,
    config_manager: State<'_, Arc<ConfigManager>>,
    dispatcher: State<'_, Arc<NotificationDispatcher>>,
    avatar_cache: State<'_, AvatarCache>,
    anchor_id: String,
    mut anchor: AnchorConfig,
) -> Result<(), AppError> {
    anchor.id = anchor_id.clone();

    // 1. 强制从 URL 提取 room_id（忽略前端传入的值）
    if let Some(new_room_id) = MissevanClient::extract_room_id(&anchor.url) {
        anchor.room_id = new_room_id;
    } else {
        let err = AppError::config("无法从主页地址提取房间号")
            .with_suggestion("请确保URL格式为 https://fm.missevan.com/live/数字");
        dispatcher
            .error("anchor_update_failed", "更新失败", err.message.clone())
            .await;
        return Err(err);
    }

    // 2. 备份旧数据（用于回滚）
    let config = config_manager.load()?;
    let old_anchor = config
        .anchors
        .iter()
        .find(|a| a.id == anchor_id)
        .ok_or_else(|| AppError::config(format!("主播 {} 不存在", anchor_id)))?
        .clone();

    // 3. 调用 API 获取最新主播名和头像（别名非空时保留，留空使用官方名称）
    let client = MissevanClient::new()?;
    let profile = match client.get_anchor_profile(&anchor.room_id).await {
        Ok(p) => p,
        Err(e) => {
            dispatcher
                .error("anchor_update_failed", "获取主播信息失败", e.to_string())
                .await;
            return Err(e);
        }
    };
    if anchor.name.trim().is_empty() {
        anchor.name = profile.name;
    }
    // 头像存入缓存，不保存到文件
    avatar_cache
        .lock()
        .await
        .insert(anchor.id.clone(), profile.avatar_url);
    anchor.avatar_url = None;

    // 4. 删除旧文件，写入新文件（失败则回滚）
    config_manager.remove_anchor(&anchor_id)?;
    match config_manager.add_anchor(&anchor) {
        Ok(()) => {
            // 5. 「关闭检测与自动录制」= 结束该主播当前录制（保存即停语义）。
            // 根因：进行中的录制由 monitor_recording 管理，其轮询用的是启动时
            // 传入的主播快照（含 enable_check 旧值）——检测循环（loop.rs）每轮
            // 读最新配置，只会决定「不再新开录制」，不会停止已进行的录制；
            // 录制是否停止须在此（配置变更点）显式处理。
            let stopped = stop_recording_if_check_disabled(
                &state.state,
                &anchor_id,
                old_anchor.enable_check,
                anchor.enable_check,
            )
            .await;
            if stopped {
                tracing::info!("关闭检测开关，已停止录制: {}", anchor_id);
                dispatcher
                    .info(
                        "anchor_update_stop",
                        "录制已停止",
                        format!(
                            "已关闭「启用检测与自动录制」，主播“{}”的录制已停止",
                            anchor.name
                        ),
                    )
                    .await;
            }
            dispatcher
                .info(
                    "anchor_update_ok",
                    "更新成功",
                    format!("主播“{}”已更新", anchor.name),
                )
                .await;
            tracing::info!("更新主播: {} -> {}", anchor_id, anchor.name);
            Ok(())
        }
        Err(e) => {
            if let Err(rollback_err) = config_manager.add_anchor(&old_anchor) {
                tracing::error!("回滚主播配置失败: {:?}", rollback_err);
            }
            dispatcher
                .error("anchor_update_failed", "更新失败", e.message.clone())
                .await;
            Err(e)
        }
    }
}

/// 获取主播公开资料（名称/头像/简介），供设置面板「主播简介」显示
#[tauri::command]
pub async fn get_anchor_profile(room_id: String) -> Result<AnchorProfile, AppError> {
    let client = MissevanClient::new()?;
    client.get_anchor_profile(&room_id).await
}

/// 房间号去重判断（add_anchor 命令使用；纯函数便于单测）：
/// 现有主播中是否存在 trim 后相同的**非空** room_id。
///
/// 仅 add 路径检查：update_anchor / refresh_anchor 走 ConfigManager 直接保存，
/// 不受此限制——更新 URL 导致房间号变化属合法编辑，不应被去重拦截
/// （若被拦截则用户无法把主播改指向另一房间）。
fn room_id_already_exists(anchors: &[AnchorConfig], room_id: &str) -> bool {
    let room_id = room_id.trim();
    !room_id.is_empty()
        && anchors
            .iter()
            .any(|a| !a.room_id.trim().is_empty() && a.room_id.trim() == room_id)
}

/// 检测开关 true→false 切换时，立即停止该主播正在进行的录制（「保存即停」语义）。
///
/// - 仅当旧值开启、新值关闭时检查（其余组合不动作——重新开启/保持关闭均不停）；
/// - 存在录制任务才停止（remove_task + cancel_token，与 stop_recording 同一模式；
///   monitor_recording 收到取消后自行完成 FFmpeg 优雅退出与状态清理）；
/// - 返回是否实际执行了停止（供调用方发通知/日志）。
async fn stop_recording_if_check_disabled(
    app_state: &AppStateHandle,
    anchor_id: &str,
    old_enable_check: bool,
    new_enable_check: bool,
) -> bool {
    if !(old_enable_check && !new_enable_check) {
        return false;
    }
    let mut guard = app_state.lock().await;
    match guard.remove_task(anchor_id) {
        Some(task) => {
            task.cancel_token.cancel();
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::state::app_state::{create_app_state_handle, Task};
    use tokio_util::sync::CancellationToken;

    /// 构造一个已取消即完成的录制任务（anchor_id 已在 tasks 表中）
    async fn insert_fake_task(app_state: &AppStateHandle, anchor_id: &str) -> CancellationToken {
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        let handle = tokio::spawn(async move {
            cancel_clone.cancelled().await;
        });
        app_state.lock().await.insert_task(
            anchor_id.to_string(),
            Task {
                anchor_id: anchor_id.to_string(),
                cancel_token: cancel.clone(),
                handle,
                anchor_name: format!("主播{}", anchor_id),
                room_id: format!("room-{}", anchor_id),
                output_path: format!("/out/{}.m4a", anchor_id),
                started_at: std::time::Instant::now(),
                pid: None,
            },
        );
        cancel
    }

    // ── 关闭检测开关 = 停止录制（「保存即停」）──

    #[tokio::test]
    async fn disabling_check_during_recording_stops_task() {
        let app_state = create_app_state_handle();
        let cancel = insert_fake_task(&app_state, "a1").await;

        let stopped =
            stop_recording_if_check_disabled(&app_state, "a1", true, false).await;
        assert!(stopped, "true→false 且录制中：应停止");
        assert!(
            !app_state.lock().await.is_recording("a1"),
            "任务应从 tasks 表移除"
        );
        assert!(cancel.is_cancelled(), "取消令牌应已触发（monitor 据此收尾）");
    }

    #[tokio::test]
    async fn disabling_check_without_recording_does_nothing() {
        let app_state = create_app_state_handle();
        // 未录制：返回 false，不报错
        let stopped =
            stop_recording_if_check_disabled(&app_state, "a1", true, false).await;
        assert!(!stopped);
    }

    #[tokio::test]
    async fn re_enabling_check_does_not_stop_recording() {
        let app_state = create_app_state_handle();
        insert_fake_task(&app_state, "a1").await;
        // false→true（重新开启）：不停止
        let stopped =
            stop_recording_if_check_disabled(&app_state, "a1", false, true).await;
        assert!(!stopped);
        assert!(app_state.lock().await.is_recording("a1"));
    }

    #[tokio::test]
    async fn check_stays_enabled_does_not_stop_recording() {
        let app_state = create_app_state_handle();
        insert_fake_task(&app_state, "a1").await;
        // true→true（保持开启）：不停止
        let stopped =
            stop_recording_if_check_disabled(&app_state, "a1", true, true).await;
        assert!(!stopped);
        assert!(app_state.lock().await.is_recording("a1"));
    }

    #[tokio::test]
    async fn already_disabled_stays_noop() {
        let app_state = create_app_state_handle();
        insert_fake_task(&app_state, "a1").await;
        // false→false（本就关闭）：不停止
        let stopped =
            stop_recording_if_check_disabled(&app_state, "a1", false, false).await;
        assert!(!stopped);
        assert!(app_state.lock().await.is_recording("a1"));
    }

    fn anchor(id: &str, room_id: &str) -> AnchorConfig {
        AnchorConfig {
            id: id.to_string(),
            name: format!("主播{}", id),
            url: format!("https://fm.missevan.com/live/{}", room_id),
            room_id: room_id.to_string(),
            proxy: None,
            cookie: None,
            enable_check: true,
            avatar_url: None,
            tags: Vec::new(),
        }
    }

    // ── 双录防御 #1：add_anchor 房间号去重 ──

    #[test]
    fn add_anchor_rejects_duplicate_room_id() {
        let anchors = vec![anchor("a1", "1"), anchor("a2", "2")];
        // 相同 room_id → 拒绝
        assert!(room_id_already_exists(&anchors, "1"), "相同房间号应命中去重");
        // trim 后比较：前后空白 / tab 差异同样命中
        assert!(room_id_already_exists(&anchors, " 1 "), "前导/尾随空格应命中");
        assert!(room_id_already_exists(&anchors, "1\t"), "tab 差异应命中");
        // 不同 room_id → 放行
        assert!(!room_id_already_exists(&anchors, "3"), "不同房间号应放行");
    }

    #[test]
    fn empty_room_id_never_matches() {
        let anchors = vec![anchor("a1", ""), anchor("a2", "2")];
        // 空 room_id 不构成冲突（新添加的空房间号也不会命中，避免误拦）
        assert!(!room_id_already_exists(&anchors, ""), "空房间号不应命中");
        assert!(!room_id_already_exists(&anchors, "   "), "纯空白不应命中");
        assert!(room_id_already_exists(&anchors, "2"), "非空相同房间号应命中");
    }
}
