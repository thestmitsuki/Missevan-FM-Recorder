use tauri::State;
use tracing::info;

use crate::domain::config::manager::ConfigManager;
use crate::domain::config::model::AnchorConfig;
use crate::domain::config::model::RecordingStatus;
use crate::domain::detector::merge_live_state;
use crate::domain::detector::r#loop::DetectionLoop;
use crate::domain::spider::{AnchorProfile, MissevanClient};
use crate::infrastructure::error::types::{AppError, ErrorCategory, ErrorSeverity};
use crate::infrastructure::notification::dispatcher::NotificationDispatcher;
use crate::infrastructure::state::app_state::AppStateHandle;
use crate::infrastructure::state::app_state::AvatarCache;
use crate::infrastructure::state::app_state::AvatarNegativeCache;
use crate::infrastructure::state::app_state::RecorderState;
use futures_util::StreamExt;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// O2：头像拉取并发上限（≤8，防止并发打爆 API / 代理）
const AVATAR_FETCH_CONCURRENCY: usize = 8;
/// O2：头像拉取失败负缓存时长（5 分钟内同一主播不再重试，避免 API 抖动期
/// 每次 get_anchors 都重试全部失败项）
const AVATAR_NEGATIVE_CACHE_TTL: Duration = Duration::from_secs(300);
/// L3 审查跟进：主播 cookie 长度上限（字节）。探针请求会把 cookie 原样写入
/// `Cookie` 请求头——超长值会撑大配置并触发请求头超限风险；防异常输入，
/// 超出即拒绝保存（add_anchor / update_anchor 共用）。
const COOKIE_MAX_LEN: usize = 4096;

/// cookie 长度校验（纯函数便于单测）：None（未填写）恒合法；
/// Some 时字节长度 ≤ `COOKIE_MAX_LEN` 才合法，超出返回明确错误。
fn validate_cookie_len(cookie: Option<&str>) -> Result<(), AppError> {
    if let Some(c) = cookie {
        if c.len() > COOKIE_MAX_LEN {
            return Err(AppError::config(format!(
                "Cookie 过长（{} 字节，上限 {} 字节）",
                c.len(),
                COOKIE_MAX_LEN
            )));
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn get_anchors(
    config_manager: State<'_, Arc<ConfigManager>>,
    avatar_cache: State<'_, AvatarCache>,
    negative_cache: State<'_, AvatarNegativeCache>,
) -> Result<Vec<AnchorConfig>, AppError> {
    let mut config = config_manager.load()?;
    // 网络分类接线：全局代理 + api_timeout_secs 统一生效（头像/简介请求同 client）
    let client = MissevanClient::from_config(&config.global)?;
    let now = Instant::now();

    // 缓存快照（避免逐主播反复上锁）+ 顺带清理过期负缓存
    let (avatar_snapshot, negative_snapshot) = {
        let av = avatar_cache.lock().await;
        let neg = negative_cache.lock().await;
        (av.clone(), neg.clone())
    };
    negative_cache.lock().await.retain(|_, until| *until > now);

    // 规划需要网络请求的主播（正缓存命中 / 负缓存冷却期 / 无法提取 room_id
    // 的直接跳过；命中正缓存的在此直接写回 avatar_url）
    let fetches = plan_avatar_fetches(
        &mut config.anchors,
        &avatar_snapshot,
        &negative_snapshot,
        now,
    );

    // O2：并发拉取（buffer_unordered 限流 AVATAR_FETCH_CONCURRENCY），结果
    // 携带原索引按顺序写回——冷启动/API 慢时从 N×10s（串行最坏 N 个超时周期）
    // 收敛到 ≤10s（最坏一个超时周期）。失败写入负缓存，TTL 内不再重试。
    if !fetches.is_empty() {
        // 闭包内仅借引用（&MissevanClient 是 Copy，可被多个 future 捕获；
        // reqwest::Client 为 Sync，跨并发 future 共享安全）
        let client_ref = &client;
        let stream = futures_util::stream::iter(fetches.into_iter().map(|(idx, room_id)| {
            async move {
                let result = client_ref
                    .get_anchor_profile(&room_id)
                    .await
                    .map(|p| p.avatar_url);
                (idx, result)
            }
        }))
        .buffer_unordered(AVATAR_FETCH_CONCURRENCY);
        let results: Vec<(usize, Result<String, AppError>)> = stream.collect().await;

        // 批量写回（成功 → 正缓存；失败 → 负缓存）
        let mut av = avatar_cache.lock().await;
        let mut neg = negative_cache.lock().await;
        let now = Instant::now();
        for (idx, result) in results {
            if let Some(anchor) = config.anchors.get_mut(idx) {
                apply_avatar_result(anchor, &mut av, &mut neg, &result, now);
            }
        }
    }

    Ok(config.anchors)
}

/// 规划头像拉取（纯逻辑便于单测）：返回需要网络请求的 (索引, room_id) 列表；
/// 命中正缓存的直接写回 avatar_url；负缓存冷却期 / 无法提取 room_id 的主播跳过。
fn plan_avatar_fetches(
    anchors: &mut [AnchorConfig],
    avatar_cache: &HashMap<String, String>,
    negative_cache: &HashMap<String, Instant>,
    now: Instant,
) -> Vec<(usize, String)> {
    let mut fetches = Vec::new();
    for (idx, anchor) in anchors.iter_mut().enumerate() {
        // 1. 正缓存命中：直接使用，不发请求
        if let Some(avatar) = avatar_cache.get(&anchor.id) {
            anchor.avatar_url = Some(avatar.clone());
            continue;
        }
        // 2. 负缓存冷却期：失败过的主播短时间不重试
        if in_negative_cooldown(negative_cache.get(&anchor.id), now) {
            continue;
        }
        // 3. 提取 room_id（与旧逻辑一致：room_id 优先，URL 兜底）；无法提取则跳过
        let room_id = if !anchor.room_id.is_empty() {
            anchor.room_id.clone()
        } else if let Some(rid) = MissevanClient::extract_room_id(&anchor.url) {
            rid
        } else {
            continue;
        };
        fetches.push((idx, room_id));
    }
    fetches
}

/// 是否处于头像失败冷却期（纯函数便于单测）：`failed_until` 晚于当前时刻即冷却中
fn in_negative_cooldown(failed_until: Option<&Instant>, now: Instant) -> bool {
    failed_until.is_some_and(|until| *until > now)
}

/// 应用单次头像拉取结果（纯逻辑便于单测）：成功写回 avatar 并缓存正结果；
/// 失败保留原值并写入负缓存（TTL 内不再重试）。
fn apply_avatar_result(
    anchor: &mut AnchorConfig,
    avatar_cache: &mut HashMap<String, String>,
    negative_cache: &mut HashMap<String, Instant>,
    result: &Result<String, AppError>,
    now: Instant,
) {
    match result {
        Ok(url) => {
            anchor.avatar_url = Some(url.clone());
            avatar_cache.insert(anchor.id.clone(), url.clone());
        }
        Err(e) => {
            tracing::warn!("获取主播 {} 头像失败: {}", anchor.id, e);
            negative_cache.insert(anchor.id.clone(), now + AVATAR_NEGATIVE_CACHE_TTL);
        }
    }
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

    // 2. 快照任务表与直播缓存（M5 修复：消除双层嵌套锁 + 长持锁）——
    //    两把锁分别独立获取并立即释放（各自作用域结束即解锁，绝不同时持有），
    //    遍历组装在锁外完成：此前 app_state + live_cache 同时持有并遍历全部
    //    主播，会阻塞同锁序的检测循环状态推送 / stop / remove 等操作整个遍历期间
    let recording_ids = {
        let app_state = state.state.lock().await;
        app_state.recording_anchor_ids()
    };
    let live_snapshot = {
        let cache = live_cache.lock().await;
        cache.clone()
    };

    // 3. 锁外遍历配置中的主播组装状态（结构不变：anchor_id / is_recording / is_live）
    let statuses = build_statuses(&config.anchors, &recording_ids, &live_snapshot);

    info!("[get_recording_status] 即将返回 {} 条状态", statuses.len());
    Ok(statuses)
}

/// 组装录制状态列表（纯逻辑便于单测）：从「任务 id 快照 + 直播缓存快照 +
/// 主播配置」生成 RecordingStatus。锁外调用——锁内只负责取快照（见
/// get_recording_status），本函数不触碰任何共享状态。
///
/// 双重验证归并：API 判定 || 录制中（API 判离线但录制进行中 → 保持直播中）
/// 由 `merge_live_state` 统一处理（domain::detector）。
fn build_statuses(
    anchors: &[AnchorConfig],
    recording_ids: &HashSet<String>,
    live_snapshot: &HashMap<String, bool>,
) -> Vec<RecordingStatus> {
    anchors
        .iter()
        .map(|anchor| {
            let is_recording = recording_ids.contains(&anchor.id);
            RecordingStatus {
                anchor_id: anchor.id.clone(),
                is_recording,
                is_live: merge_live_state(
                    live_snapshot.get(&anchor.id).copied().unwrap_or(false),
                    is_recording,
                ),
            }
        })
        .collect()
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
    } else if guard.cancel_pending_start(&anchor_id) {
        // pre_record_delay 窗口内的启动尚未注册进 tasks——从 pending_starts 取消
        tracing::info!("已取消延迟中的录制启动: {}", anchor_id);
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
    // L3 审查跟进：cookie 长度上限校验（防异常输入撑大配置 / 请求头超限），
    // 校验失败直接拒绝，不进入后续网络调用与写盘
    if let Err(err) = validate_cookie_len(anchor.cookie.as_deref()) {
        dispatcher
            .error("anchor_add_failed", "Cookie 过长", err.message.clone())
            .await;
        return Err(err);
    }

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
    let config = config_manager.load()?;
    // 网络分类接线：全局代理 + api_timeout_secs
    let client = MissevanClient::from_config(&config.global)?;
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

/// 删除主播：停止其录制任务（含延迟窗口内的启动）→ 清理直播状态缓存 →
/// 清理按主播维度的运行时状态（R3/L10：崩溃熔断/录制序号/429 限流冷却/头像
/// 正负缓存）→ 从配置文件中删除。
#[tauri::command]
pub async fn remove_anchor(
    state: State<'_, RecorderState>,
    config_manager: State<'_, Arc<ConfigManager>>,
    dispatcher: State<'_, Arc<NotificationDispatcher>>,
    live_cache: State<'_, Arc<Mutex<HashMap<String, bool>>>>,
    avatar_cache: State<'_, AvatarCache>,
    negative_cache: State<'_, AvatarNegativeCache>,
    detection_loop: State<'_, Arc<DetectionLoop>>,
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
    // 2b. pre_record_delay 窗口内的启动尚未注册进 tasks——同样取消
    if app_state.cancel_pending_start(&id) {
        tracing::info!("已取消延迟中的录制启动: {}", id);
    }
    // 2c. R3/L10：清理该主播其余按主播维度的运行时状态（崩溃熔断条目/
    // 录制序号/延迟启动注册兜底）——此前仅复位熔断计数，条目随删除回收
    app_state.prune_anchor(&id);
    drop(app_state); // 尽早释放锁，避免后续阻塞

    // 3. 清理直播状态缓存
    {
        let mut cache = live_cache.lock().await;
        cache.remove(&id);
    }

    // 3b. R3/L10：429 限流冷却（DetectionLoop）与头像正/负缓存同样随主播
    // 删除清理——避免长期运行累积无主条目（结构虽有界，主播已删除则无保留
    // 价值；重新添加同 id 主播时冷却/头像从零开始）
    detection_loop.prune_anchor_state(&id).await;
    {
        avatar_cache.lock().await.remove(&id);
        negative_cache.lock().await.remove(&id);
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

    // 3. 调用猫耳 API 获取最新信息（网络分类接线：全局代理 + api_timeout_secs）
    let client = MissevanClient::from_config(&config.global)?;
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

    // L3 审查跟进：cookie 长度上限校验（与 add_anchor 同规则），失败直接拒绝
    if let Err(err) = validate_cookie_len(anchor.cookie.as_deref()) {
        dispatcher
            .error("anchor_update_failed", "Cookie 过长", err.message.clone())
            .await;
        return Err(err);
    }

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

    // 3. 调用 API 获取最新主播名和头像（L2/M6 软门槛：网络失败**不阻断本地
    //    保存**——仅改别名/Cookie/代理/标签/检测开关时，断网也必须能落盘；
    //    网络只用于名称/头像的自动补全，失败降级为「保存本地 + 警告通知」）
    let config = config_manager.load()?;
    // 网络分类接线：全局代理 + api_timeout_secs
    let client = MissevanClient::from_config(&config.global)?;
    let network_ok = {
        let profile = client.get_anchor_profile(&anchor.room_id).await;
        let mut avatar_guard = avatar_cache.lock().await;
        apply_profile_result(
            &mut anchor,
            &old_anchor.name,
            profile,
            &mut avatar_guard,
        )
    };
    // 头像不落盘约定（与 add_anchor / refresh_anchor 一致）；失败路径不动
    // 头像缓存（保留上次成功值，不因一次断网清空）
    anchor.avatar_url = None;

    // 4. 单次原子覆盖写入（S4a）：add_anchor 内部为「临时文件 + fsync + rename」
    //    直接覆盖旧文件——不再先删后写，消除两步之间的崩溃窗口（曾可致主播配置
    //    永久丢失，架构审查 TOP3）
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
            if network_ok {
                dispatcher
                    .info(
                        "anchor_update_ok",
                        "更新成功",
                        format!("主播“{}”已更新", anchor.name),
                    )
                    .await;
            } else {
                // L2/M6 部分成功：本地已保存、网络校验失败——用 Warning 级
                // 通知明确「已保存 + 网络失败」双重语义（前端零改动：错误
                // 通知只用于整体失败路径；此处 Ok 返回使设置面板正常关闭，
                // 警告通知由 app:notification 全局展示）
                dispatcher
                    .warning(
                        "anchor_update_partial",
                        "已保存，但网络校验失败",
                        format!(
                            "主播“{}”的配置已保存到本地；无法联网获取最新名称/头像，可在网络恢复后点击「刷新信息」",
                            anchor.name
                        ),
                    )
                    .await;
            }
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

/// L2（M6）软门槛：网络获取主播信息的应用逻辑（纯函数便于单测）。
///
/// - 成功：名称留空时用官方名称（用户别名非空则保留）；头像写入缓存；
/// - 失败：**不阻断**——名称留空时回退旧名称（避免保存空名），头像缓存
///   不动（保留上次成功值），仅记 warn 日志；
/// - 返回是否网络成功（供 update_anchor 决定成功通知文案）。
fn apply_profile_result(
    anchor: &mut AnchorConfig,
    fallback_name: &str,
    profile: Result<AnchorProfile, AppError>,
    avatar_cache: &mut HashMap<String, String>,
) -> bool {
    match profile {
        Ok(p) => {
            if anchor.name.trim().is_empty() {
                anchor.name = p.name;
            }
            avatar_cache.insert(anchor.id.clone(), p.avatar_url);
            true
        }
        Err(e) => {
            tracing::warn!(
                "[update_anchor] 网络获取主播 {} 信息失败（本地保存不受影响，名称/头像未自动更新）: {}",
                anchor.id,
                e
            );
            if anchor.name.trim().is_empty() {
                anchor.name = fallback_name.to_string();
            }
            false
        }
    }
}

/// 获取主播公开资料（名称/头像/简介），供设置面板「主播简介」显示
/// （网络分类接线：全局代理 + api_timeout_secs 经配置构建 client）
#[tauri::command]
pub async fn get_anchor_profile(
    room_id: String,
    config_manager: State<'_, Arc<ConfigManager>>,
) -> Result<AnchorProfile, AppError> {
    let config = config_manager.load()?;
    let client = MissevanClient::from_config(&config.global)?;
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
        // pre_record_delay 窗口内的启动：同样取消（延迟结束复检也会放弃，
        // 此处立即取消，避免主播已关检测还要空等剩余延迟）
        None => guard.cancel_pending_start(anchor_id),
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

    // ── M5：get_recording_status 锁外组装（build_statuses 纯函数）──

    #[test]
    fn build_statuses_marks_recording_and_merges_live_state() {
        let anchors = vec![anchor("a1", "1"), anchor("a2", "2"), anchor("a3", "3")];
        // a1 在录制；a2 仅 API 判直播；a3 均无
        let recording_ids: HashSet<String> = ["a1".to_string()].into_iter().collect();
        let mut live_snapshot = HashMap::new();
        live_snapshot.insert("a2".to_string(), true);
        live_snapshot.insert("a3".to_string(), false);

        let statuses = build_statuses(&anchors, &recording_ids, &live_snapshot);
        assert_eq!(statuses.len(), 3);
        // 录制中 → is_recording=true；API 离线但录制中 → 保持直播（归并）
        assert_eq!(statuses[0].anchor_id, "a1");
        assert!(statuses[0].is_recording);
        assert!(statuses[0].is_live, "录制中即视为直播中（merge_live_state）");
        // 仅 API 判直播 → 直播但不录制
        assert!(statuses[1].is_live);
        assert!(!statuses[1].is_recording);
        // 均无 → 全部 false
        assert!(!statuses[2].is_live);
        assert!(!statuses[2].is_recording);
    }

    #[test]
    fn build_statuses_handles_empty_and_unknown_anchors() {
        // 空主播列表 → 空结果
        assert!(build_statuses(&[], &HashSet::new(), &HashMap::new()).is_empty());
        // 直播快照中无该主播记录 → 按 false 处理，不 panic
        let anchors = vec![anchor("x", "9")];
        let statuses = build_statuses(&anchors, &HashSet::new(), &HashMap::new());
        assert_eq!(statuses.len(), 1);
        assert!(!statuses[0].is_live);
        assert!(!statuses[0].is_recording);
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

    // ── O2：头像拉取规划 / 失败负缓存 ──

    #[test]
    fn plan_avatar_fetches_uses_cache_and_skips_cooldown() {
        let now = Instant::now();
        let mut anchors = vec![
            anchor("a1", "1"), // 正缓存命中 → 直接写回
            anchor("a2", "2"), // 负缓存冷却 → 跳过
            anchor("a3", "3"), // 负缓存已过期 → 仍入列
            anchor("a4", ""),  // room_id 空且 URL 无法提取 → 跳过
        ];
        let mut avatar_cache = HashMap::new();
        avatar_cache.insert("a1".into(), "https://avatar/a1.png".into());
        let mut negative_cache = HashMap::new();
        negative_cache.insert("a2".into(), now + Duration::from_secs(60));
        negative_cache.insert("a3".into(), now - Duration::from_secs(60));

        let fetches = plan_avatar_fetches(&mut anchors, &avatar_cache, &negative_cache, now);
        assert_eq!(
            anchors[0].avatar_url.as_deref(),
            Some("https://avatar/a1.png"),
            "命中正缓存直接写回 avatar_url"
        );
        assert_eq!(
            fetches,
            vec![(2, "3".to_string())],
            "仅冷却期外且可提取 room_id 的主播入列"
        );
    }

    #[test]
    fn plan_avatar_fetches_room_id_preferred_over_url() {
        let now = Instant::now();
        let mut anchors = vec![
            anchor("a1", "1001"),
            {
                let mut a = anchor("a2", "");
                a.url = "https://fm.missevan.com/live/2002".into();
                a
            },
        ];
        let fetches = plan_avatar_fetches(&mut anchors, &HashMap::new(), &HashMap::new(), now);
        assert_eq!(
            fetches,
            vec![(0, "1001".to_string()), (1, "2002".to_string())],
            "room_id 优先，URL 提取兜底"
        );
    }

    #[test]
    fn negative_cooldown_only_before_deadline() {
        let now = Instant::now();
        assert!(
            in_negative_cooldown(Some(&(now + Duration::from_secs(5))), now),
            "未到截止时刻 → 冷却中"
        );
        assert!(
            !in_negative_cooldown(Some(&now), now),
            "恰好在截止时刻 → 不冷却"
        );
        assert!(
            !in_negative_cooldown(Some(&(now - Duration::from_secs(1))), now),
            "已过截止时刻 → 不冷却"
        );
        assert!(!in_negative_cooldown(None, now), "无记录 → 不冷却");
    }

    #[test]
    fn apply_avatar_result_success_and_failure() {
        let now = Instant::now();
        let mut a = anchor("a1", "1");
        let mut avatar_cache = HashMap::new();
        let mut negative_cache = HashMap::new();

        // 成功：写回 avatar + 正缓存
        apply_avatar_result(
            &mut a,
            &mut avatar_cache,
            &mut negative_cache,
            &Ok("https://x/a.png".to_string()),
            now,
        );
        assert_eq!(a.avatar_url.as_deref(), Some("https://x/a.png"));
        assert_eq!(
            avatar_cache.get("a1").map(String::as_str),
            Some("https://x/a.png")
        );
        assert!(negative_cache.is_empty(), "成功不写负缓存");

        // 失败：保留原 avatar + 写负缓存（TTL 内冷却）
        apply_avatar_result(
            &mut a,
            &mut avatar_cache,
            &mut negative_cache,
            &Err(AppError::network("timeout")),
            now,
        );
        assert_eq!(
            a.avatar_url.as_deref(),
            Some("https://x/a.png"),
            "失败不清空已有头像"
        );
        let until = negative_cache.get("a1").unwrap();
        assert!(*until > now, "负缓存截止时刻应在未来");
        assert!(
            *until <= now + AVATAR_NEGATIVE_CACHE_TTL,
            "负缓存截止时刻不超过 TTL"
        );
    }

    // ── L2（M6）：update_anchor 网络软门槛——网络失败不阻断本地保存 ──

    fn profile(name: &str, avatar: &str) -> AnchorProfile {
        AnchorProfile {
            name: name.to_string(),
            avatar_url: avatar.to_string(),
            introduction: None,
        }
    }

    #[test]
    fn apply_profile_success_uses_official_name_when_alias_empty() {
        let mut a = anchor("a1", "1");
        a.name.clear();
        let mut cache = HashMap::new();
        let ok = apply_profile_result(
            &mut a,
            "旧名",
            Ok(profile("官方名", "https://x/a.png")),
            &mut cache,
        );
        assert!(ok, "网络成功应返回 true");
        assert_eq!(a.name, "官方名", "别名留空 → 使用官方名称");
        assert_eq!(
            cache.get("a1").map(String::as_str),
            Some("https://x/a.png"),
            "头像应写入缓存"
        );
    }

    #[test]
    fn apply_profile_success_keeps_custom_alias() {
        let mut a = anchor("a1", "1");
        a.name = "我的别名".to_string();
        let mut cache = HashMap::new();
        let ok = apply_profile_result(
            &mut a,
            "旧名",
            Ok(profile("官方名", "https://x/a.png")),
            &mut cache,
        );
        assert!(ok);
        assert_eq!(a.name, "我的别名", "用户别名非空 → 保留");
        assert_eq!(
            cache.get("a1").map(String::as_str),
            Some("https://x/a.png")
        );
    }

    #[test]
    fn apply_profile_failure_keeps_custom_alias_and_cache() {
        let mut a = anchor("a1", "1");
        a.name = "我的别名".to_string();
        let mut cache = HashMap::new();
        cache.insert("a1".into(), "https://old/avatar.png".into());
        let ok = apply_profile_result(
            &mut a,
            "旧名",
            Err(AppError::network("timeout")),
            &mut cache,
        );
        assert!(!ok, "网络失败应返回 false（部分成功语义）");
        assert_eq!(a.name, "我的别名", "网络失败不清空用户别名");
        assert_eq!(
            cache.get("a1").map(String::as_str),
            Some("https://old/avatar.png"),
            "失败路径不动头像缓存（保留上次成功值）"
        );
    }

    #[test]
    fn apply_profile_failure_falls_back_to_old_name_when_alias_empty() {
        let mut a = anchor("a1", "1");
        a.name.clear();
        let mut cache = HashMap::new();
        let ok = apply_profile_result(
            &mut a,
            "旧名",
            Err(AppError::network("timeout")),
            &mut cache,
        );
        assert!(!ok);
        assert_eq!(
            a.name, "旧名",
            "别名留空且网络失败 → 回退旧名称，避免保存空名"
        );
        assert!(cache.is_empty(), "失败路径不得写入头像缓存");
    }

    // ── L3 审查跟进：cookie 长度上限校验 ──

    #[test]
    fn cookie_length_validation_accepts_none_and_short_values() {
        // None（未填写）恒合法
        assert!(validate_cookie_len(None).is_ok());
        // 空串 / 边界值（恰 4096 字节）合法
        assert!(validate_cookie_len(Some("")).is_ok());
        assert!(validate_cookie_len(Some(&"a".repeat(COOKIE_MAX_LEN))).is_ok());
        assert!(validate_cookie_len(Some("token=abc123")).is_ok());
    }

    #[test]
    fn cookie_length_validation_rejects_oversized_values() {
        let err = validate_cookie_len(Some(&"a".repeat(COOKIE_MAX_LEN + 1))).unwrap_err();
        assert!(err.message.contains("Cookie 过长"), "错误: {}", err.message);
        // 消息应包含实际长度与上限（便于用户理解）
        assert!(err.message.contains(&(COOKIE_MAX_LEN + 1).to_string()));
        assert!(err.message.contains(&COOKIE_MAX_LEN.to_string()));
    }
}
