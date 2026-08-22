use std::time::Duration;
use tauri::WebviewWindow;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use crate::domain::config::manager::ConfigManager;
use crate::domain::config::model::{AnchorConfig, AnchorStatusUpdate, GlobalConfig};
use crate::domain::recorder::disk::{
    check_disk_space, DiskSpaceStatus, CRASH_BACKOFF_THRESHOLD,
};
use crate::domain::recorder::engine::{
    is_abnormal_exit, is_clean_exit, mark_crash_partials, ChildProbe, FfmpegRecorder,
    RecorderEngine,
};
use crate::domain::services::cleanup::cleanup_on_recording_end;
use crate::domain::services::file_cache::{FileCacheHandle, FileCacheManager};
use crate::domain::spider::MissevanClient;
use crate::infrastructure::state::app_state::{AppStateHandle, RecordingSummary};
use crate::tr;
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

/// M6：从主播列表取指定主播的「直播状态探测 Cookie」——与检测循环
/// （detector/loop.rs 的 `check_live(&room_id, anchor.cookie.as_deref())`）保持
/// 同一携带语义：登录房间（需 Cookie）在录制期间的 API 探测同样携带，避免
/// 探测失败被误判离线而停止录制。主播不存在 / 未配置 Cookie → None（探测
/// 不带 Cookie，与修复前行为一致）。
fn anchor_probe_cookie<'a>(anchors: &'a [AnchorConfig], anchor_id: &str) -> Option<&'a str> {
    anchors
        .iter()
        .find(|a| a.id == anchor_id)
        .and_then(|a| a.cookie.as_deref())
}

// ── S3：录制运行中磁盘定期检查 ──
/// 磁盘检查周期（监控 tick 数）：10s × 30 = 约 5 分钟一次低开销 statfs
const DISK_CHECK_EVERY_TICKS: u64 = 30;
// ── S2b：熔断恢复 ──
/// 「状态探针成功」恢复条件：录制稳定运行 ≥60s 且探针显示进程存活 → 清零崩溃计数
const HEALTHY_RUN_RESET_AFTER: std::time::Duration = std::time::Duration::from_secs(60);

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
    // 录制后动作 open_folder 需要 AppHandle（opener 插件）；window 随后被移入
    // cleanup_on_recording_end / FileCacheManager，故提前克隆
    let app_handle = window.app_handle().clone();
    // 最近一次 API 直播判定（默认 true：录制启动时流存在）；
    // 录制结束时推送该值（而非硬编码 false），避免直播实际仍在时误显离线
    let mut last_api_live = true;
    // B1：FFmpeg 子进程异常退出标记（REC_CRASH 路径）。true 时收尾跳过自动清理
    // 与录制后动作——崩溃产生的半成品文件不应触发保留期清理判定，也不应执行
    // 「打开文件夹/自定义命令」等面向完整录制的后动作。
    let mut abnormal_exit = false;
    // S3：监控 tick 计数（10s/tick，驱动周期性磁盘检查）
    let mut tick_count: u64 = 0;

    notifier
        .info(
            "REC_START",
            tr!("recorder.start_title", name = anchor_name),
            tr!("recorder.start_body", name = anchor_name),
        )
        .await;

    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => {
                notifier.info("REC_STOP", tr!("recorder.cancelled_title", name = anchor_name), tr!("recorder.cancelled_body")).await;
                break;
            }
            _ = sleep(Duration::from_secs(10)) => {
                tick_count += 1;
                if start_time.elapsed() > max_duration {
                    notifier.error("REC_TIMEOUT", tr!("recorder.timeout_title", name = anchor_name), tr!("recorder.timeout_body")).await;
                    break;
                }

                // B1（对抗式审查）：FFmpeg 子进程存活探测——ffmpeg 因磁盘满/流
                // 中断等崩溃后，任务表不得残留「录制中」（否则该主播检测门控被
                // 占、永不重启录制——静默丢数据）。probe 为同步 try_wait（与停止
                // 流程经共享句柄互斥）；判定见 is_abnormal_exit：仅「已退出且非
                // 用户取消」算异常退出（停止流程中 cancel 已触发，子进程退出属
                // 正常停止，不误判）。异常退出后任务/进程条目由统一收尾移除，
                // 检测循环下一轮看到「直播中 + 未在录」即自动重启录制（loop.rs
                // 门控：enable_check && is_live && !already_recording）。
                let probe = recorder.probe_process(&anchor_id);
                // S2b 恢复条件「状态探针成功」：录制稳定运行 ≥60s 且探针显示
                // 进程存活 → 清零崩溃熔断计数（证明管道健康，后续崩溃从 1
                // 重新计数，避免历史崩溃无限累积退避）
                if matches!(probe, ChildProbe::Running)
                    && start_time.elapsed() >= HEALTHY_RUN_RESET_AFTER
                {
                    app_state.lock().await.reset_crash(&anchor_id);
                }
                if is_abnormal_exit(probe, cancel_token.is_cancelled()) {
                    abnormal_exit = true;
                    let exit_code = match probe {
                        ChildProbe::Exited(s) => s.code(),
                        _ => None,
                    };
                    notifier
                        .error(
                            "REC_CRASH",
                            tr!("recorder.crash_title", name = anchor_name),
                            tr!("recorder.crash_body", exit = format!("{:?}", exit_code)),
                        )
                        .await;
                    // S2b：上报崩溃熔断计数（连续达阈值后，检测循环门控暂停
                    // 自动重启，退避期内不再产生 REC_START/REC_CRASH 通知对）
                    let crash_count = app_state.lock().await.record_crash(&anchor_id);
                    tracing::warn!(
                        "{}",
                        tr!(
                            "recorder.crash_recorded",
                            count = crash_count,
                            threshold = CRASH_BACKOFF_THRESHOLD,
                            name = anchor_name
                        )
                    );
                    // H5：崩溃产物处置——本次崩溃产生的半成品文件（主输出 / 已写
                    // 出的分段）改名为 `.part` 标记（改名失败时直接删除）并记录
                    // 告警；绝不删除其他文件。auto_cleanup 开关控制的是「录制结束
                    // 的保留期/总量清理」（cleanup.rs），不适用于崩溃残留——半成品
                    // 不随用户语义保留，但保留为 `.part` 可恢复形态，下次启动由
                    // 启动清理（cleanup_orphan_recordings，H3）识别并删除。
                    // 改名标记 + 日志告知：用户可自行检查/删除 .part 文件。
                    if config.segment_seconds > 0 {
                        mark_crash_partials(&output_path, true, &config.record_format);
                    } else {
                        mark_crash_partials(&output_path, false, &config.record_format);
                    }
                    break;
                }

                // B1 补充：ffmpeg 以成功码退出（exit 0——流 EOF，主播下播或流
                // 断开）且非用户取消 → 按正常结束收尾（abnormal_exit=false：
                // 保留完整文件、移除录制标记、触发录制后动作）。检测循环下一轮
                // 按主播直播状态决定是否自动重启——主播仍在播则自动恢复录制，
                // 已下播则不重启。
                // 边界修复：exit 0 不再误判崩溃（is_abnormal_exit 修复）后，若
                // 此处不主动结束，进程已死而任务仍挂起，将空转到 API 判定下播/
                // 手动停止/24h 超时，失去旧逻辑意外提供的「流断即重启」重连能力。
                if is_clean_exit(probe, cancel_token.is_cancelled()) {
                    notifier
                        .info(
                            "REC_ENDED",
                            tr!("recorder.ended_title", name = anchor_name),
                            tr!("recorder.ended_body"),
                        )
                        .await;
                    break;
                }

                // 兜底（方案 C 防漏）：录制期间主播的「检测与自动录制」被关闭
                //（update_anchor 保存即停为主路径，此处低频兜底覆盖竞态/其他写
                // 路径）。monitor 持有的是录制启动时的主播快照，enable_check 的
                // 最新值须实时读配置；配置读取失败时跳过（保持录制，避免误停）。
                // M6：直播探测 Cookie 也取自同一次 load() 的最新配置——与检测
                // 循环（loop.rs 传 anchor.cookie.as_deref()）同一携带语义：登录
                // 房间（需 Cookie）录制启动约 30s 后探测不因缺 Cookie 被误判
                // 离线。复用本 tick 已加载的配置，不新增磁盘 IO（M7 缓存后该
                // load 走内存）。主播已删除时 remove_anchor 已取消录制任务
                //（cancel 分支先退出），此处取不到 Cookie 回退 None（不影响
                // B1 崩溃检测：探测请求本身不依赖 Cookie 成功与否）。
                // S3：磁盘阈值（disk_space_limit_gb）与 enable_check 同取自本次
                // load（不新增磁盘 IO）
                let (probe_cookie, disk_threshold_gb): (Option<String>, u64) =
                    match config_manager.load() {
                        Ok(cfg) => {
                            let anchor = cfg.anchors.iter().find(|a| a.id == anchor_id);
                            let check_disabled = anchor.is_some_and(|a| !a.enable_check);
                            if check_disabled {
                                notifier.info("REC_STOP_CHECK_DISABLED", tr!("recorder.check_disabled_title", name = anchor_name), tr!("recorder.check_disabled_body")).await;
                                break;
                            }
                            (
                                anchor_probe_cookie(&cfg.anchors, &anchor_id).map(str::to_owned),
                                cfg.global.disk_space_limit_gb,
                            )
                        }
                        Err(e) => {
                            tracing::warn!("{}", tr!("recorder.config_load_failed_skip", err = e));
                            (None, 0)
                        }
                    };

                // S3：磁盘阈值运行中检查（disk_space_limit_gb 预警激活）——每
                // DISK_CHECK_EVERY_TICKS 个 tick（约 5 分钟）一次低开销 statfs。
                // 低于阈值发节流 DISK_LOW 预警（与 S2a 启动前检查共用 AppState
                // 冷却，不刷屏）；**不**主动停正在进行的录制——阈值语义为「暂停
                // 新录制」（前端提示同义），运行中录制照常进行，待正常结束后由
                // 启动前检查（engine.rs S2a）拦截后续录制。
                if tick_count % DISK_CHECK_EVERY_TICKS == 0 && disk_threshold_gb > 0 {
                    let check_dir = std::path::Path::new(&output_path)
                        .parent()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if let DiskSpaceStatus::Low {
                        available_gb,
                        threshold_gb,
                    } = check_disk_space(&check_dir, disk_threshold_gb)
                    {
                        let should_notify = app_state.lock().await.disk_notify_allowed();
                        if should_notify {
                            notifier
                                .warning(
                                    "DISK_LOW",
                                    tr!("recorder.disk_low_warn_title"),
                                    tr!(
                                        "recorder.disk_low_warn_body",
                                        available_gb = available_gb,
                                        threshold_gb = threshold_gb
                                    ),
                                )
                                .await;
                        }
                        tracing::warn!(
                            "{}",
                            tr!(
                                "recorder.disk_low_cleanup_warn",
                                available_gb = available_gb,
                                threshold_gb = threshold_gb
                            )
                        );
                    }
                }

                match client.check_live(&room_id, probe_cookie.as_deref()).await {
                    Ok(result) => {
                        consecutive_api_failures = 0;
                        last_api_live = result.is_live;
                        if !result.is_live {
                            notifier.info("REC_ENDED", tr!("recorder.live_ended_title", name = anchor_name), tr!("recorder.live_ended_body")).await;
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
                                "{}",
                                tr!("recorder.api_transient_error", name = anchor_name, err = e)
                            );
                        } else {
                            consecutive_api_failures += 1;
                            notifier.warning("REC_API_ERR", tr!("recorder.api_check_failed_title", count = consecutive_api_failures, max = MAX_API_FAILURES, name = anchor_name), e.message().to_string()).await;
                            if consecutive_api_failures >= MAX_API_FAILURES {
                                notifier.error("REC_API_FAILED", tr!("recorder.api_failed_stop_title", name = anchor_name), tr!("recorder.api_failed_stop_body")).await;
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

    // H3 配套：正常收尾（停止/取消/直播结束/超时）移除活动录制标记——崩溃路径
    // 保留标记（mark_crash_partials 已把产物改名 .part），供下次启动
    //（cleanup_orphan_recordings）识别并清理残留；此处仅正常路径移除。
    if !abnormal_exit {
        crate::domain::recorder::engine::remove_recording_marker(&output_path);
    }

    // 从 AppState 中移除录制任务
    app_state.lock().await.remove_task(&anchor_id);

    // S2b 恢复条件「正常结束/手动操作」：非崩溃收尾清零熔断计数——正常结束、
    // 用户取消、直播结束等均证明管道健康，后续崩溃从 1 重新计数。崩溃路径
    // （REC_CRASH）保持计数：检测循环门控据此暂停自动重启并指数退避。
    if !abnormal_exit {
        app_state.lock().await.reset_crash(&anchor_id);
    }

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
    tracing::info!("{}", tr!("recorder.task_removed", anchor_id = anchor_id));

    // 刷新文件缓存，让前端立刻看到新文件
    // （任务已从 AppState 移除，刷新时该文件不会再被标记为「录制中」）
    let cache_manager = FileCacheManager::new(window.clone(), file_cache.clone());
    if let Err(e) = cache_manager.refresh(&config_manager, &app_state).await {
        tracing::error!("{}", tr!("recorder.cache_refresh_failed", err = e));
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
    // 录制后动作（§11.1 post_record_action / post_record_command）——正常结束/
    // 取消/错误全部汇聚于此统一出口；命令失败仅 warn，不阻断录制结束流程
    //（open_folder 分支需要 AppHandle：见函数头部 app_handle 克隆）
    // 异常退出（B1，REC_CRASH 路径）除外：跳过自动清理与录制后动作（见上方
    // abnormal_exit 注释）。
    if !abnormal_exit {
        cleanup_on_recording_end(window, file_cache, config_manager.clone(), app_state.clone()).await;
        run_post_record_action(
            &config,
            &output_path,
            &anchor_name,
            &room_id,
            Some(&app_handle),
        );
    }
}

/// 命令变量替换 + 智能引号包裹 + 变量值消毒（M1 命令注入修复）。
///
/// 替换 `{file}` / `{output_dir}` / `{anchor_name}` / `{room_id}` 四个变量。
/// 变量值先经 `sanitize_variable_value` 消毒（双引号包裹挡不住 sh 的 `$`/
/// 反引号命令替换与 cmd 的 `%VAR%` 展开——见该函数注释），再统一用双引号
/// 包裹：含空格路径在 cmd /C 下不会被拆成多个参数，含 `&` / `|` / `^` 的
/// 文件名不会被 shell 解释为管道/命令分隔符。
///
/// 不双重包裹：用户手写命令里变量两侧紧邻字符**任一**已是双引号（如
/// `copy "{file}" dest`）时按原样替换，避免产生 `""C:\a b\x.m4a""`。
fn substitute_command_variables(
    template: &str,
    file: &str,
    output_dir: &str,
    anchor_name: &str,
    room_id: &str,
) -> String {
    let mut out = template.to_string();
    for (token, value) in [
        ("{file}", sanitize_variable_value(file)),
        ("{output_dir}", sanitize_variable_value(output_dir)),
        ("{anchor_name}", sanitize_variable_value(anchor_name)),
        ("{room_id}", sanitize_variable_value(room_id)),
    ] {
        out = replace_token_quoted(&out, token, &value);
    }
    out
}

/// 变量值消毒（M1 命令注入修复核心）：主播名/房间号来自 Missevan API（远程
/// 可控），输出路径来自文件系统（目录名同样可能源自主播名）——这些值在拼入
/// shell 命令前必须消毒。双引号包裹只能防拆词与 `&`/`|`/`<`/`>` 解释，挡不住
/// 双引号内**仍然生效**的元字符：
/// - POSIX sh：双引号内 `$`（`$VAR` / `${...}` / `$(...)`）与反引号仍触发
///   命令替换/变量展开，`\` 仍作转义符（值尾 `\` 可吃掉包裹引号）；
/// - cmd /C：双引号内 `%VAR%`（环境变量展开）与 `!VAR!`（延迟展开）仍生效。
///
/// 按平台处理（cmd 与 sh 元字符集不同，不能共用一套消毒）：
/// - Unix（sh -c）：`$` / 反引号 / `"` / `\` 前置 `\` 转义——POSIX 双引号内
///   `\$` `` \` `` `\"` `\\` 均按字面量输出，命令语义不变（`\\` → 字面 `\`）；
///   控制字符（含换行）直接删除。Unix 路径用 `/`，值内 `\` 本就罕见，转义
///   后经 shell 还原为同一字面值。
/// - Windows（cmd /C）：`%` / `!` / `"` 直接删除——cmd 引号内没有转义机制，
///   无法表达字面 `%`/`!`；`"` 在 Windows 路径中非法且主播名/房间号已消毒，
///   出现即删（纵深防御）。`\` 是路径分隔符必须保留；`&` `|` `<` `>` `^` 在
///   双引号内本就按字面处理，无需处理。
///
/// 取舍：删除/转义会改变值本身（如 Windows 文件名含 `%` 时路径对不上），但
/// 这类字符在合法文件名/主播名中属罕见，安全性优先；命令最多引用不到该文件，
/// 绝不会执行注入内容。
fn sanitize_variable_value(value: &str) -> String {
    #[cfg(windows)]
    {
        value
            .chars()
            .filter(|c| !matches!(c, '%' | '!' | '"' | '\u{0}'..='\u{1f}'))
            .collect()
    }
    #[cfg(not(windows))]
    {
        let mut out = String::with_capacity(value.len());
        for c in value.chars() {
            match c {
                '$' | '`' | '"' | '\\' => out.push('\\'),
                '\u{0}'..='\u{1f}' => continue,
                _ => {}
            }
            out.push(c);
        }
        out
    }
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

/// 录制后动作：`none`（默认，不操作）/ `open_folder`（opener 插件打开录制文件
/// 所在文件夹）/ `command`（执行自定义命令，变量替换 `{file}` /
/// `{output_dir}` / `{anchor_name}` / `{room_id}`）。
///
/// `app` 为 `Option<&AppHandle>`：生产路径由调用方传入（`open_folder` 分支
/// 需要）；单测不构造 AppHandle，传 None（测试仅覆盖 none/unknown/command 分支）。
///
/// 命令经系统 shell 执行（Windows `cmd /C`，其余平台 `sh -c`），spawn 后由
/// 后台线程 wait 回收（M4）——录制结束流程不被外部命令阻塞，且 Linux 上不会
/// 因不 wait 留下僵尸进程。失败只记 warn（不阻断）。
///
/// 变量值先经 `sanitize_variable_value` 消毒再统一用双引号包裹（M1 命令注入
/// 修复）：含空格路径在 cmd /C 与 sh -c 下都不会被拆词，含 `&` / `|` 的文件名
/// 不会被 shell 解释为管道/命令分隔（双引号内 POSIX shell 同样按字面处理
/// `&`/`|`/空格）；sh 的 `$`/反引号与 cmd 的 `%VAR%`/`!VAR!` 展开由消毒层
/// 显式处理（转义/删除），注入 payload 只按字面出现、绝不执行。用户手写命令
/// 已带引号（如 `"{file}"`）时不双重包裹。已知限制：消毒会改变值本身（如
/// Windows 文件名含 `%` 时路径对不上、Unix 值内 `\` 经转义还原为字面 `\`），
/// 但这类字符在合法文件名/主播名中属罕见，安全性优先。
fn run_post_record_action(
    config: &GlobalConfig,
    output_path: &str,
    anchor_name: &str,
    room_id: &str,
    app: Option<&tauri::AppHandle>,
) {
    match config.post_record_action.as_str() {
        "open_folder" => {
            // opener 插件 open_path 打开目录本身（Windows 资源管理器 / Linux
            // xdg-open）；文件路径 → 打开其父目录（「文件管理器中定位并选中」
            // 语义保留给托盘菜单的 reveal_item_in_dir，见 tray/mod.rs）
            let target = std::path::Path::new(output_path);
            let target = if target.is_dir() {
                target.to_path_buf()
            } else {
                target
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| target.to_path_buf())
            };
            match app {
                Some(app) => {
                    // tauri_plugin_opener::Opener::open_path(path: impl Into<String>, with: Option<impl Into<String>>)
                    if let Err(e) = app
                        .opener()
                        .open_path(target.to_string_lossy().into_owned(), None::<&str>)
                    {
                        tracing::warn!("{}", tr!("recorder.post_open_folder_failed", err = e));
                    } else {
                        tracing::info!("{}", tr!("recorder.post_folder_opened", path = output_path));
                    }
                }
                None => {
                    tracing::warn!("{}", tr!("recorder.post_open_folder_no_handle"));
                }
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
            tracing::info!("{}", tr!("recorder.post_executing_command", cmd = cmd));
            // 隐藏控制台（tools.rs::apply_create_no_window）：cmd /C 是控制台
            // 子系统，发布构建无控制台时会弹黑窗口。取舍：CREATE_NO_WINDOW 只
            // 隐藏 cmd 自身的控制台窗口，用户命令内启动的 GUI 程序窗口不受影响
            #[cfg(windows)]
            let spawn = {
                use std::os::windows::process::CommandExt;
                let mut process_cmd = std::process::Command::new("cmd");
                // raw_arg：整条命令串**原样**传给 cmd /C。不能用 args()——Rust 会
                // 把串内 `"` 转义为 `\"`，cmd /C 解析后引号丢失（echo 多出反斜杠、
                // 重定向目标解析失败），变量替换产生的引号包裹全部失效
                // （M1 E2E 实测：args 版 status=1 报「文件名语法不正确」）。
                // 注入防护不依赖此处：变量值已由 sanitize_variable_value 消毒
                process_cmd.raw_arg("/C").raw_arg(&cmd);
                crate::domain::tools::apply_create_no_window(&mut process_cmd);
                process_cmd.spawn()
            };
            #[cfg(not(windows))]
            let spawn = std::process::Command::new("sh").args(["-c", &cmd]).spawn();
            match spawn {
                Ok(child) => {
                    // M4：后台线程 wait 回收——Linux 上 spawn 后不 wait，子进程
                    // 退出即成为 defunct 僵尸进程（7×24 运行累积数百个）。回收
                    // 线程在子进程退出后即结束，不阻塞录制结束流程（与「spawn
                    // 后不等待」的既有语义一致）；Windows 无僵尸概念但路径统一。
                    crate::domain::tools::reap_in_background(child);
                    tracing::info!("{}", tr!("recorder.post_command_started"));
                }
                Err(e) => tracing::warn!("{}", tr!("recorder.post_command_spawn_failed", err = e)),
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
            None,
        );
    }

    #[test]
    fn post_action_unknown_falls_back_to_noop() {
        run_post_record_action(
            &config_with("explode", "rm -rf /"),
            r"D:\rec\x.m4a",
            "x",
            "1",
            None,
        );
    }

    #[test]
    fn post_action_command_variable_substitution() {
        // 变量替换的纯函数部分：`{file}` / `{output_dir}` / `{anchor_name}` / `{room_id}`
        let output_path = r"D:\rec\主播A\2026-08-07_12-30-45_主播A.m4a";
        let cmd = "echo {file} {output_dir} {anchor_name} {room_id}".to_string();
        // 注意：不能通过 Path::parent() 从 Windows 路径推导——Unix 上 `\` 不是路径
        // 分隔符，parent() 返回空串导致 {output_dir} 被替换为空（Linux CI 回归）。
        let output_dir = r"D:\rec\主播A".to_string();
        let substituted =
            substitute_command_variables(&cmd, output_path, &output_dir, "主播A", "123456");
        // 无空格路径也被双引号包裹（统一包裹策略——含空格/&/| 时防拆词与解释）
        // Unix 侧值内 `\` 被转义为 `\\`（sh 双引号内 `\\` → 字面 `\`，
        // 命令实际拿到的参数不变，仅原始替换串不同）
        let expected = if cfg!(windows) {
            r#"echo "D:\rec\主播A\2026-08-07_12-30-45_主播A.m4a" "D:\rec\主播A" "主播A" "123456""#
        } else {
            r#"echo "D:\\rec\\主播A\\2026-08-07_12-30-45_主播A.m4a" "D:\\rec\\主播A" "主播A" "123456""#
        };
        assert_eq!(substituted, expected);
    }

    #[test]
    fn command_substitution_quotes_paths_with_spaces_and_metachars() {
        // 含空格路径：不加引号会在 cmd /C 下拆词；含 & 会被 shell 解释——都必须引住
        let file = r"D:\rec\my anchor\2026-08-07 live&fun.m4a";
        let output_dir = r"D:\rec\my anchor";
        let cmd = "echo {file} {output_dir}";
        let substituted = substitute_command_variables(cmd, file, output_dir, "主播A", "1");
        let expected = if cfg!(windows) {
            r#"echo "D:\rec\my anchor\2026-08-07 live&fun.m4a" "D:\rec\my anchor""#
        } else {
            r#"echo "D:\\rec\\my anchor\\2026-08-07 live&fun.m4a" "D:\\rec\\my anchor""#
        };
        assert_eq!(
            substituted,
            expected,
            "含空格/& 的路径必须整体引住（& 在引号内不被 shell 解释）"
        );
        // 管道符同理
        let file2 = r"D:\rec\a|b.m4a";
        let s2 = substitute_command_variables("move {file} dest", file2, "D:/rec", "x", "1");
        let expected2 = if cfg!(windows) {
            r#"move "D:\rec\a|b.m4a" dest"#
        } else {
            r#"move "D:\\rec\\a|b.m4a" dest"#
        };
        assert_eq!(s2, expected2);
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
        let expected = if cfg!(windows) {
            r#"copy "D:\rec a\2026-08-07_x.m4a" "D:\rec a" /Y"#
        } else {
            r#"copy "D:\\rec a\\2026-08-07_x.m4a" "D:\\rec a" /Y"#
        };
        assert_eq!(substituted, expected);
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
        run_post_record_action(&config_with("command", "   "), "x.m4a", "x", "1", None);
    }

    // ── M6：录制期间直播探测携带主播 Cookie（与检测循环一致）──

    fn anchor_with_cookie(id: &str, cookie: Option<&str>) -> AnchorConfig {
        AnchorConfig {
            id: id.to_string(),
            name: "主播".to_string(),
            url: "https://m.missevan.com/live/1".to_string(),
            room_id: "1".to_string(),
            proxy: None,
            cookie: cookie.map(String::from),
            enable_check: true,
            avatar_url: None,
            tags: Vec::new(),
        }
    }

    #[test]
    fn anchor_probe_cookie_returns_cookie_of_matching_anchor() {
        let anchors = vec![
            anchor_with_cookie("a1", Some("ck=login")),
            anchor_with_cookie("a2", None),
        ];
        // 已配置 Cookie 的主播 → 探测携带其 Cookie（与检测循环一致）
        assert_eq!(anchor_probe_cookie(&anchors, "a1"), Some("ck=login"));
        // 未配置 Cookie 的主播 → None（探测不带 Cookie）
        assert_eq!(anchor_probe_cookie(&anchors, "a2"), None);
        // 未知主播 → None（不 panic）
        assert_eq!(anchor_probe_cookie(&anchors, "nope"), None);
        // 空列表 → None
        assert_eq!(anchor_probe_cookie(&[], "a1"), None);
    }

    #[test]
    fn anchor_probe_cookie_matches_by_id_not_position() {
        // id 精确匹配：同位置不同 id 不命中（避免列表顺序变化取错 Cookie）
        let anchors = vec![anchor_with_cookie("a1", Some("ck-1")), anchor_with_cookie("a2", Some("ck-2"))];
        assert_eq!(anchor_probe_cookie(&anchors, "a2"), Some("ck-2"));
        assert_eq!(anchor_probe_cookie(&anchors, "a1"), Some("ck-1"));
    }

    // ── M1：命令注入（变量值消毒）──

    #[test]
    fn command_substitution_neutralizes_sh_command_substitution_payloads() {
        // anchor_name/room_id 来自 Missevan API（远程可控）：`$()` / 反引号 /
        // `${}` payload 必须被消毒为字面量——sh 双引号内这些仍会命令替换，
        // 引号包裹挡不住，必须转义 `$`/反引号（Unix）或原样保留（Windows 的
        // cmd 不识别这些元字符，引号内即字面）
        let cmd = "echo {anchor_name} {room_id}";
        // $() 命令替换
        let s1 = substitute_command_variables(cmd, "x.m4a", "rec", "x$(touch /tmp/pwned)", "1");
        #[cfg(windows)]
        assert_eq!(s1, r#"echo "x$(touch /tmp/pwned)" "1""#);
        #[cfg(not(windows))]
        assert_eq!(s1, r#"echo "x\$(touch /tmp/pwned)" "1""#);
        // 反引号命令替换
        let s2 = substitute_command_variables(cmd, "x.m4a", "rec", "x`touch /tmp/pwned`", "2");
        #[cfg(windows)]
        assert_eq!(s2, r#"echo "x`touch /tmp/pwned`" "2""#);
        #[cfg(not(windows))]
        assert_eq!(s2, r#"echo "x\`touch /tmp/pwned\`" "2""#);
        // ${...} 展开
        let s3 = substitute_command_variables(cmd, "x.m4a", "rec", "x${IFS}touch", "3");
        #[cfg(windows)]
        assert_eq!(s3, r#"echo "x${IFS}touch" "3""#);
        #[cfg(not(windows))]
        assert_eq!(s3, r#"echo "x\${IFS}touch" "3""#);
    }

    #[cfg(windows)]
    #[test]
    fn command_substitution_neutralizes_cmd_env_expansion_payloads() {
        // cmd /C：双引号内 `%VAR%` / `!VAR!` 仍会展开且无法转义——`%`/`!` 直接删除
        let cmd = "echo {file} {anchor_name}";
        // 路径含 `%`（罕见）→ 删除后路径对不上，但绝不被展开为其他内容
        let s = substitute_command_variables(cmd, r"C:\100%rec\a.m4a", "rec", "x%COMSPEC%y", "1");
        assert_eq!(s, r#"echo "C:\100rec\a.m4a" "xCOMSPECy""#);
        let s2 = substitute_command_variables(cmd, "a.m4a", "rec", "x!PATH!y", "1");
        assert_eq!(s2, r#"echo "a.m4a" "xPATHy""#);
    }

    /// M1 端到端：消毒后的命令喂给真实 shell（cmd /C / sh -c），验证注入 payload
    /// 只按字面输出、绝不执行。payload 按平台选最强向量：
    /// - Windows：`%COMSPEC%`（cmd 双引号内也会展开为 cmd.exe 路径，消毒后删 `%`）
    /// - Unix：`$(touch <标记文件>)`（sh 双引号内命令替换，消毒后转义 `$`）
    /// 模板带输出重定向，shell 执行后从输出文件读回内容断言。
    #[test]
    fn post_action_command_injection_not_executed_end_to_end() {
        let tmp = std::env::temp_dir().join(format!("missevan_m1_e2e_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let out_file = tmp.join("out.txt");
        let marker = tmp.join("pwned");
        let _ = std::fs::remove_file(&out_file);
        let _ = std::fs::remove_file(&marker);
        let out_str = out_file.to_string_lossy().into_owned();

        #[cfg(windows)]
        let payload = "x%COMSPEC%y";
        #[cfg(not(windows))]
        let payload = {
            let marker_str = marker.to_string_lossy().into_owned();
            format!("x$(touch {})", marker_str)
        };

        let template = format!("echo {{anchor_name}} > \"{}\"", out_str);
        run_post_record_action(&config_with("command", &template), "x.m4a", &payload, "1", None);

        // run_post_record_action 不等待子进程——轮询输出文件（最长 5s）
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let content = loop {
            if let Ok(c) = std::fs::read_to_string(&out_file) {
                break c;
            }
            if std::time::Instant::now() > deadline {
                panic!("shell 未生成输出文件（命令启动失败？）");
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        // 输出必须是字面 payload（Windows 删 `%` 后为 xCOMSPECy；Unix 转义后
        // 原样输出 x$(touch ...)），不得出现展开/执行结果
        assert!(
            content.contains("xCOMSPECy") || content.contains("x$(touch"),
            "输出应为字面量，实际: {}",
            content
        );
        #[cfg(windows)]
        assert!(
            !content.contains("cmd.exe"),
            "环境变量被展开（注入生效）: {}",
            content
        );
        #[cfg(not(windows))]
        assert!(!marker.exists(), "命令替换被执行（标记文件被创建）");
        // 清理
        let _ = std::fs::remove_file(&out_file);
        let _ = std::fs::remove_file(&marker);
        let _ = std::fs::remove_dir(&tmp);
    }
}
