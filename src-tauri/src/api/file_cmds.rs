use tauri::AppHandle;
use tauri::State;
use tauri_plugin_dialog::DialogExt;

use crate::domain::config::manager::ConfigManager;
use crate::domain::services::file_cache::{
    build_folder_tree, mark_active, FileCacheHandle, FileCacheManager, RecordingFile,
};
use crate::infrastructure::error::types::AppError;
use crate::infrastructure::state::app_state::RecorderState;
use crate::tr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[tauri::command]
pub async fn get_recording_files(
    search: Option<String>,
    cache: State<'_, FileCacheHandle>,
    recorder_state: State<'_, RecorderState>,
) -> Result<serde_json::Value, AppError> {
    // 活跃录制输出路径集合（先取路径集再锁缓存，避免与 refresh 侧锁顺序反转死锁）
    let active_paths = recorder_state.state.lock().await.active_output_paths();

    let cache = cache.lock().await;

    // 根据 search 过滤文件（文件名 + 主播名，不区分大小写）
    let filter_fn = |file: &&RecordingFile| -> bool {
        if let Some(ref q) = search {
            file.name.contains(q.as_str()) || file.anchor_name.contains(q.as_str())
        } else {
            true
        }
    };

    let mut files: Vec<RecordingFile> = cache
        .files
        .iter()
        .filter(|f| filter_fn(f))
        .cloned()
        .collect();

    // 以当前活跃任务为准重算 is_active（缓存可能是录制开始前的旧扫描）
    mark_active(&mut files, &active_paths);

    // 聚合为文件夹树：录制输出目录 → 主播文件夹 → 音频文件（空文件夹剔除）
    let folders: Vec<serde_json::Value> = build_folder_tree(&files)
        .into_iter()
        .filter(|f| !f.files.is_empty())
        .map(|f| serde_json::to_value(f).unwrap_or(serde_json::Value::Null))
        .collect();

    Ok(serde_json::json!({ "folders": folders }))
}

#[tauri::command]
pub async fn refresh_recording_files(
    window: tauri::WebviewWindow,
    cache: State<'_, FileCacheHandle>,
    config_manager: State<'_, Arc<ConfigManager>>,
    recorder_state: State<'_, RecorderState>,
) -> Result<(), AppError> {
    let manager = FileCacheManager::new(window, cache.inner().clone());
    manager.refresh(&config_manager, &recorder_state.state).await
}

#[tauri::command]
pub async fn rename_recording_file(
    old_path: String,
    new_name: String,
    window: tauri::WebviewWindow,
    cache: State<'_, FileCacheHandle>,
    config_manager: State<'_, Arc<ConfigManager>>,
    recorder_state: State<'_, RecorderState>,
) -> Result<(), AppError> {
    // 录制中文件禁止重命名（FFmpeg 正在写入：Windows 共享冲突 + 数据损坏风险）
    // 含分段段文件（is_active_path 前缀匹配 `{前缀}_NNN.{ext}`）
    {
        let active = recorder_state.state.lock().await.active_output_paths();
        if crate::domain::services::file_cache::is_active_path(&old_path, &active) {
            return Err(AppError::config(tr!("app.file_rename_active")));
        }
    }
    // H4：路径必须位于输出目录内（canonicalize 前缀匹配）；new_name 服务端消毒
    let config = config_manager.load()?;
    let old = ensure_within_output_dir(Path::new(&old_path), &config.global.output_dir)?;
    let ext = old
        .extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| AppError::config(tr!("app.file_no_extension")))?;
    let parent = old
        .parent()
        .ok_or_else(|| AppError::config(tr!("app.file_parent_missing")))?;
    validate_new_name(&new_name)?;
    let new_path = parent.join(format!("{}.{}", new_name.trim(), ext));
    // 目标重名检查（风险 1 修复）：std::fs::rename 在 Windows 上经
    // MoveFileExW + MOVEFILE_REPLACE_EXISTING 会**静默覆盖**已存在目标——
    // 旧录音将永久丢失（与 ffmpeg `-y` 覆盖同类风险）。改名目标已存在时
    // 明确拒绝，由用户换名，绝不覆盖。
    if new_path.exists() {
        return Err(AppError::config(tr!(
            "app.file_duplicate_name",
            path = new_path.display()
        )));
    }
    std::fs::rename(&old, &new_path)?;

    // 重命名后刷新缓存
    let manager = FileCacheManager::new(window, cache.inner().clone());
    manager.refresh(&config_manager, &recorder_state.state).await
}

#[tauri::command]
pub async fn delete_recording_file(
    path: String,
    window: tauri::WebviewWindow,
    cache: State<'_, FileCacheHandle>,
    config_manager: State<'_, Arc<ConfigManager>>,
    recorder_state: State<'_, RecorderState>,
) -> Result<(), AppError> {
    // 录制中文件禁止删除（FFmpeg 正在写入，删除会导致录制损坏/数据丢失）
    // 含分段段文件（is_active_path 前缀匹配 `{前缀}_NNN.{ext}`）
    {
        let active = recorder_state.state.lock().await.active_output_paths();
        if crate::domain::services::file_cache::is_active_path(&path, &active) {
            return Err(AppError::config(tr!("app.file_delete_active")));
        }
    }
    // H4：路径必须位于输出目录内（canonicalize 前缀匹配）——杜绝任意文件删除
    let config = config_manager.load()?;
    let canonical = ensure_within_output_dir(Path::new(&path), &config.global.output_dir)?;
    std::fs::remove_file(&canonical)?;

    // 删除后刷新缓存
    let manager = FileCacheManager::new(window, cache.inner().clone());
    manager.refresh(&config_manager, &recorder_state.state).await
}

#[tauri::command]
pub async fn play_recording_file(path: String) -> Result<String, AppError> {
    let url = tauri::Url::from_file_path(&path)
        .map_err(|_| AppError::internal(tr!("app.path_convert_failed")))?;
    Ok(url.to_string())
}

/// H4：校验路径位于输出目录内。
///
/// 两侧都 canonicalize 后做前缀匹配（跟随符号链接/junction 的真实路径判定，
/// 目录外的链接目标同样被拒绝）。文件/目录不存在时返回错误。
fn ensure_within_output_dir(path: &Path, output_dir: &str) -> Result<PathBuf, AppError> {
    let canonical = path
        .canonicalize()
        .map_err(|_| AppError::config(tr!("app.file_not_found")))?;
    let base = Path::new(output_dir)
        .canonicalize()
        .map_err(|_| AppError::config(tr!("app.output_dir_not_found")))?;
    if canonical.starts_with(&base) {
        Ok(canonical)
    } else {
        Err(AppError::config(tr!("app.path_outside_output_dir")))
    }
}

/// 重命名目标名校验（H4/L1）：非空；不含路径分隔符、`..`、Windows 非法字符
/// 与控制字符（服务端强制——前端 INVALID_NAME_CHARS 仅客户端校验可被绕过）
fn validate_new_name(name: &str) -> Result<(), AppError> {
    if name.trim().is_empty() {
        return Err(AppError::config(tr!("app.file_name_empty")));
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(AppError::config(tr!("app.file_name_invalid_path")));
    }
    if name
        .chars()
        .any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return Err(AppError::config(tr!("app.file_name_invalid")));
    }
    Ok(())
}

/// 打开系统目录选择对话框，返回用户选择的目录路径
///
/// 如果用户取消选择，返回 None
#[tauri::command]
pub async fn pick_output_dir(app_handle: AppHandle) -> Result<Option<String>, AppError> {
    let (tx, rx) = tokio::sync::oneshot::channel();

    app_handle.dialog().file().pick_folder(move |folder_path| {
        let selected = folder_path.map(|p| p.to_string());
        let _ = tx.send(selected);
    });

    rx.await
        .map_err(|e| AppError::internal(tr!("app.pick_folder_failed", err = e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_new_name_accepts_normal_names() {
        assert!(validate_new_name("我的录音").is_ok());
        assert!(validate_new_name("my file (1)").is_ok());
        assert!(validate_new_name("a-b_c.d").is_ok());
    }

    #[test]
    fn validate_new_name_rejects_traversal_and_invalid_chars() {
        assert!(validate_new_name("").is_err());
        assert!(validate_new_name("   ").is_err());
        assert!(validate_new_name("../evil").is_err());
        assert!(validate_new_name("..\\evil").is_err());
        assert!(validate_new_name("a/b").is_err());
        assert!(validate_new_name("a\\b").is_err());
        assert!(validate_new_name("a<b").is_err());
        assert!(validate_new_name("a:b").is_err());
        assert!(validate_new_name("a\"b").is_err());
        assert!(validate_new_name("a|b").is_err());
        assert!(validate_new_name("a?b").is_err());
        assert!(validate_new_name("a*b").is_err());
        assert!(validate_new_name("a\x00b").is_err());
    }

    fn unique_dir(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "missevan-test-fcmds-{}-{}-{}",
            tag,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ))
    }

    #[test]
    fn ensure_within_output_dir_accepts_inside_rejects_outside() {
        let root = unique_dir("within");
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let inside = sub.join("a.m4a");
        std::fs::write(&inside, b"x").unwrap();

        let outside = std::env::temp_dir().join(format!(
            "missevan-test-fcmds-outside-{}.txt",
            std::process::id()
        ));
        std::fs::write(&outside, b"x").unwrap();

        // 输出目录内的文件 → 通过（返回 canonical 路径）
        let ok = ensure_within_output_dir(&inside, root.to_str().unwrap()).unwrap();
        assert_eq!(ok, inside.canonicalize().unwrap());
        // 目录外文件 → 拒绝
        assert!(ensure_within_output_dir(&outside, root.to_str().unwrap()).is_err());
        // 相对路径形式同样归一化判定
        assert!(ensure_within_output_dir(&sub.join("a.m4a"), root.to_str().unwrap()).is_ok());
        // 不存在的文件 → 拒绝
        assert!(ensure_within_output_dir(&sub.join("nope.m4a"), root.to_str().unwrap()).is_err());

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn ensure_within_output_dir_rejects_sibling_prefix_spoof() {
        // 前缀匹配陷阱：/out2 不是 /out 的子路径（starts_with 按组件判定）
        let root = unique_dir("spoof");
        let sibling = root.join("out2");
        std::fs::create_dir_all(&sibling).unwrap();
        let f = sibling.join("x.m4a");
        std::fs::write(&f, b"x").unwrap();
        let out_dir = root.join("out").to_string_lossy().into_owned();
        assert!(
            ensure_within_output_dir(&f, &out_dir).is_err(),
            "out2 不得视为 out 的子路径"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
