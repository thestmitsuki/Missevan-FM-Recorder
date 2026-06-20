use crate::checker;
use crate::config::model::{normalize_path, AnchorConfig, Config};
use crate::error::AppError;
use crate::recorder;
use crate::spider;
use crate::state::RecorderState;
use crate::utils::check_ffmpeg;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::Emitter;
use tauri::State;
use tokio_util::sync::CancellationToken;
use walkdir::WalkDir;

// -------- 公共数据结构 ----------
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnchorInfo {
    pub id: String,
    pub name: String,
    pub avatar: Option<String>,
    pub is_live: bool,
    pub is_recording: bool,
    pub room_url: String,
    pub title: String,
    pub introduction: String,
    pub config: AnchorConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioFileInfo {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub modified: String,
    pub folder: String,  // 所属主播文件夹名
}

// -------- 辅助函数（私有） ----------

async fn is_anchor_recording(state: &State<'_, RecorderState>, anchor_id: &str) -> bool {
    let guard = state.state.lock().await;
    guard.tasks.contains_key(anchor_id)
}

/// 内部：开始录制（不持锁做 I/O）
async fn start_anchor_recording_internal(
    state: &State<'_, RecorderState>,
    anchor_id: &str,
    app_handle: tauri::AppHandle,
) -> Result<(), AppError> {
    if is_anchor_recording(state, anchor_id).await {
        return Err(AppError::Recorder("该主播已在录制中".to_string()));
    }

    // === 第1步：获取配置（持锁，短暂）===
    let (config, anchor) = {
        let guard = state.state.lock().await;
        let cfg = guard.config.clone();
        let anc = cfg
            .get_anchor(anchor_id)
            .ok_or_else(|| AppError::AnchorNotFound(anchor_id.to_string()))?
            .clone();
        (cfg, anc)
    };

    // === 第2步：FFmpeg 检查（不持锁）===
    let ffmpeg_path = match check_ffmpeg(config.ffmpeg_path.as_deref()).await {
        Ok(p) => p,
        Err(e) => {
            let notif = checker::CheckNotification {
                level: "error".to_string(),
                message: format!("FFmpeg 未找到，无法录制 {}。\n{}", anchor.name, e),
                check_name: "ffmpeg".to_string(),
            };
            let _ = app_handle.emit("check-notification", notif);
            return Err(AppError::FfmpegNotFound(e.to_string()));
        }
    };

    // === 第3步：检测直播状态（不持锁）===
    let live_info = spider::get_live_info(
        &anchor.url,
        anchor.proxy.as_deref(),
        anchor.cookie.as_deref(),
    )
    .await?;
    if !live_info.is_live {
        return Err(AppError::Recorder("主播未开播".to_string()));
    }

    // === 第4步：启动录制任务（不持锁）===
    let cancel_token = CancellationToken::new();
    let child_token = cancel_token.child_token();
    let config_clone = config.clone();
    let anchor_name = anchor.name.clone();
    let title = live_info.title.clone();
    let ffmpeg_path_clone = ffmpeg_path.clone();
    let anchor_id_clone = anchor_id.to_string();

    let task = tokio::spawn(async move {
        let _ = recorder::start_recording(
            &config_clone,
            &anchor_name,
            title.as_deref(),
            &ffmpeg_path_clone,
            child_token,
            &anchor_id_clone,
        )
        .await;
    });

    // === 第5步：注册到状态（持锁，短暂）===
    let mut guard = state.state.lock().await;
    guard.tasks.insert(
        anchor_id.to_string(),
        crate::state::AnchorTask {
            cancel_token,
            handle: task,
        },
    );
    Ok(())
}

/// 内部：停止录制（仅发取消信号 + 等待，不强制 abort）
async fn stop_anchor_recording_internal(
    state: &State<'_, RecorderState>,
    anchor_id: &str,
) -> Result<(), AppError> {
    let task = {
        let mut guard = state.state.lock().await;
        guard.tasks.remove(anchor_id)
    };
    if let Some(task) = task {
        // 发送取消信号，让 ffmpeg 自然退出
        task.cancel_token.cancel();
        // 等待最多 10 秒让任务自行结束
        tokio::time::timeout(std::time::Duration::from_secs(10), task.handle)
            .await
            .ok();
    }
    Ok(())
}

// -------- Tauri 命令 ----------

/// 获取主播列表（含直播状态）—— 使用 futures::future::join_all 并发请求
#[tauri::command]
pub async fn get_anchors(state: State<'_, RecorderState>) -> Result<Vec<AnchorInfo>, AppError> {
    let config = {
        let guard = state.state.lock().await;
        guard.config.clone()
    };

    // 使用 tokio::spawn 并行请求（生命周期与当前函数绑定，不会泄漏）
    let tasks: Vec<_> = config.anchors.iter().map(|cfg| {
        let url = cfg.url.clone();
        let proxy = cfg.proxy.clone();
        let cookie = cfg.cookie.clone();
        let id = cfg.id.clone();
        let name = cfg.name.clone();
        tokio::spawn(async move {
            let result = spider::get_live_info_raw(&url, proxy.as_deref(), cookie.as_deref()).await;
            (id, name, result)
        })
    }).collect();

    // 收集结果（单次锁获取录制状态）
    let recording_set: std::collections::HashSet<String> = {
        let guard = state.state.lock().await;
        guard.tasks.keys().cloned().collect()
    };

    let mut result = Vec::new();
    for task in tasks {
        match task.await {
            Ok((id, name, Ok(json))) => {
                let is_live = json["info"]["room"]["status"]["broadcasting"]
                    .as_bool()
                    .unwrap_or(false);
                let avatar = spider::extract_avatar_from_json(&json);
                let title = json["info"]["room"]["name"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let intro = json["info"]["creator"]["introduction"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let live_name = json["info"]["creator"]["username"]
                    .as_str()
                    .unwrap_or(&name)
                    .to_string();
                let anchor_cfg = config.get_anchor(&id);
                let is_rec = recording_set.contains(&id);
                result.push(AnchorInfo {
                    id,
                    name: live_name,
                    avatar,
                    is_live,
                    is_recording: is_rec,
                    room_url: anchor_cfg.map(|c| c.url.clone()).unwrap_or_default(),
                    title,
                    introduction: intro,
                    config: anchor_cfg.cloned().unwrap_or_else(|| AnchorConfig::new("未知", "")),
                });
            }
            Ok((id, name, Err(_))) => {
                let anchor_cfg = config.get_anchor(&id);
                let is_rec = recording_set.contains(&id);
                result.push(AnchorInfo {
                    id,
                    name,
                    avatar: None,
                    is_live: false,
                    is_recording: is_rec,
                    room_url: anchor_cfg.map(|c| c.url.clone()).unwrap_or_default(),
                    title: String::new(),
                    introduction: String::new(),
                    config: anchor_cfg.cloned().unwrap_or_else(|| AnchorConfig::new("未知", "")),
                });
            }
            Err(_) => {}
        }
    }
    Ok(result)
}

#[tauri::command]
pub async fn start_recording_anchor(
    state: State<'_, RecorderState>,
    anchor_id: String,
    app_handle: tauri::AppHandle,
) -> Result<(), AppError> {
    start_anchor_recording_internal(&state, &anchor_id, app_handle).await
}

#[tauri::command]
pub async fn stop_recording_anchor(
    state: State<'_, RecorderState>,
    anchor_id: String,
) -> Result<(), AppError> {
    stop_anchor_recording_internal(&state, &anchor_id).await
}

#[tauri::command]
pub async fn add_anchor(
    state: State<'_, RecorderState>,
    name: String,
    url: String,
) -> Result<AnchorConfig, AppError> {
    // 操作配置（持锁，不 I/O）
    let (id, updated_config) = {
        let mut guard = state.state.lock().await;
        let id = guard.config.add_anchor(&name, &url);
        (id, guard.config.clone())
    };
    // 保存（不持锁）
    crate::config::manager::save_global_config(&updated_config)
        .map_err(|e| AppError::Config(e.to_string()))?;
    Ok(updated_config
        .get_anchor(&id)
        .ok_or_else(|| AppError::AnchorNotFound(id))?
        .clone())
}

#[tauri::command]
pub async fn remove_anchor(
    state: State<'_, RecorderState>,
    anchor_id: String,
) -> Result<(), AppError> {
    stop_anchor_recording_internal(&state, &anchor_id).await?;
    let updated_config = {
        let mut guard = state.state.lock().await;
        guard.config.remove_anchor(&anchor_id);
        guard.config.clone()
    };
    crate::config::manager::save_global_config(&updated_config)
        .map_err(|e| AppError::Config(e.to_string()))?;
    // 删除独立配置文件
    crate::config::manager::AnchorStore::delete_anchor_file(&anchor_id);
    Ok(())
}

#[tauri::command]
pub async fn update_anchor_config(
    state: State<'_, RecorderState>,
    anchor_config: AnchorConfig,
) -> Result<(), AppError> {
    let updated_config = {
        let mut guard = state.state.lock().await;
        if let Some(existing) = guard.config.get_anchor_mut(&anchor_config.id) {
            *existing = anchor_config.clone();
            Ok(guard.config.clone())
        } else {
            Err(AppError::AnchorNotFound(anchor_config.id))
        }
    }?;
    crate::config::manager::save_global_config(&updated_config)
        .map_err(|e| AppError::Config(e.to_string()))?;
    Ok(())
}

#[tauri::command]
pub async fn get_global_config(state: State<'_, RecorderState>) -> Result<Config, AppError> {
    let guard = state.state.lock().await;
    Ok(guard.config.clone())
}

#[tauri::command]
pub async fn update_global_config(
    state: State<'_, RecorderState>,
    mut new_config: Config,
) -> Result<(), AppError> {
    if let Some(ref mut path) = new_config.ffmpeg_path {
        *path = normalize_path(path);
    }
    {
        let mut guard = state.state.lock().await;
        guard.config = new_config.clone();
    }
    crate::config::manager::save_global_config(&new_config)
        .map_err(|e| AppError::Config(e.to_string()))?;
    Ok(())
}

#[tauri::command]
pub async fn run_all_checks_backend(
    state: State<'_, RecorderState>,
    app_handle: tauri::AppHandle,
) -> Result<(), AppError> {
    let config = {
        let guard = state.state.lock().await;
        guard.config.clone()
    };
    checker::run_all_checks(app_handle, &config).await;
    Ok(())
}

// -------- 音频文件管理 ----------

#[tauri::command]
pub async fn get_audio_files(state: State<'_, RecorderState>) -> Result<Vec<AudioFileInfo>, AppError> {
    let output_dir = {
        let guard = state.state.lock().await;
        guard.config.output_dir.clone()
    };
    let output_path = PathBuf::from(&output_dir).join("猫耳FM直播");
    if !output_path.exists() {
        return Ok(vec![]);
    }
    let mut files = Vec::new();
    for entry in WalkDir::new(&output_path)
        .max_depth(3)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext == "m4a" || ext == "mp3" {
                let metadata = fs::metadata(path)?;
                let modified = metadata.modified()?;
                let modified_str = chrono::DateTime::<chrono::Local>::from(modified)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string();
                // 提取所属文件夹（相对于 output_path）
                let folder = path
                    .parent()
                    .and_then(|p| {
                        if p == output_path {
                            None
                        } else {
                            p.file_name().map(|f| f.to_string_lossy().to_string())
                        }
                    })
                    .unwrap_or_default();
                files.push(AudioFileInfo {
                    name: path.file_name().unwrap().to_string_lossy().to_string(),
                    path: path.to_string_lossy().to_string(),
                    size: metadata.len(),
                    modified: modified_str,
                    folder,
                });
            }
        }
    }
    Ok(files)
}

#[tauri::command]
pub fn rename_audio_file(old_path: String, new_name: String) -> Result<(), AppError> {
    let old = PathBuf::from(&old_path);
    let parent = old
        .parent()
        .ok_or_else(|| AppError::Io("无效路径".to_string()))?;
    let safe_name = new_name.replace(|c: char| c == '/' || c == '\\', "_");
    let new_path = parent.join(&safe_name);
    fs::rename(&old, &new_path)?;
    Ok(())
}

#[tauri::command]
pub fn delete_audio_file(path: String) -> Result<(), AppError> {
    fs::remove_file(&path)?;
    Ok(())
}

#[tauri::command]
pub async fn get_avatar_base64(avatar_url: String) -> Result<String, AppError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let response = client
        .get(&avatar_url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .header("Accept", "image/webp,image/apng,image/*,*/*;q=0.8")
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
        .header("Referer", "https://fm.missevan.com/")
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(AppError::Network(format!("HTTP 错误: {}", response.status())));
    }
    let bytes = response.bytes().await?;
    let mime = match avatar_url
        .split('.')
        .last()
        .unwrap_or("jpg")
        .to_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "image/jpeg",
    };
    let base64_str = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{};base64,{}", mime, base64_str))
}

#[tauri::command]
pub fn open_with_system(path: String) -> Result<(), AppError> {
    use std::process::Command;
    let pb = PathBuf::from(&path);
    if !pb.exists() {
        return Err(AppError::Io("文件不存在".to_string()));
    }
    #[cfg(target_os = "windows")]
    { Command::new("cmd").args(&["/c", "start", "", &pb.to_string_lossy()]).spawn()?; }
    #[cfg(target_os = "macos")]
    { Command::new("open").arg(&pb).spawn()?; }
    #[cfg(target_os = "linux")]
    { Command::new("xdg-open").arg(&pb).spawn()?; }
    Ok(())
}

#[tauri::command]
pub fn open_folder(path: String) -> Result<(), AppError> {
    use std::path::Path;
    use std::process::Command;
    // 解析相对路径为绝对路径
    let raw = Path::new(&path);
    let pb = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| AppError::Io(format!("获取当前目录失败: {}", e)))?
            .join(raw)
    };
    if !pb.exists() {
        return Err(AppError::Io(format!("文件夹不存在: {}", pb.display())));
    }
    #[cfg(target_os = "windows")]
    { Command::new("explorer").arg(&pb).spawn()?; }
    #[cfg(target_os = "macos")]
    { Command::new("open").arg(&pb).spawn()?; }
    #[cfg(target_os = "linux")]
    { Command::new("xdg-open").arg(&pb).spawn()?; }
    Ok(())
}
