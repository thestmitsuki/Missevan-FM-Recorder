use std::sync::Arc;
use tauri::{Manager, State};

use crate::domain::config::autostart::{apply_autostart, AutostartStore};
use crate::domain::config::manager::{ConfigManager, ImportSummary};
use crate::domain::config::model::GlobalConfig;
use crate::domain::services::cleanup::{run_cleanup, CleanupSummary};
use crate::domain::services::file_cache::FileCacheHandle;
use crate::infrastructure::error::types::AppError;
use crate::infrastructure::notification::dispatcher::NotificationDispatcher;
use crate::infrastructure::state::app_state::RecorderState;

#[tauri::command]
pub async fn get_config(
    config_manager: State<'_, Arc<ConfigManager>>, // 新增依赖
    dispatcher: State<'_, Arc<NotificationDispatcher>>,
) -> Result<GlobalConfig, AppError> {
    let path = config_manager.global_config_path();
    let exists = path.exists();

    match config_manager.load() {
        Ok(config) => {
            if !exists {
                // 首次运行或配置文件被删除，发送 Info 通知
                dispatcher
                    .info(
                        "config_not_found",
                        "配置初始化",
                        "未找到配置文件，已使用默认设置。请保存配置以生成文件。",
                    )
                    .await;
            }
            Ok(config.global)
        }
        Err(e) => {
            // 加载失败（如 TOML 解析错误），发送 Error 通知
            dispatcher
                .error(
                    "config_load_error",
                    "配置加载失败",
                    format!("无法读取配置文件：{}", e),
                )
                .await;
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn save_config(
    app: tauri::AppHandle,
    state: State<'_, RecorderState>,
    config: GlobalConfig,
    config_manager: tauri::State<'_, Arc<ConfigManager>>,
    dispatcher: State<'_, Arc<NotificationDispatcher>>,
) -> Result<(), AppError> {
    // 若 state 未被使用，可移除锁或使用 _state
    let _guard = state.state.lock().await;

    tracing::info!("保存配置");
    tracing::info!("📥 后端接收到的全局配置: {:?}", config);
    tracing::info!(
        "配置保存路径: {}",
        config_manager.global_config_path().display()
    );

    // 尝试保存，捕获错误
    if let Err(e) = config_manager.save_global(&config) {
        // 发送错误通知到所有渠道
        dispatcher
            .error(
                "config_save_failed",
                "配置保存失败",
                format!("保存配置文件时出错：{}", e),
            )
            .await;
        return Err(e);
    }

    // 保存成功，发送成功通知
    dispatcher
        .info("config_save_ok", "配置保存成功", "配置已保存至文件")
        .await;

    // 托盘图标开关即时生效（Task 17：show_tray 变化无需重启）
    crate::infrastructure::tray::reconcile_tray(&app, config.show_tray);

    // Task 20（Task 12 Important-1 跟进）：输出目录动态放行 asset protocol——
    // 内置播放器经 convertFileSrc 加载本地音频，tauri.conf.json 的 asset scope 默认
    // `$HOME/**`；用户把输出目录设在 $HOME 之外时播放会被拦截，保存配置时动态放行
    //（递归）。allow_directory 是运行时态（重启后 scope 恢复默认），应用启动时由
    // lib.rs setup 调用同一函数恢复放行（Task 20 Important-2）。
    if let Err(e) = allow_output_dir(&app, config_manager.inner()) {
        tracing::warn!("保存配置后放行输出目录失败: {}", e);
    }

    // 自动清理不再有定时调度（cleanup_scheduler 已删除）：录制结束时按最新
    // 配置即时触发（monitor.rs cleanup_on_recording_end），保存配置无需重建任何任务。
    Ok(())
}

/// 放行 asset protocol 对输出目录的访问（Task 20：save_config 与 lib.rs setup 共用）。
///
/// 内置播放器经 convertFileSrc 加载本地音频，tauri.conf.json 的 asset scope 默认
/// `$HOME/**`；用户把输出目录设在 $HOME 之外时播放会被拦截。`allow_directory` 是
/// 运行时态——重启后 scope 回默认，因此保存配置时与应用启动时（Important-2：
/// 本次启动未保存设置也要恢复放行）都要执行本函数（递归放行）。
/// 路径为空（自动探测默认）时跳过。
pub(crate) fn allow_output_dir(
    app: &tauri::AppHandle,
    config_manager: &ConfigManager,
) -> Result<(), AppError> {
    let config = config_manager.load()?;
    if !config.global.output_dir.trim().is_empty() {
        app.asset_protocol_scope()
            .allow_directory(std::path::Path::new(&config.global.output_dir), true)
            .map_err(|e| AppError::internal(format!("放行输出目录失败: {}", e)))?;
    }
    Ok(())
}

/// 导出配置为 JSON 字符串（§11.2）。
///
/// 格式：`{ "version": 1, "global": {...}, "anchors": [...] }`；
/// 敏感字段置空（`global.proxy_password` → ""、`anchor.cookie` → null）。
/// 返回给前端展示/保存（前端已有预览 UI；文件保存由前端选择路径后写入）。
#[tauri::command]
pub(crate) async fn export_config(
    config_manager: State<'_, Arc<ConfigManager>>,
) -> Result<String, AppError> {
    config_manager.export_json()
}

/// 导入配置（§11.2）。
///
/// `mode`：`replace`（global 全替换 + 文件含 anchors 时主播全替换）/ `merge`
/// （global 按字段合并 + 主播按 id 合并，重复 id 跳过保留本地）。
/// 接受包裹式（export_config 输出）与扁平式（GlobalConfig 单对象）两种 JSON。
/// 校验失败（非法 JSON / 字段类型错误 / 空主播 id）报错且不写入。
#[tauri::command]
pub(crate) async fn import_config(
    json: String,
    mode: String,
    config_manager: State<'_, Arc<ConfigManager>>,
    dispatcher: State<'_, Arc<NotificationDispatcher>>,
) -> Result<ImportSummary, AppError> {
    let summary = config_manager.import_json(&json, &mode)?;
    dispatcher
        .info(
            "config_import_ok",
            "配置导入成功",
            "配置已导入，部分更改将在重启后生效",
        )
        .await;
    tracing::info!(
        "配置导入完成: mode={} anchors_added={} anchors_skipped={} anchors_removed={}",
        summary.mode,
        summary.anchors_added,
        summary.anchors_skipped,
        summary.anchors_removed
    );
    Ok(summary)
}

/// 重置所有设置：删除配置目录（config.toml + anchors/）后重启应用（§11.2）。
///
/// 重启后首次运行向导会重新出现。`app.restart()` 不会返回——命令成功路径
/// 上 invoke Promise 不会 resolve，前端不应 await（或按连接断开处理）。
#[tauri::command]
pub(crate) async fn reset_config(
    app: tauri::AppHandle,
    config_manager: State<'_, Arc<ConfigManager>>,
) -> Result<(), AppError> {
    config_manager.delete_all()?;
    tracing::info!("配置已删除，应用即将重启进入首次运行向导");
    app.restart();
}

/// 设置开机自启（§11.2）。
///
/// 写 Windows 注册表 `HKCU\...\CurrentVersion\Run` 键 `MissevanRecorder`
/// （值 = `"{exe_path}" --minimized`），并同步更新 GlobalConfig.autostart。
/// 前端在 autostart 开关变化时应调用本命令（save_config 只落盘字段、不写注册表）。
#[tauri::command]
pub(crate) async fn set_autostart(
    enabled: bool,
    autostart_store: State<'_, Arc<dyn AutostartStore>>,
    config_manager: State<'_, Arc<ConfigManager>>,
) -> Result<(), AppError> {
    let exe = std::env::current_exe()
        .map_err(|e| AppError::internal(format!("获取可执行文件路径失败: {}", e)))?;
    apply_autostart(autostart_store.inner().as_ref(), enabled, &exe.to_string_lossy())?;
    // 同步 GlobalConfig.autostart（前端 save_config 也写该字段，这里保证一致）
    let mut config = config_manager.load()?;
    config.global.autostart = enabled;
    config_manager.save_global(&config.global)
}

/// 保存快捷键映射到 GlobalConfig.shortcuts（§11.2）。
///
/// `keys` 为空串表示解绑（删除该条目）。全局热键的实际绑定依赖
/// tauri-plugin-global-shortcut，本任务不实现（声明为未来项）。
#[tauri::command]
pub(crate) async fn set_shortcut(
    id: String,
    keys: String,
    config_manager: State<'_, Arc<ConfigManager>>,
) -> Result<(), AppError> {
    if id.trim().is_empty() {
        return Err(AppError::config("快捷键 id 不能为空"));
    }
    let mut config = config_manager.load()?;
    if keys.trim().is_empty() {
        config.global.shortcuts.remove(&id);
    } else {
        config.global.shortcuts.insert(id, keys);
    }
    config_manager.save_global(&config.global)
}

/// 立即执行一次录制文件清理（§11.2 run_cleanup_now）。
///
/// 按 `retention_days`（0 = 不按天数）删 N 天前的旧文件；若总量超
/// `max_total_gb`（0 = 不限制）按最旧优先删除直到达标或清空。
/// 清理完成后刷新文件缓存（emit `recording_files_changed`）。
/// 实现与录制结束自动触发共用 `domain::services::cleanup::run_cleanup`。
#[tauri::command]
pub(crate) async fn run_cleanup_now(
    window: tauri::WebviewWindow,
    cache: State<'_, FileCacheHandle>,
    config_manager: State<'_, Arc<ConfigManager>>,
    recorder_state: State<'_, RecorderState>,
) -> Result<CleanupSummary, AppError> {
    run_cleanup(
        window,
        cache.inner().clone(),
        config_manager.inner().clone(),
        recorder_state.state.clone(),
    )
    .await
}
