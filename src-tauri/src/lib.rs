pub mod checker;
pub mod commands;
pub mod config;
pub mod error;
pub mod recorder;
pub mod spider;
pub mod state;
pub mod utils;

use crate::state::RecorderState;
use std::fs::OpenOptions;
use std::io::Write;
use std::panic;
use tauri::Manager;
use tokio::time::{sleep, Duration};

fn log_to_file(msg: &str) {
    let _ = OpenOptions::new()
        .create(true)
        .append(true)
        .open("debug.log")
        .and_then(|mut f| f.write_all(format!("[{}] {}\n", chrono::Local::now(), msg).as_bytes()));
}

/// 为单个主播启动自动检测录制循环
async fn run_auto_recording_for_anchor(
    _app_handle: tauri::AppHandle,
    state_handle: crate::state::AppStateHandle,
    anchor_id: String,
) {
    loop {
        let (config, anchor_cfg) = {
            let guard = state_handle.lock().await;
            let anchor = guard.config.get_anchor(&anchor_id).cloned();
            if anchor.is_none() {
                log_to_file(&format!("主播 {} 已被删除，停止检测", anchor_id));
                return;
            }
            (guard.config.clone(), anchor.unwrap())
        };

        if !anchor_cfg.enable_check {
            sleep(Duration::from_secs(60)).await;
            continue;
        }

        let is_recording = {
            let guard = state_handle.lock().await;
            guard.tasks.contains_key(&anchor_id)
        };
        if is_recording {
            sleep(Duration::from_secs(anchor_cfg.check_interval_secs)).await;
            continue;
        }

        let base_interval = Duration::from_secs(anchor_cfg.check_interval_secs);

        match spider::get_live_info(
            &anchor_cfg.url,
            anchor_cfg.proxy.as_deref(),
            anchor_cfg.cookie.as_deref(),
        )
        .await
        {
            Ok(info) => {
                if info.is_live {
                    log_to_file(&format!(
                        "自动检测到主播 {} 开播，开始录制",
                        anchor_cfg.name
                    ));
                    let ffmpeg_path =
                        match utils::check_ffmpeg(config.ffmpeg_path.as_deref()).await {
                            Ok(p) => p,
                            Err(e) => {
                                log_to_file(&format!("自动录制：FFmpeg 不可用: {}", e));
                                sleep(base_interval).await;
                                continue;
                            }
                        };

                    let cancel_token = tokio_util::sync::CancellationToken::new();
                    let child_token = cancel_token.child_token();
                    let config_clone = config.clone();
                    let anchor_name = info.anchor_name.clone();
                    let title = info.title.clone();
                    let anchor_id_clone = anchor_id.clone();

                    let task = tokio::spawn(async move {
                        let _ = recorder::start_recording(
                            &config_clone,
                            &anchor_name,
                            title.as_deref(),
                            &ffmpeg_path,
                            child_token,
                            &anchor_id_clone,
                        )
                        .await;
                    });

                    {
                        let mut guard = state_handle.lock().await;
                        guard.tasks.insert(
                            anchor_id.clone(),
                            crate::state::AnchorTask {
                                cancel_token,
                                handle: task,
                            },
                        );
                    }
                }
            }
            Err(e) => {
                log_to_file(&format!("自动检测主播 {} 失败: {}", anchor_cfg.name, e));
            }
        }

        // 开播检测用基础间隔，否则用回退间隔节省 API
        // 注：这里简化处理，实际可在上面循环中记录状态
        sleep(base_interval).await;
    }
}

/// 应用入口
pub fn run_app() {
    panic::set_hook(Box::new(|panic_info| {
        let msg = format!("PANIC: {:?}", panic_info);
        log_to_file(&msg);
        eprintln!("{}", msg);
    }));

    log_to_file("程序启动");
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    tauri::Builder::default()
        .setup(|app| {
            log_to_file("setup 开始");

            let recorder_state = state::RecorderState::default();
            app.manage(recorder_state);

            let app_handle = app.handle().clone();
            let state_handle = {
                let state = app_handle.try_state::<RecorderState>().unwrap();
                state.state.clone()
            };

            tauri::async_runtime::spawn(async move {
                log_to_file("后台自动录制任务启动");
                let config = match crate::config::manager::load_global_config() {
                    Ok(c) => c,
                    Err(e) => {
                        log_to_file(&format!("配置加载失败: {}", e));
                        eprintln!("启动时加载配置失败: {}", e);
                        return;
                    }
                };
                // 更新状态中的配置
                {
                    let mut guard = state_handle.lock().await;
                    guard.config = config.clone();
                }

                // 运行一次性检查
                {
                    let app_clone = app_handle.clone();
                    let config_clone = config.clone();
                    tauri::async_runtime::spawn(async move {
                        checker::run_all_checks(app_clone, &config_clone).await;
                    });
                }

                // 为每个主播生成独立检测任务
                let mut tasks = Vec::new();
                for anchor in &config.anchors {
                    let app_clone = app_handle.clone();
                    let state_clone = state_handle.clone();
                    let anchor_id = anchor.id.clone();
                    let task = tokio::spawn(async move {
                        run_auto_recording_for_anchor(app_clone, state_clone, anchor_id).await;
                    });
                    tasks.push(task);
                }

                // 等待所有任务结束
                for task in tasks {
                    if let Err(e) = task.await {
                        log_to_file(&format!("主播检测任务出错: {:?}", e));
                    }
                }
            });

            log_to_file("setup 完成");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_anchors,
            commands::start_recording_anchor,
            commands::stop_recording_anchor,
            commands::add_anchor,
            commands::remove_anchor,
            commands::update_anchor_config,
            commands::get_global_config,
            commands::update_global_config,
            commands::run_all_checks_backend,
            commands::get_audio_files,
            commands::rename_audio_file,
            commands::delete_audio_file,
            commands::get_avatar_base64,
            commands::open_with_system,
            commands::open_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
