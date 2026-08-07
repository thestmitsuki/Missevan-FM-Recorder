mod api;
mod domain;
mod infrastructure;

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use domain::detector::stats::DetectorStats;
use domain::recorder::engine::FfmpegRecorder;
use infrastructure::logging::network::global_store as network_log_store;
use infrastructure::logging::setup::init_logging;
use infrastructure::notification::dispatcher::NotificationDispatcher;
use infrastructure::state::app_state::{AvatarCache, RecorderState};
use infrastructure::state::mock_store::MockStore;
use infrastructure::tray::{self, TrayManager};

use crate::domain::config::autostart::AutostartStore;
#[cfg(windows)]
use crate::domain::config::autostart::WinregAutostart;
#[cfg(not(windows))]
use crate::domain::config::autostart::NoopAutostart;
use crate::domain::services::cleanup_scheduler::CleanupScheduler;
use crate::domain::services::file_cache::{FileCache, FileCacheHandle, FileCacheManager};
use domain::config::manager::ConfigManager;
use domain::config::model::{AnchorConfig, Config};
use domain::detector::r#loop::DetectionLoop;
use domain::spider::MissevanClient;

use tauri::Manager;

/// 注册进程级 panic hook（在 `run()` 早期、init_logging 之后调用）。
///
/// 实现方式：**先** `std::panic::take_hook()` 取回默认 hook 保存，**再**
/// `std::panic::set_hook` 安装包装 hook——包装 hook 先 `tracing::error!` 写入
/// `[panic] {msg}\n{location}`，最后链式调用默认 hook（panic 消息仍打印到
/// stderr / 测试 harness 捕获）。**不能**先 set_hook 覆盖默认 hook：那会吞掉
/// 默认输出（含 backtrace 提示）。`set_hook` 是进程级的——主线程与所有子线程
/// （含 tokio 工作线程 / tauri 异步运行时）的 panic 都经过同一 hook。
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = panic_payload_message(info.payload());
        let location = info
            .location()
            .map(|l| l.to_string())
            .unwrap_or_else(|| "<未知位置>".to_string());
        tracing::error!("[panic] {msg}\n{location}");
        default_hook(info);
    }));
}

/// panic 载荷消息提取：`&str` / `String` → 原文；其余类型（如 `panic_any`）→ 固定占位
/// （`dyn Any` 无 Debug 实现，不能直接格式化）。
fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<非字符串 panic 载荷>".to_string()
    }
}

/// 主播「启用检测与自动录制」当前值（从最新加载的配置判断）。
/// 录制启动前与延迟结束复检共用——延迟窗口内用户可能关闭检测，两处都必须
/// 读最新配置而非启动时的快照。
fn anchor_check_enabled(config: &Config, anchor_id: &str) -> bool {
    config
        .anchors
        .iter()
        .any(|a| a.id == anchor_id && a.enable_check)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::config::model::GlobalConfig;

    #[test]
    fn anchor_check_enabled_reflects_latest_config() {
        let config = Config {
            global: GlobalConfig::default(),
            anchors: vec![
                AnchorConfig {
                    id: "a1".into(),
                    name: "主播A".into(),
                    url: "https://fm.missevan.com/live/1".into(),
                    room_id: "1".into(),
                    proxy: None,
                    cookie: None,
                    enable_check: true,
                    avatar_url: None,
                    tags: Vec::new(),
                },
                AnchorConfig {
                    id: "a2".into(),
                    name: "主播B".into(),
                    url: "https://fm.missevan.com/live/2".into(),
                    room_id: "2".into(),
                    proxy: None,
                    cookie: None,
                    enable_check: false,
                    avatar_url: None,
                    tags: Vec::new(),
                },
            ],
        };
        assert!(anchor_check_enabled(&config, "a1"), "开启检测 → true");
        assert!(!anchor_check_enabled(&config, "a2"), "关闭检测 → false");
        // 主播已删除 / 不存在 → false（延迟结束复检据此放弃启动）
        assert!(!anchor_check_enabled(&config, "gone"));
    }

    #[test]
    fn panic_payload_message_extracts_str_and_string() {
        let payload: &(dyn std::any::Any + Send) = &"boom";
        assert_eq!(panic_payload_message(payload), "boom");
        let s = String::from("录制引擎失败");
        let payload: &(dyn std::any::Any + Send) = &s;
        assert_eq!(panic_payload_message(payload), "录制引擎失败");
        // 非字符串载荷（如 panic_any(42)）→ 固定占位，不 panic、不格式化
        let payload: &(dyn std::any::Any + Send) = &42i32;
        assert_eq!(panic_payload_message(payload), "<非字符串 panic 载荷>");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_data_dir = dirs::data_dir()
        .map(|p| p.join("missevan-recorder"))
        .unwrap_or_else(|| std::path::PathBuf::from("./data"));
    // 日志级别（高级分类 log_level 接线，「重启生效」语义）：init_logging 之前
    // 读取配置，本次启动的日志级别 = 上次保存的值。配置缺失/损坏/字段非法
    // 一律回退 "info"，不阻断启动。
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let config_dir = exe_dir.join("config");
    let log_level = ConfigManager::new(config_dir.clone())
        .load()
        .map(|c| c.global.log_level.clone())
        .unwrap_or_else(|_| String::from("info"));
    let (_log_guard, log_buffer, log_handle_slot) = init_logging(&app_data_dir, &log_level);
    // 注册进程级 panic hook：panic 先写 tracing（文件 + 缓冲 + 控制台），再链式调用
    // 默认 hook（stderr）——修复「线程 panic 被 JoinHandle 静默吞掉」问题
    //（曾导致录制启动 panic 后无日志、无录音文件——panic 只进 stderr 且被异步框架
    //  捕获，GUI 应用中主线程 stderr 又不可见）。须在 init_logging 之后注册，
    // 否则 hook 触发的 tracing 事件没有订阅者。
    install_panic_hook();
    // 网络请求插桩 store（spider 调用点与 get_network_logs 命令共用全局实例）
    let network_store = network_log_store();

    // ── 单实例锁（双录防御 #4：防应用双开）──
    // 双开时两个实例各自持有独立检测循环 / 任务表 / FFmpeg 进程表，会为同一
    // 主播同时启动两个录制进程（双录根因候选②）。Windows 命名互斥体：
    // 第二实例启动时检测到互斥体已存在，直接退出（日志说明，不弹窗）。
    // 实现取舍（自实现而非 tauri-plugin-single-instance）见
    // infrastructure::single_instance.rs 模块注释。
    let Some(_single_instance_guard) = infrastructure::single_instance::acquire(
        "missevan-recorder-single-instance",
    ) else {
        tracing::warn!("检测到应用已在运行（单实例互斥体被占用），本实例退出");
        return;
    };

    let notifier = Arc::new(NotificationDispatcher::new());
    // Task 18：ConfigManager 注入通知分发器（备份恢复通知 + 通知设置同步）。
    // cfg(not(test))：测试构建不链入 dispatcher 代码（本机 rust-lld + Windows 下
    // 测试可执行文件无法加载 0xC0000139，见 dispatcher.rs 测试注释）
    #[cfg(not(test))]
    let config_manager = Arc::new(ConfigManager::new(config_dir).with_notifier(notifier.clone()));
    #[cfg(test)]
    let config_manager = Arc::new(ConfigManager::new(config_dir));
    // 开机自启注册表读写（Windows：winreg；其他平台：空实现）
    #[cfg(windows)]
    let autostart_store: Arc<dyn AutostartStore> = Arc::new(WinregAutostart::default());
    #[cfg(not(windows))]
    let autostart_store: Arc<dyn AutostartStore> = Arc::new(NoopAutostart::default());
    let mock_store = Arc::new(MockStore::new());
    let recorder_state = RecorderState::new(mock_store.clone());
    let live_cache = Arc::new(Mutex::new(HashMap::<String, bool>::new()));
    let avatar_cache: AvatarCache = Arc::new(Mutex::new(HashMap::new()));
    let file_cache: FileCacheHandle = Arc::new(Mutex::new(FileCache::new()));
    let detection_wake = Arc::new(tokio::sync::Notify::new());
    // 共享录制引擎（双录防御 #2/#3 的前提）：进程表全局唯一——每个录制任务
    // 共用同一个 FfmpegRecorder，入口 is_recording 与 insert_process 锁内去重
    // 才跨启动调用生效（此前每次录制 new 一个实例，进程表形同虚设）
    let recorder_shared: Arc<FfmpegRecorder> = Arc::new(FfmpegRecorder::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(recorder_state.clone())
        .manage(config_manager.clone())
        .manage(autostart_store.clone()) // 开机自启注册表读写（set_autostart）
        .manage(notifier.clone())
        .manage(live_cache.clone())
        .manage(avatar_cache.clone()) // 注册头像缓存
        .manage(file_cache.clone())
        .manage(detection_wake.clone()) // 检测循环唤醒信号（finish_wizard 触发一次立即检测）
        .manage(log_buffer.clone()) // 调试日志环形缓冲（get_logs / clear_logs）
        .manage(network_store.clone()) // 网络请求插桩缓冲（get_network_logs / clear_network_logs）
        .manage(CleanupScheduler::new()) // 自动清理每日定时调度（save_config 重建）
        // ── 关闭行为（Task 17：规格 1.1 / 设计 §11.5）──
        // 决策见 infrastructure::tray::decide_close_action（配置矩阵 × 托盘实际可用性）：
        //   tray × show_tray=true 且托盘存在且可见 → prevent_close + hide（驻留托盘）
        //   其余组合 → 不拦截，窗口正常关闭后走统一优雅退出（等录制任务 ≤5s 后 app.exit）
        // （托盘创建失败 / 运行中禁用时一律退出，避免隐藏窗口后应用「人间蒸发」）
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // 仅主窗口参与关闭行为；向导窗口始终直接关闭
                if window.label() != "main" {
                    return;
                }
                let app = window.app_handle();
                let config = app.state::<Arc<ConfigManager>>().load().unwrap_or_default();
                let close_behavior = config.global.close_behavior.clone();
                let show_tray = config.global.show_tray;
                // 托盘实际可用性：try_state 为 None = 创建失败；
                // enabled() 与运行中 show_tray 切换（reconcile_tray → set_enabled）同步
                let tray_enabled = app.try_state::<Arc<TrayManager>>().map(|t| t.enabled());
                match tray::decide_close_action(&close_behavior, show_tray, tray_enabled) {
                    tray::CloseAction::HideToTray => {
                        api.prevent_close();
                        let _ = window.hide();
                        tracing::info!(
                            "关闭请求已拦截：最小化到系统托盘（close_behavior={}, show_tray={}）",
                            close_behavior,
                            show_tray
                        );
                    }
                    tray::CloseAction::Exit => {
                        // 不 prevent_close：窗口正常关闭，随后统一优雅退出
                        tracing::info!(
                            "关闭请求：执行优雅退出（close_behavior={}, show_tray={}, 托盘可用={}）",
                            close_behavior,
                            show_tray,
                            tray_enabled.is_some_and(|e| e)
                        );
                        tray::request_shutdown(app);
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            crate::api::anchor_cmds::get_anchors,
            crate::api::anchor_cmds::add_anchor,
            crate::api::anchor_cmds::get_anchor_profile,
            crate::api::anchor_cmds::remove_anchor,
            crate::api::anchor_cmds::refresh_anchor,
            crate::api::anchor_cmds::stop_anchors_recording,
            crate::api::anchor_cmds::get_recording_status,
            crate::api::config_cmds::get_config,
            crate::api::config_cmds::save_config,
            crate::api::config_cmds::export_config,
            crate::api::config_cmds::import_config,
            crate::api::config_cmds::reset_config,
            crate::api::config_cmds::set_autostart,
            crate::api::config_cmds::set_shortcut,
            crate::api::config_cmds::run_cleanup_now,
            crate::api::fs_utils::open_output_dir,
            crate::api::recording_cmds::start_recording,
            crate::api::recording_cmds::stop_recording,
            crate::api::debug_cmds::run_health_check,
            crate::api::debug_cmds::get_debug_info,
            crate::api::debug_cmds::get_logs,
            crate::api::debug_cmds::clear_logs,
            crate::api::debug_cmds::get_network_logs,
            crate::api::debug_cmds::clear_network_logs,
            crate::api::debug_cmds::get_detector_stats,
            crate::api::debug_cmds::trigger_detection_now,
            crate::api::debug_cmds::reset_detector_stats,
            crate::api::debug_cmds::get_recorder_state,
            crate::api::debug_cmds::get_file_cache_state,
            crate::api::debug_cmds::clear_file_cache,
            crate::api::debug_cmds::export_diagnostic_report,
            crate::api::debug_cmds::get_mock_state,
            crate::api::file_cmds::pick_output_dir,
            crate::api::mock_cmds::set_mock_live_data,
            crate::api::mock_cmds::set_mock_mode,
            crate::api::mock_cmds::list_mock_anchors,
            crate::api::mock_cmds::add_mock_anchor,
            crate::api::mock_cmds::update_mock_anchor,
            crate::api::mock_cmds::remove_mock_anchor,
            crate::api::mock_cmds::set_all_mock_live,
            crate::api::mock_cmds::reset_mock,
            crate::api::anchor_cmds::update_anchor,
            crate::api::file_cmds::get_recording_files,
            crate::api::file_cmds::refresh_recording_files,
            crate::api::file_cmds::rename_recording_file,
            crate::api::file_cmds::delete_recording_file,
            crate::api::file_cmds::play_recording_file,
            crate::api::wizard_cmds::download_ffmpeg,
            crate::api::wizard_cmds::exit_app,
            crate::api::wizard_cmds::finish_wizard,
            crate::api::wizard_cmds::run_wizard_health_check,
            crate::api::update_cmds::check_update,
            crate::api::update_cmds::get_app_info,
            crate::api::update_cmds::open_browser,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();

            // 注入 AppHandle 到调试日志层（此后日志事件才 emit `debug:log`）
            *log_handle_slot.lock().unwrap() = Some(handle.clone());

            let window = handle.get_webview_window("main").expect("未找到主窗口");
            let window_for_recording = window.clone();

            {
                let state: tauri::State<'_, RecorderState> = app.state();
                state.set_app_handle(handle.clone());
            }
            {
                let handle_for_notifier = handle.clone();
                let notifier: tauri::State<'_, Arc<NotificationDispatcher>> = app.state();
                tauri::async_runtime::block_on(async move {
                    notifier.set_app_handle(handle_for_notifier).await;
                });
            }

            // ── 启动配置与参数（Task 17）──
            // show_tray：决定托盘图标是否可见（可见性可运行中经 reconcile_tray 切换）
            // --minimized（Task 14 自启参数）：仅当托盘创建成功时生效，
            // 否则回退为显示主窗口——托盘失败 + 窗口不可见 = 应用「人间蒸发」
            let startup_config = app.state::<Arc<ConfigManager>>().load().unwrap_or_default();
            // 自动清理定时调度（文件分类接线）：启动时按配置重建每日任务；
            // 配置变更经 save_config 的 reschedule 重建（auto_cleanup_enabled /
            // cleanup_time 变化即时生效）。
            app.state::<CleanupScheduler>().reschedule(handle.clone());
            // Task 20（Important-2）：启动时恢复输出目录的 asset protocol 放行。
            // allow_directory 是运行时态，重启后 scope 回 tauri.conf.json 默认
            // `$HOME/**`——输出目录在 $HOME 外且本次启动未保存设置时，内置播放器
            // 会被拦截。与 save_config 共用 allow_output_dir，配置加载成功后同步执行。
            {
                let config_manager: tauri::State<'_, Arc<ConfigManager>> = app.state();
                if let Err(e) =
                    crate::api::config_cmds::allow_output_dir(&handle, config_manager.inner())
                {
                    tracing::warn!("启动时放行输出目录失败: {}", e);
                }
            }
            // Task 18：启动时同步通知设置（系统通知开关/事件勾选立即生效）
            {
                let notifier: tauri::State<'_, Arc<NotificationDispatcher>> = app.state();
                notifier.sync_from_config(&startup_config.global);
            }
            // 组 C/3：Windows 通知 AUMID 注册（toast 应用身份，代替 PowerShell 兜底）。
            // 开发模式：在用户“开始菜单”创建带 AppUserModelID 的快捷方式（指向当前
            // 可执行文件），toast 以本应用身份显示；已安装（安装器已注册）时跳过。
            #[cfg(windows)]
            {
                use crate::infrastructure::notification::windows_toast;
                match windows_toast::ensure_aumid_registered() {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::debug!("通知 AUMID 已注册（安装器），跳过注册");
                    }
                    Err(e) => {
                        tracing::warn!("通知 AUMID 注册失败，Windows toast 可能不可用: {}", e);
                    }
                }
            }
            let show_tray = startup_config.global.show_tray;
            let start_minimized = std::env::args().any(|a| a == "--minimized") && show_tray;

            // ── 双窗口首次运行逻辑 ──
            // 首次运行（无配置文件）：只显示设置向导窗口（wizard），隐藏主窗口（main）
            // 非首次运行：直接关闭向导窗口；主窗口可见性在托盘创建成功后决定
            //（见下方「主窗口可见性」段——Task 17 修复：实现顺序调整）
            let is_first_run = app.state::<Arc<ConfigManager>>().is_first_run();
            if is_first_run {
                tracing::info!("首次运行：显示设置向导窗口（wizard），隐藏主窗口（main）");
                if let Some(wizard) = app.get_webview_window("wizard") {
                    let _ = wizard.show();
                }
                if let Some(main_win) = app.get_webview_window("main") {
                    let _ = main_win.hide();
                }
            } else {
                tracing::info!("非首次运行：关闭设置向导窗口（wizard）");
                if let Some(wizard) = app.get_webview_window("wizard") {
                    // 与 finish_wizard 同理：wizard 前端注册了 onCloseRequested 且 prevent_default()，
                    // close() 会被无条件取消；setup 期 JS 尚未挂载也会产生竞态。destroy() 直接销毁，
                    // 不触发 CloseRequested 事件
                    let _ = wizard.destroy();
                }
            }

            let config_manager_arc = (*app.state::<Arc<ConfigManager>>()).clone();
            let notifier_arc = (*app.state::<Arc<NotificationDispatcher>>()).clone();
            let live_cache_arc = (*app.state::<Arc<Mutex<HashMap<String, bool>>>>()).clone();
            let avatar_cache_arc = (*app.state::<AvatarCache>()).clone();
            let recorder_state = app.state::<RecorderState>();
            let app_state_arc = recorder_state.state.clone();

            // ── 上次退出干净度检查（托盘幽灵图标缓解，Windows）──
            // Windows shell 对死进程的托盘图标不主动清理（悬停通知区才移除）——
            // 应用被强杀/崩溃后重启，旧幽灵图标与新图标并存（「偶尔出现多个
            // 图标」候选根因之一，见 infrastructure::tray 模块注释）。检查
            // `{exe_dir}/.clean_exit` 标记：存在 = 上次退出不干净 → 提示用户
            // 「悬停通知区可清除残留图标」。标记随后常驻，直到统一优雅退出
            // （tray::request_shutdown → do_shutdown）时移除。
            #[cfg(windows)]
            {
                let unclean_exit = tray::clean_exit_marker_exists();
                tray::write_clean_exit_marker();
                if unclean_exit {
                    let notifier = notifier_arc.clone();
                    tauri::async_runtime::spawn(async move {
                        // 延迟 1s：setup 期前端可能尚未挂载 app:notification
                        // 监听，立即 emit 会被吞掉（通知文本双语文案，见报告）
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        notifier
                            .warning(
                                "TRAY_GHOST_HINT",
                                "检测到上次异常退出 (Abnormal exit detected)",
                                "若托盘中残留旧图标，可将鼠标悬停通知区清除。If an old tray icon remains, hover over the notification area to clear it.",
                            )
                            .await;
                    });
                }
            }

            // ── 系统托盘（Task 17：规格 1.1 / 设计 §11.5）──
            // 图标 + 右键菜单（显示主窗口 / 录制中：N / 最近录制 5 条 / 退出应用）；
            // show_tray=false 时创建但隐藏（运行中可经设置页 reconcile_tray 切换可见性）
            let tray_ok = match TrayManager::new(&handle, show_tray) {
                Ok(manager) => {
                    manager.spawn_refresher(app_state_arc.clone());
                    app.manage(manager);
                    true
                }
                Err(e) => {
                    tracing::error!("创建系统托盘失败: {}", e);
                    false
                }
            };

            // ── 主窗口可见性（Task 17 修复：必须在托盘创建**成功之后**决定）──
            // --minimized 仅当托盘创建成功时生效（驻留托盘，主窗保持隐藏）；
            // 托盘创建失败时回退为显示主窗——否则无托盘 + 无窗口，应用无法恢复
            if !is_first_run {
                if let Some(main_win) = app.get_webview_window("main") {
                    if start_minimized {
                        if tray_ok {
                            tracing::info!("--minimized：主窗口保持隐藏（驻留托盘）");
                        } else {
                            tracing::warn!("--minimized 已忽略：托盘创建失败，回退为显示主窗口");
                            let _ = main_win.show();
                        }
                    } else {
                        let _ = main_win.show();
                    }
                }
            }

            // 创建 MissevanClient （DetectionLoop 唯一使用；网络分类接线：
            // 全局代理 + api_timeout_secs 从启动配置读取）
            let client_for_detector = match MissevanClient::from_config(&startup_config.global) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("创建 MissevanClient 失败: {}", e);
                    return Ok(());
                }
            };

            let detector_stats = Arc::new(DetectorStats::new());
            let detection_loop = Arc::new(DetectionLoop::new(
                client_for_detector,
                window, // 原始 window 被移走
                live_cache_arc.clone(),
                app_state_arc.clone(),
                avatar_cache_arc.clone(),
                mock_store,
                detection_wake.clone(),                     // 手动唤醒信号（finish_wizard 触发一次立即检测）
                recorder_state.shutdown_notify.clone(),     // 退出信号（Task 17：优雅退出时停止循环）
                detector_stats, // 检测统计（get_detector_stats / trigger_detection_now / reset_detector_stats）
            ));
            // 托管检测循环：调试命令（get_detector_stats / trigger_detection_now / reset_detector_stats）
            app.manage(detection_loop.clone());

            let config_manager_for_get_config = config_manager_arc.clone();
            let config_manager_for_recording = config_manager_arc.clone();
            let notifier_for_recording = notifier_arc.clone();
            let app_state_for_recording = app_state_arc.clone();

            //文件缓存
            let file_cache_for_init = (*app.state::<FileCacheHandle>()).clone();
            let config_for_cache = config_manager_arc.clone();
            let window_for_init = window_for_recording.clone(); // 用 WebviewWindow
            let app_state_for_cache = app_state_arc.clone(); // 活跃录制任务（is_active 标记）
            tauri::async_runtime::spawn(async move {
                let manager = FileCacheManager::new(window_for_init, file_cache_for_init);
                if let Err(e) = manager.refresh(&config_for_cache, &app_state_for_cache).await {
                    tracing::error!("初始文件缓存刷新失败: {}", e);
                }
            });

            let file_cache = (*app.state::<FileCacheHandle>()).clone();
            let config_manager_for_rec = config_manager_arc.clone(); // Arc<ConfigManager>
            let start_recording = Arc::new(
                move |anchor: AnchorConfig, stream_url: String, cancel: CancellationToken| {
                    if stream_url.is_empty() {
                        tracing::warn!("流地址为空，放弃录制: {}", anchor.name);
                        return;
                    }
                    // 退出保护（Task 17）：优雅退出已开始（global_cancel 已 cancel）则不再启动新录制。
                    // 注意：此处**不能**用 tokio Mutex::blocking_lock()——本闭包在 DetectionLoop 的
                    // tokio 异步任务内同步调用，blocking_lock = block_on + try_enter_blocking_region()
                    // 会直接 panic（"Cannot block the current thread from within a runtime"），
                    // 静默吞掉录制启动（回归：检测到开播但从未产生录音文件/FFmpeg 进程）。
                    // 退出保护改由下方 spawn 的异步任务内 `app_state.lock().await` 双保险完成。
                    let notifier = notifier_for_recording.clone();
                    let app_state = app_state_for_recording.clone();
                    let config_manager = config_manager_for_recording.clone();
                    let window = window_for_recording.clone(); // 每次调用时克隆
                    let file_cache = file_cache.clone();
                    let config_manager_for_rec = config_manager_for_rec.clone();
                    // 共享录制引擎（双录防御 #2/#3：进程表全局唯一，跨启动调用生效）
                    let recorder = recorder_shared.clone();

                    tauri::async_runtime::spawn(async move {
                        // 双保险（Task 17）：spawn 到执行的间隙再查一次，关闭竞态窗口
                        if app_state.lock().await.global_cancel.is_cancelled() {
                            tracing::info!("应用正在退出，取消自动录制启动: {}", anchor.name);
                            return;
                        }
                        let config_full = match config_manager.load() {
                            Ok(c) => c,
                            Err(e) => {
                                tracing::error!("加载全局配置失败: {}", e);
                                return;
                            }
                        };
                        // 录制启动前验证最新配置（主 AGENT 双录竞态分析）：
                        // 检测循环可能在旧配置轮触发本任务（spawn 到注册的窗口内用户
                        // 保存 enable_check=false，update_anchor 的"保存即停"看不到未
                        // 注册的任务）——此处读磁盘最新配置，已关闭检测则直接放弃，
                        // 杜绝"保存后仍启动录制、延迟才被 monitor 兜底停止"。
                        if !anchor_check_enabled(&config_full, &anchor.id) {
                            tracing::info!(
                                "[录制] 主播 {} 已关闭自动检测，取消录制启动",
                                anchor.name
                            );
                            return;
                        }
                        // 录制前延迟（pre_record_delay_secs）：检测到开播后延迟 N 秒
                        // 再启动录制（等流稳定）。延迟窗口可取消——期间用户停止
                        // 录制/关闭检测/应用退出则放弃本次录制启动。
                        let config = if config_full.global.pre_record_delay_secs > 0 {
                            tracing::info!(
                                "[录制] {} 秒后开始录制: {}",
                                config_full.global.pre_record_delay_secs,
                                anchor.name
                            );
                            // 延迟窗口可取消（实装审查回归修复）：任务尚未注册进
                            // tasks 表，stop_recording/remove_anchor 找不到任务会
                            // 返回"未在录制中"——先把本次启动注册进
                            // AppState.pending_starts（含取消令牌），停止命令据此
                            // 取消；延迟结束复检通过后才真正启动 ffmpeg。
                            if !app_state
                                .lock()
                                .await
                                .register_pending_start(&anchor.id, cancel.clone())
                            {
                                tracing::info!(
                                    "[录制] 已有延迟中的录制启动，跳过重复触发: {}",
                                    anchor.name
                                );
                                return;
                            }
                            let global_cancel = app_state.lock().await.global_cancel.clone();
                            tokio::select! {
                                _ = tokio::time::sleep(std::time::Duration::from_secs(
                                    config_full.global.pre_record_delay_secs as u64,
                                )) => {}
                                // 用户停止（stop_recording/remove_anchor 取消
                                // pending_starts 中的令牌）
                                _ = cancel.cancelled() => {}
                                // 应用退出（global_cancel 由优雅退出路径取消）
                                _ = global_cancel.cancelled() => {}
                            }
                            // 清理 pending 注册（幂等：若停止命令已取消并移除，
                            // 这里不再动作）。先查令牌再放行——停止命令可能在
                            // sleep 完成与清理之间的窗口内触发。
                            app_state.lock().await.remove_pending_start(&anchor.id);
                            if app_state.lock().await.global_cancel.is_cancelled() {
                                tracing::info!(
                                    "[录制] 应用正在退出（延迟期间），取消录制启动: {}",
                                    anchor.name
                                );
                                return;
                            }
                            if cancel.is_cancelled() {
                                tracing::info!(
                                    "[录制] 录制已取消（延迟期间），放弃录制: {}",
                                    anchor.name
                                );
                                return;
                            }
                            // 延迟结束复检（实装审查回归修复）：延迟期间用户可能
                            // 关闭检测/退出应用——重新读磁盘最新配置，检测已关则
                            // 放弃启动（正是上方既有注释要杜绝的场景：延迟结束后
                            // 仍启动、只靠 monitor 兜底 ≤10s 才停）。
                            let config_recheck = match config_manager.load() {
                                Ok(c) => c,
                                Err(e) => {
                                    tracing::error!(
                                        "加载全局配置失败（延迟结束复检）: {}",
                                        e
                                    );
                                    return;
                                }
                            };
                            if !anchor_check_enabled(&config_recheck, &anchor.id) {
                                tracing::info!(
                                    "[录制] 主播 {} 已关闭自动检测（延迟结束复检），取消录制启动",
                                    anchor.name
                                );
                                return;
                            }
                            config_recheck.global
                        } else {
                            config_full.global
                        };
                        // 录制 monitor 客户端：与检测循环同一代理/超时配置
                        //（网络分类接线：proxy_* / api_timeout_secs 全局生效）
                        let client = match MissevanClient::from_config(&config) {
                            Ok(c) => c,
                            Err(e) => {
                                tracing::error!("创建 MissevanClient 失败: {}", e);
                                return;
                            }
                        };
                        if let Err(e) = crate::domain::recorder::engine::start_ffmpeg_recording(
                            anchor,
                            stream_url,
                            cancel,
                            config,
                            recorder,
                            client,
                            notifier,
                            app_state,
                            window,
                            file_cache,
                            config_manager_for_rec,
                        )
                        .await
                        {
                            tracing::error!("自动录制启动失败: {}", e);
                        }
                    });
                },
            );

            // 启动统一的检测循环
            tauri::async_runtime::spawn(async move {
                let get_config = move || match config_manager_for_get_config.load() {
                    Ok(config) => config,
                    Err(e) => {
                        tracing::error!("加载配置失败: {}", e);
                        Config::default()
                    }
                };
                detection_loop.start(get_config, start_recording).await;
            });

            tracing::info!("应用启动完成");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
