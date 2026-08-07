use std::time::Duration;
use tauri::WebviewWindow;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use crate::domain::config::manager::ConfigManager;
use crate::domain::config::model::{AnchorStatusUpdate, GlobalConfig};
use crate::domain::recorder::engine::{FfmpegRecorder, RecorderEngine};
use crate::domain::services::cleanup::cleanup_on_recording_end;
use crate::domain::services::file_cache::{FileCacheHandle, FileCacheManager};
use crate::domain::spider::MissevanClient;
use crate::infrastructure::state::app_state::{AppStateHandle, RecordingSummary};
use std::sync::Arc;
use tauri::Emitter;

pub async fn monitor_recording(
    anchor_id: String,
    anchor_name: String,
    room_id: String,
    output_path: String,
    cancel_token: CancellationToken,
    recorder: Arc<FfmpegRecorder>,
    client: MissevanClient,
    notifier: Arc<crate::infrastructure::notification::dispatcher::NotificationDispatcher>,
    // 录制后动作（post_record_action / post_record_command）消费；其余录制参数由 engine 侧消费
    config: GlobalConfig,
    app_state: AppStateHandle,
    window: WebviewWindow,              // 用于推送事件
    file_cache: FileCacheHandle,        // 文件缓存
    config_manager: Arc<ConfigManager>, // 配置管理器（用于刷新缓存时获取主播列表）
) {
    let start_time = std::time::Instant::now();
    let max_duration = Duration::from_secs(24 * 60 * 60);
    let mut consecutive_api_failures = 0;
    const MAX_API_FAILURES: u32 = 3;
    // 最近一次 API 直播判定（默认 true：录制启动时流存在）；
    // 录制结束时推送该值（而非硬编码 false），避免直播实际仍在时误显离线
    let mut last_api_live = true;

    notifier
        .info(
            "REC_START",
            format!("开始录制: {}", anchor_name),
            format!("主播 {} 的直播正在录制", anchor_name),
        )
        .await;

    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => {
                notifier.info("REC_STOP", format!("录制已取消: {}", anchor_name), "用户取消或直播结束".to_string()).await;
                break;
            }
            _ = sleep(Duration::from_secs(10)) => {
                if start_time.elapsed() > max_duration {
                    notifier.error("REC_TIMEOUT", format!("录制超时: {}", anchor_name), "超过 24 小时安全阀".to_string()).await;
                    break;
                }

                // 兜底（方案 C 防漏）：录制期间主播的「检测与自动录制」被关闭
                //（update_anchor 保存即停为主路径，此处低频兜底覆盖竞态/其他写
                // 路径）。monitor 持有的是录制启动时的主播快照，enable_check 的
                // 最新值须实时读配置；配置读取失败时跳过（保持录制，避免误停）。
                match config_manager.load() {
                    Ok(cfg) => {
                        let check_disabled = cfg
                            .anchors
                            .iter()
                            .find(|a| a.id == anchor_id)
                            .is_some_and(|a| !a.enable_check);
                        if check_disabled {
                            notifier.info("REC_STOP_CHECK_DISABLED", format!("已停止录制: {}", anchor_name), "主播的「启用检测与自动录制」已关闭".to_string()).await;
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::warn!("[录制] 读取配置失败（跳过检测开关兜底检查）: {}", e);
                    }
                }

                match client.check_live(&room_id, None).await {
                    Ok(result) => {
                        consecutive_api_failures = 0;
                        last_api_live = result.is_live;
                        if !result.is_live {
                            notifier.info("REC_ENDED", format!("直播结束: {}", anchor_name), "API 返回未直播状态".to_string()).await;
                            break;
                        }
                    }
                    Err(e) => {
                        // 错误分类（规格「直播状态异常修复」）：
                        // Server/Network/Format 为瞬时错误（5XX/429/网络抖动/格式变化），
                        // 不判离线也不计入失败阈值——FFmpeg 正在录说明流存在，避免风控
                        // 误报中断进行中的录制；仅「明确离线」（Other，如 404）计失败。
                        if e.is_transient() {
                            tracing::warn!(
                                "[录制] API 瞬时错误（不影响录制，保持直播判定）: {}: {}",
                                anchor_name,
                                e
                            );
                        } else {
                            consecutive_api_failures += 1;
                            notifier.warning("REC_API_ERR", format!("API 检测失败 ({}/{}): {}", consecutive_api_failures, MAX_API_FAILURES, anchor_name), e.message().to_string()).await;
                            if consecutive_api_failures >= MAX_API_FAILURES {
                                notifier.error("REC_API_FAILED", format!("API 连续失败，停止录制: {}", anchor_name), "连续 3 次 API 调用失败".to_string()).await;
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    // 停止 FFmpeg
    let _ = recorder.stop(&anchor_id).await;

    // 从 AppState 中移除录制任务
    app_state.lock().await.remove_task(&anchor_id);

    // 记录录制历史摘要（调试页「录制引擎」模块；最新在前）
    {
        let duration = start_time.elapsed().as_secs();
        let ended_at = chrono::Local::now();
        let started_at = ended_at - chrono::Duration::seconds(duration as i64);
        app_state.lock().await.record_history(RecordingSummary {
            anchor_id: anchor_id.clone(),
            anchor_name: anchor_name.clone(),
            room_id: room_id.clone(),
            output_path: output_path.clone(),
            started_at: started_at.to_rfc3339(),
            duration_secs: duration,
            ended_at: ended_at.to_rfc3339(),
        });
    }

    // 🔔 推送录制状态变为 false；is_live 用最近一次 API 判定（双重验证语义下
    // 录制中一直保持「直播中」；结束时的直播状态由下一轮检测循环校正）
    let update = AnchorStatusUpdate {
        anchor_id: anchor_id.clone(),
        is_live: last_api_live,
        is_recording: false,
    };
    let _ = window.emit("recording_status_changed", &update);
    tracing::info!("录制任务已从状态中移除: {}", anchor_id);

    // 刷新文件缓存，让前端立刻看到新文件
    // （任务已从 AppState 移除，刷新时该文件不会再被标记为「录制中」）
    let cache_manager = FileCacheManager::new(window.clone(), file_cache.clone());
    if let Err(e) = cache_manager.refresh(&config_manager, &app_state).await {
        tracing::error!("文件缓存刷新失败: {}", e);
    }

    // 录制结束自动清理（§11.1 auto_cleanup_enabled）：正常结束/取消/错误全部
    // 汇聚于此统一出口，每次录制结束触发一次清理检查——读**最新**配置（而非
    // 录制启动时的快照），启用时按 retention_days / max_total_gb 清理旧文件
    //（cleanup_on_recording_end 内部复用 run_cleanup：刷新文件缓存并 emit
    // `recording_files_changed`）。替代原 cleanup_time 每日定时调度
    //（cleanup_scheduler 已删除）。
    // 顺序：recorder.stop + 任务移除 + 文件缓存刷新之后、post_record_action
    // 之前——新录制文件先入缓存，且「打开文件夹」等录制后动作看到的是清理
    // 完成后的目录。
    cleanup_on_recording_end(window, file_cache, config_manager.clone(), app_state.clone()).await;

    // 录制后动作（§11.1 post_record_action / post_record_command）——正常结束/
    // 取消/错误全部汇聚于此统一出口；命令失败仅 warn，不阻断录制结束流程
    run_post_record_action(&config, &output_path, &anchor_name, &room_id);
}

/// 命令变量替换 + 智能引号包裹（实装审查跟进，Minor：命令注入/拆词）。
///
/// 替换 `{file}` / `{output_dir}` / `{anchor_name}` / `{room_id}` 四个变量。
/// 变量值统一用双引号包裹：含空格路径在 cmd /C 下不会被拆成多个参数，含
/// `&` / `|` / `^` 的文件名不会被 shell 解释为管道/命令分隔符。
///
/// 不双重包裹：用户手写命令里变量两侧紧邻字符**任一**已是双引号（如
/// `copy "{file}" dest`）时按原样替换，避免产生 `""C:\a b\x.m4a""`。
///
/// 已知限制（报告取舍）：值内含双引号不转义——Windows 路径本身不可能含 `"`，
/// 主播名/房间号经 `sanitize_path_component` 消毒后也不含，故无需转义；cmd
/// 的 `%VAR%` 环境变量展开语义不做处理（路径含 `%` 属罕见场景，超出本修复
/// 范围）。
fn substitute_command_variables(
    template: &str,
    file: &str,
    output_dir: &str,
    anchor_name: &str,
    room_id: &str,
) -> String {
    let mut out = template.to_string();
    for (token, value) in [
        ("{file}", file),
        ("{output_dir}", output_dir),
        ("{anchor_name}", anchor_name),
        ("{room_id}", room_id),
    ] {
        out = replace_token_quoted(&out, token, value);
    }
    out
}

/// 替换单个变量 token：紧邻字符（前或后）已含双引号 → 原样替换（用户自引）；
/// 否则值整体用双引号包裹。纯文本扫描，token 为 ASCII 故字节索引安全。
fn replace_token_quoted(input: &str, token: &str, value: &str) -> String {
    let mut out = String::with_capacity(input.len() + value.len() + 8);
    let mut rest = input;
    while let Some(pos) = rest.find(token) {
        out.push_str(&rest[..pos]);
        let prev_is_quote = rest[..pos].chars().next_back() == Some('"');
        let next_is_quote = rest[pos + token.len()..].chars().next() == Some('"');
        if prev_is_quote || next_is_quote {
            out.push_str(value);
        } else {
            out.push('"');
            out.push_str(value);
            out.push('"');
        }
        rest = &rest[pos + token.len()..];
    }
    out.push_str(rest);
    out
}

/// 录制后动作：`none`（默认，不操作）/ `open_folder`（资源管理器打开录制文件
/// 所在文件夹，选中文件）/ `command`（执行自定义命令，变量替换 `{file}` /
/// `{output_dir}` / `{anchor_name}` / `{room_id}`）。
///
/// 命令经系统 shell 执行（Windows `cmd /C`，其余平台 `sh -c`），spawn 后不
/// 等待——录制结束流程不被外部命令阻塞。失败只记 warn（不阻断）。
///
/// 变量值统一用双引号包裹（实装审查跟进）：含空格路径在 cmd /C 下不会被拆词，
/// 含 `&` / `|` 的文件名不会被 shell 解释为管道/命令分隔。用户手写命令已带
/// 引号（如 `"{file}"`）时不双重包裹。已知限制：值内含双引号不转义（Windows
/// 路径不可能含 `"`，主播名/房间号经 sanitize_path_component 消毒也不含）；
/// cmd 的 `%VAR%` 展开语义不做处理（路径含 `%` 时可能被展开，属罕见场景）。
fn run_post_record_action(
    config: &GlobalConfig,
    output_path: &str,
    anchor_name: &str,
    room_id: &str,
) {
    match config.post_record_action.as_str() {
        "open_folder" => {
            if let Err(e) = crate::domain::tools::open_in_explorer(std::path::Path::new(
                output_path,
            )) {
                tracing::warn!("[录制后] 打开文件夹失败: {}", e);
            } else {
                tracing::info!("[录制后] 已打开文件所在文件夹: {}", output_path);
            }
        }
        "command" if !config.post_record_command.trim().is_empty() => {
            let output_dir = std::path::Path::new(output_path)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let cmd = substitute_command_variables(
                &config.post_record_command,
                output_path,
                &output_dir,
                anchor_name,
                room_id,
            );
            tracing::info!("[录制后] 执行自定义命令: {}", cmd);
            // 隐藏控制台（tools.rs::apply_create_no_window）：cmd /C 是控制台
            // 子系统，发布构建无控制台时会弹黑窗口。取舍：CREATE_NO_WINDOW 只
            // 隐藏 cmd 自身的控制台窗口，用户命令内启动的 GUI 程序窗口不受影响
            #[cfg(windows)]
            let spawn = {
                let mut process_cmd = std::process::Command::new("cmd");
                process_cmd.args(["/C", &cmd]);
                crate::domain::tools::apply_create_no_window(&mut process_cmd);
                process_cmd.spawn()
            };
            #[cfg(not(windows))]
            let spawn = std::process::Command::new("sh").args(["-c", &cmd]).spawn();
            match spawn {
                Ok(_) => tracing::info!("[录制后] 自定义命令已启动"),
                Err(e) => tracing::warn!("[录制后] 启动自定义命令失败: {}", e),
            }
        }
        // none / 空命令 / 未知动作：不操作
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(action: &str, command: &str) -> GlobalConfig {
        let mut c = GlobalConfig::default();
        c.post_record_action = action.to_string();
        c.post_record_command = command.to_string();
        c
    }

    #[test]
    fn post_action_none_does_nothing() {
        // 不 panic 即通过（none 分支无副作用）
        run_post_record_action(
            &config_with("none", ""),
            r"D:\rec\主播A\2026-08-07_12-30-45_主播A.m4a",
            "主播A",
            "123456",
        );
    }

    #[test]
    fn post_action_unknown_falls_back_to_noop() {
        run_post_record_action(
            &config_with("explode", "rm -rf /"),
            r"D:\rec\x.m4a",
            "x",
            "1",
        );
    }

    #[test]
    fn post_action_command_variable_substitution() {
        // 变量替换的纯函数部分：`{file}` / `{output_dir}` / `{anchor_name}` / `{room_id}`
        let output_path = r"D:\rec\主播A\2026-08-07_12-30-45_主播A.m4a";
        let cmd = "echo {file} {output_dir} {anchor_name} {room_id}".to_string();
        let output_dir = std::path::Path::new(output_path)
            .parent()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let substituted =
            substitute_command_variables(&cmd, output_path, &output_dir, "主播A", "123456");
        // 无空格路径也被双引号包裹（统一包裹策略——含空格/&/| 时防拆词与解释）
        assert_eq!(
            substituted,
            r#"echo "D:\rec\主播A\2026-08-07_12-30-45_主播A.m4a" "D:\rec\主播A" "主播A" "123456""#
        );
    }

    #[test]
    fn command_substitution_quotes_paths_with_spaces_and_metachars() {
        // 含空格路径：不加引号会在 cmd /C 下拆词；含 & 会被 shell 解释——都必须引住
        let file = r"D:\rec\my anchor\2026-08-07 live&fun.m4a";
        let output_dir = r"D:\rec\my anchor";
        let cmd = "echo {file} {output_dir}";
        let substituted = substitute_command_variables(cmd, file, output_dir, "主播A", "1");
        assert_eq!(
            substituted,
            r#"echo "D:\rec\my anchor\2026-08-07 live&fun.m4a" "D:\rec\my anchor""#,
            "含空格/& 的路径必须整体引住（& 在引号内不被 shell 解释）"
        );
        // 管道符同理
        let file2 = r"D:\rec\a|b.m4a";
        let s2 = substitute_command_variables("move {file} dest", file2, "D:/rec", "x", "1");
        assert_eq!(s2, r#"move "D:\rec\a|b.m4a" dest"#);
    }

    #[test]
    fn command_substitution_does_not_double_quote() {
        // 用户手写命令已带引号（"{file}" / "{output_dir}"）→ 不双重包裹
        let cmd = r#"copy "{file}" "{output_dir}" /Y"#;
        let substituted = substitute_command_variables(
            cmd,
            r"D:\rec a\2026-08-07_x.m4a",
            r"D:\rec a",
            "主播A",
            "1",
        );
        assert_eq!(
            substituted,
            r#"copy "D:\rec a\2026-08-07_x.m4a" "D:\rec a" /Y"#
        );
        // 仅一侧有引号的写法也不双重包裹（用户自引优先）：
        // `"{file} {output_dir}"` 整体在一个引号对内——{file} 前引号、
        // {output_dir} 后引号，两者都不再包裹
        let cmd2 = r#"echo "{file} {output_dir}""#;
        let s2 = substitute_command_variables(cmd2, "C:/a.m4a", "C:/", "x", "1");
        assert_eq!(s2, r#"echo "C:/a.m4a C:/""#);
    }

    #[test]
    fn post_action_empty_command_skips_shell() {
        // 动作是 command 但命令为空 → 不执行（无 shell 调用）
        run_post_record_action(&config_with("command", "   "), "x.m4a", "x", "1");
    }
}
