use std::sync::Arc;
use tauri::{Manager, State};

use crate::domain::config::manager::{redact_proxy_url, ConfigManager, ImportSummary};
use crate::domain::config::model::GlobalConfig;
use crate::domain::services::cleanup::{run_cleanup, CleanupSummary};
use crate::domain::services::file_cache::FileCacheHandle;
use crate::infrastructure::error::types::AppError;
use crate::infrastructure::i18n;
use crate::infrastructure::logging::setup::LogLevelReload;
use crate::infrastructure::notification::dispatcher::NotificationDispatcher;
use crate::infrastructure::state::app_state::RecorderState;
use crate::infrastructure::tray::TrayManager;
use crate::tr;

/// 同步前端语言到后端（前端 i18n 初始化/切换时调用；语言存前端 localStorage，
/// 后端无法直接读取，需显式同步）。此后后端通知/错误提示/日志按当前语言输出。
#[tauri::command]
pub fn set_locale(app: tauri::AppHandle, locale: String) {
    i18n::set_language(&locale);
    // 托盘菜单文本按语言渲染：语言切换后强制重建（数据未变时 apply 会跳过重建，
    // 菜单会保持旧语言直到录制数变化——M4 修复）
    if let Some(manager) = app.try_state::<Arc<TrayManager>>() {
        manager.refresh_menu_language();
    }
    // info 级别：切换语言时终端可见，便于确认后端语言已同步
    tracing::info!("{}", tr!("config.locale_synced", locale = locale));
}

/// 日志脱敏：`proxy_password` → `***`、`proxy_addr` 内嵌凭据 → `user:***@`
/// （避免明文密码进日志文件/调试页；与 debug_cmds::redact_config 同思路，
/// 此处作用于 GlobalConfig）
fn redact_global_config(config: &GlobalConfig) -> String {
    let mut v = serde_json::to_value(config).unwrap_or(serde_json::Value::Null);
    if let Some(global) = v.as_object_mut() {
        global.insert(
            "proxy_password".to_string(),
            serde_json::Value::String("***".to_string()),
        );
        if let Some(addr) = global.get_mut("proxy_addr") {
            if let Some(s) = addr.as_str() {
                *addr = serde_json::Value::String(redact_proxy_url(s));
            }
        }
    }
    serde_json::to_string(&v).unwrap_or_else(|_| "<serialize failed>".to_string())
}

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
                        tr!("config.not_found"),
                        tr!("config.not_found_body"),
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
                    tr!("config.load_failed"),
                    tr!("config.load_failed_body", err = e),
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
    log_reload: State<'_, LogLevelReload>,
) -> Result<(), AppError> {
    // 若 state 未被使用，可移除锁或使用 _state
    let _guard = state.state.lock().await;

    // U5 变更判断：保存前读取旧级别（load 带缓存，无额外磁盘 IO）——
    // 仅级别实际变化才热更新，避免每次保存都重建 callsite 缓存
    let prev_log_level = config_manager.load().ok().map(|c| c.global.log_level);

    tracing::info!("{}", tr!("config.saving"));
    tracing::info!(
        "{}",
        tr!("config.received_config", config = redact_global_config(&config))
    );
    tracing::info!(
        "{}",
        tr!(
            "config.save_path",
            path = config_manager.global_config_path().display()
        )
    );

    // 尝试保存，捕获错误
    if let Err(e) = config_manager.save_global(&config) {
        // 发送错误通知到所有渠道
        dispatcher
            .error(
                "config_save_failed",
                tr!("config.save_failed"),
                tr!("config.save_failed_body", err = e),
            )
            .await;
        return Err(e);
    }

    // 保存成功，发送成功通知
    dispatcher
        .info(
            "config_save_ok",
            tr!("config.save_ok"),
            tr!("config.save_ok_body"),
        )
        .await;

    // 托盘图标可见性即时生效（简化后由 close_behavior 派生：tray→显示 / exit→隐藏）
    crate::infrastructure::tray::reconcile_tray(
        &app,
        crate::infrastructure::tray::should_hide_to_tray(&config.close_behavior),
    );

    // Task 20（Task 12 Important-1 跟进）：输出目录动态放行 asset protocol——
    // 内置播放器经 convertFileSrc 加载本地音频，tauri.conf.json 的 asset scope 默认
    // `$HOME/**`；用户把输出目录设在 $HOME 之外时播放会被拦截，保存配置时动态放行
    //（递归）。allow_directory 是运行时态（重启后 scope 恢复默认），应用启动时由
    // lib.rs setup 调用同一函数恢复放行（Task 20 Important-2）。
    if let Err(e) = allow_output_dir(&app, config_manager.inner()) {
        tracing::warn!("{}", tr!("config.allow_dir_after_save_failed", err = e));
    }

    // U5：日志级别热更新——配置已成功落盘，运行中即时切换级别（白名单校验
    // 在 LogLevelReload::reload 内，非法值回退 info；失败静默降级不阻断保存）。
    // 仅在级别实际变化时触发（避免无谓的 callsite 缓存重建）。
    if prev_log_level.as_deref() != Some(config.log_level.as_str()) {
        log_reload.reload(&config.log_level);
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
            .map_err(|e| AppError::internal(tr!("config.allow_dir_failed", err = e)))?;
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
/// 校验失败（非法 JSON / 字段类型错误 / 字段值非法 / 空主播 id）报错且不写入
/// （S4b：写入前执行与正常加载相同的 Config::is_valid 全量校验）。
#[tauri::command]
pub(crate) async fn import_config(
    app: tauri::AppHandle,
    json: String,
    mode: String,
    config_manager: State<'_, Arc<ConfigManager>>,
    dispatcher: State<'_, Arc<NotificationDispatcher>>,
    log_reload: State<'_, LogLevelReload>,
) -> Result<ImportSummary, AppError> {
    // U5 变更判断：导入前读取旧级别（load 带缓存，无额外磁盘 IO）——
    // 仅级别实际变化才热更新，避免每次导入都重建 callsite 缓存
    let prev_log_level = config_manager.load().ok().map(|c| c.global.log_level);
    let summary = config_manager.import_json(&json, &mode)?;
    // S4b（M2 跟进）：导入成功后与 save_config 一致触发托盘 reconcile + 输出目录
    // asset scope 放行——导入的 close_behavior（派生托盘可见性）/ output_dir 立即生效
    let effective = config_manager.load()?;
    crate::infrastructure::tray::reconcile_tray(
        &app,
        crate::infrastructure::tray::should_hide_to_tray(&effective.global.close_behavior),
    );
    if let Err(e) = allow_output_dir(&app, config_manager.inner()) {
        tracing::warn!(
            "{}",
            tr!("config.allow_dir_after_import_failed", err = e)
        );
    }
    // U5：导入的 log_level 同样热更新即时生效（无需重启）；仅级别实际变化时触发
    if prev_log_level.as_deref() != Some(effective.global.log_level.as_str()) {
        log_reload.reload(&effective.global.log_level);
    }
    dispatcher
        .info(
            "config_import_ok",
            tr!("config.import_ok"),
            tr!("config.import_ok_body"),
        )
        .await;
    tracing::info!(
        "{}",
        tr!(
            "config.import_done",
            mode = summary.mode,
            anchors_added = summary.anchors_added,
            anchors_skipped = summary.anchors_skipped,
            anchors_removed = summary.anchors_removed
        )
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
    tracing::info!("{}", tr!("config.reset_done"));
    app.restart();
}

/// 设置开机自启（§11.2）。
///
/// 经 tauri-plugin-autostart（auto_launch crate）实现：Windows 写注册表
/// `HKCU\...\CurrentVersion\Run` 键、Linux 写 XDG autostart desktop 文件；
/// 值名/参数在 lib.rs 插件注册时配置（app_name="MissevanRecorder"、
/// arg="--minimized"，与旧手写实现一致），并同步更新 GlobalConfig.autostart。
/// 前端在 autostart 开关变化时应调用本命令（save_config 只落盘字段、不写系统）。
#[tauri::command]
pub(crate) async fn set_autostart(
    enabled: bool,
    app: tauri::AppHandle,
    config_manager: State<'_, Arc<ConfigManager>>,
) -> Result<(), AppError> {
    use tauri_plugin_autostart::ManagerExt;
    // tauri_plugin_autostart::ManagerExt::autolaunch(&self) -> State<'_, AutoLaunchManager>
    //（impl for T: Manager<R>，AppHandle 适用）；AutoLaunchManager::enable/disable
    // -> Result<(), tauri_plugin_autostart::Error>
    // auto_launch 的 enable/disable 幂等（内部先查 is_enabled），重复调用安全
    let auto = app.autolaunch();
    let result = if enabled { auto.enable() } else { auto.disable() };
    result.map_err(|e| {
        AppError::system(
            crate::infrastructure::error::types::IO_WRITE_FAIL,
            tr!("config.autostart_failed"),
        )
        .with_technical(e.to_string())
    })?;
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
        return Err(AppError::config(tr!("config.shortcut_id_empty")));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// H1：save_config 日志脱敏——proxy_password 不得出现在日志输出中。
    /// 用 serde_json 构造（GlobalConfig 实现 serde(default)），不依赖字段 Rust 类型。
    #[test]
    fn redact_global_config_blanks_proxy_password() {
        let config: GlobalConfig = serde_json::from_str(r#"{"proxy_password":"s3cret"}"#).unwrap();
        let s = redact_global_config(&config);
        assert!(!s.contains("s3cret"), "明文密码泄漏进日志: {s}");
        assert!(s.contains("***"), "脱敏占位符缺失: {s}");
    }

    /// M1：proxy_addr 内嵌凭据（http://user:pass@host）同样脱敏。
    #[test]
    fn redact_global_config_redacts_proxy_addr_credentials() {
        let config: GlobalConfig = serde_json::from_str(
            r#"{"proxy_password":"s3cret","proxy_addr":"http://user:pw@proxy.example.com:8080"}"#,
        )
        .unwrap();
        let s = redact_global_config(&config);
        assert!(!s.contains("user:pw@"), "内嵌凭据泄漏进日志: {s}");
        assert!(s.contains("user:***@"), "代理 URL 密码未脱敏: {s}");
    }
}
