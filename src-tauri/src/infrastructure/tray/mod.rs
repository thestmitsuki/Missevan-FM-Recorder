//! 系统托盘（规格 §1.1 系统托盘与后台运行 / 设计文档 §11.5，Task 17）
//!
//! [`TrayManager`] 封装托盘图标与右键菜单：
//! - **显示主窗口**：取消最小化 + 显示 + 聚焦
//! - **录制中：N**：实时显示活跃录制任务数；N>0 时可点击（显示主窗 + emit
//!   `tray:open_live_page`，供前端监听后导航到直播页）
//! - **最近录制**：子菜单，最多 5 条（取自 `AppState.history`，最新在前），
//!   点击用资源管理器选中该文件（`explorer /select,`）
//! - **退出应用**：优雅退出（保存配置 → 停检测循环 → cancel 所有录制任务 →
//!   等 JoinHandle ≤5s → `app.exit(0)`）
//!
//! 动态更新：后台轮询任务每 2s 读取 `AppState.active_count()` 与
//! `AppState.history`，与上次菜单数据比对，有变化才重建菜单（`TrayIcon::set_menu`）。
//! 选轮询而非监听 `recording_status_changed` / `recording_files_changed` 事件：
//! 这两个事件目前只 `window.emit` 到前端（无 AppHandle 级广播），轮询直接从
//! 共享状态取值，无事件接线改动、无竞态，2s 间隔开销可忽略。
//!
//! 可测性说明：托盘本体是平台 API（需要 AppHandle / 事件循环），无法单测；
//! 菜单数据构建（[`recent_files_from_history`]）、文件名截断
//! （[`truncated_label`]）、关闭行为决策（[`should_hide_to_tray`] /
//! [`decide_close_action`]）、
//! 菜单 id 解析（[`recent_index_from_menu_id`]）等纯逻辑拆为纯函数并配单测
//! （本模块 `#[cfg(test)]`）。
//!
//! **show_tray 简化（修复子代理 B）**：早期「是否显示系统托盘图标」是独立于
//! `close_behavior` 的开关，导致 `decide_close_action` 决策矩阵强耦合冗余
//! （show_tray=false 时 close_behavior=tray 也强制 Exit）。现托盘图标可见性改由
//! `close_behavior` 单一派生：「最小化到托盘」→ 自动显示图标（隐藏窗口后必须有
//! 图标可恢复）；「直接退出」→ 隐藏图标。`GlobalConfig.show_tray` 字段**保留**
//! 仅为兼容旧配置读取（serde 反序列化不报错），运行逻辑（`should_hide_to_tray` /
//! `decide_close_action` / `reconcile_tray` 调用方）不再消费该字段。
//!
//! **退出时配置保存时机（修复子代理 B）**：配置文件唯一写入点在向导最后一步
//! 「完成」按钮（前端 save_config 全量落盘）。[`do_shutdown`] 只保存**已存在**
//! 的配置（[`should_persist_on_shutdown`]）——首次运行向导中途退出绝不产生
//! config.toml（旧实现无条件 save_global 会把默认配置（wizard_completed=true）
//! 落盘，下次启动绕过向导直接进主页面）。

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tauri::menu::{IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_opener::OpenerExt;

use crate::domain::config::manager::ConfigManager;
use crate::domain::recorder::engine::FfmpegRecorder;
use crate::infrastructure::state::app_state::{AppStateHandle, RecorderState, RecordingSummary, Task};

/// 最近录制菜单上限（规格 1.1：最多 5 条）
const RECENT_LIMIT: usize = 5;
/// 菜单轮询刷新间隔
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
/// 录制文件名在菜单中的显示长度上限（字符数）
const LABEL_MAX_CHARS: usize = 40;
/// 退出时等待录制任务结束的超时（规格 1.2：等待 FFmpeg 进程结束或超时后退出）
const SHUTDOWN_WAIT: Duration = Duration::from_secs(5);

// ── 菜单项 id（MenuEvent 分发依据）──
const MENU_SHOW_MAIN: &str = "tray-show-main";
const MENU_RECORDING: &str = "tray-recording-count";
const MENU_RECENT_PREFIX: &str = "tray-recent-";
const MENU_RECENT_EMPTY: &str = "tray-recent-empty";
const MENU_EXIT: &str = "tray-exit";

/// 最近录制菜单项（纯数据，用于菜单构建与变化检测）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentFile {
    /// 菜单显示名（文件名，超长截断，`&` 已转义）
    pub label: String,
    /// 完整路径（点击用资源管理器选中）
    pub path: String,
}

/// 托盘菜单数据快照（`recording_count` 与 `recent_files` 任一变化都触发菜单重建）
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrayMenuData {
    pub recording_count: usize,
    pub recent_files: Vec<RecentFile>,
}

/// 从录制历史（最新在前）构建最近录制菜单数据，最多取 `limit` 条。
/// 历史记录由 monitor.rs 在任务结束时 `push_front`，天然有序。
pub fn recent_files_from_history<'a>(
    history: impl Iterator<Item = &'a RecordingSummary>,
    limit: usize,
) -> Vec<RecentFile> {
    history
        .take(limit)
        .map(|summary| RecentFile {
            label: file_label(&summary.output_path),
            path: summary.output_path.clone(),
        })
        .collect()
}

/// 菜单显示名：取文件名（失败时退回完整路径），超长截断，
/// 并转义 `&`（muda 将 `&` 视为 Windows 助记符前缀，`&&` 才显示字面 `&`）。
fn file_label(path: &str) -> String {
    let name = Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| path.to_string());
    sanitize_menu_text(&truncated_label(&name, LABEL_MAX_CHARS))
}

/// 截断长文本为 `max_chars` 字符（超出部分以 `...` 结尾；按字符截断，不切碎 UTF-8）
pub fn truncated_label(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let keep = max_chars.saturating_sub(3);
    let mut out: String = s.chars().take(keep).collect();
    out.push_str("...");
    out
}

/// 转义菜单文本中的 `&`（仅 Windows：muda 将 `&` 视为助记符前缀，`&&` 才显示
/// 字面 `&`）。Linux/GTK 菜单 `&` 无特殊含义，转义反而显示 `&&`，故原样返回。
fn sanitize_menu_text(s: &str) -> String {
    #[cfg(windows)]
    {
        s.replace('&', "&&")
    }
    #[cfg(not(windows))]
    {
        s.to_string()
    }
}

/// 从菜单项 id 解析最近录制索引（`tray-recent-0` → `Some(0)`）
fn recent_index_from_menu_id(id: &str) -> Option<usize> {
    id.strip_prefix(MENU_RECENT_PREFIX)?.parse().ok()
}

/// 关闭主窗时是否「最小化到托盘」（由 `close_behavior` 单一决定）。
///
/// 简化后（修复子代理 B）：show_tray 独立开关已移除——选择「最小化到托盘」
/// 即自动显示托盘图标（隐藏窗口后必须有图标可恢复），选择「直接退出」则
/// 隐藏图标。本函数同时充当**托盘图标可见性派生**（reconcile_tray / 启动
/// 创建托盘的调用方传 `should_hide_to_tray(&close_behavior)`）。
///
/// | close_behavior | 关闭主窗行为 |
/// |---|---|
/// | `"tray"` | `prevent_close` + `hide`，驻留托盘继续运行（托盘实际可用时） |
/// | `"exit"` | 直接退出 |
/// | 其他值 | 直接退出（未知值按 exit 保守处理） |
pub fn should_hide_to_tray(close_behavior: &str) -> bool {
    close_behavior == "tray"
}

/// 关闭主窗的动作
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// 拦截关闭：隐藏主窗，驻留托盘继续运行
    HideToTray,
    /// 不拦截：窗口正常关闭，随后统一优雅退出
    Exit,
}

/// 关闭主窗动作决策：close_behavior × 托盘**实际可用性**。
///
/// 与 [`should_hide_to_tray`] 的关系：后者只查 `close_behavior`；此处额外
/// 要求托盘实际存在且图标可见（`tray_enabled`），否则关闭主窗时隐藏窗口
/// 将无法恢复（托盘创建失败 / 运行中图标已隐藏时隐藏窗口 = 应用「人间蒸发」，
/// 只能任务管理器杀进程）。
///
/// `tray_enabled`：`None` = 托盘未创建（创建失败，`try_state` 取不到），
/// `Some(b)` = 托盘存在且图标可见性为 `b`（`TrayManager::enabled`）。
pub fn decide_close_action(
    close_behavior: &str,
    tray_enabled: Option<bool>,
) -> CloseAction {
    if should_hide_to_tray(close_behavior) && tray_enabled == Some(true) {
        CloseAction::HideToTray
    } else {
        CloseAction::Exit
    }
}

/// 退出时是否保存配置：仅当配置文件**已存在**时保存。
///
/// 根因修复（修复子代理 B）：`do_shutdown` 旧实现无条件 `save_global`——用户在
/// 向导第 1 步就关闭窗口也会把默认配置落盘（默认 `wizard_completed=true`），
/// 下次启动 `is_first_run` 判定为已完成 → 绕过向导直接进主页面。修复后配置
/// 文件唯一写入点在向导最后一步「完成」按钮（前端 save_config 全量落盘）；
/// 退出只保存已存在的配置（主窗口/设置页正常退出语义不变），首次运行向导
/// 中途退出不产生任何配置文件。
pub fn should_persist_on_shutdown(config_manager: &ConfigManager) -> bool {
    config_manager.global_config_path().exists()
}

/// 系统托盘管理器：持有 TrayIcon，负责菜单构建、动态更新、菜单事件分发。
///
/// 生命周期：setup 中创建并 `app.manage(Arc<TrayManager>)`（见 lib.rs）；
/// 运行中设置页切换 `show_tray` 时经 [`reconcile_tray`] 调整图标可见性。
pub struct TrayManager {
    app: AppHandle,
    tray: TrayIcon,
    /// 托盘图标是否可见（show_tray 设置；刷新循环据此跳过无用重建）
    enabled: Arc<AtomicBool>,
    /// 当前菜单数据（菜单事件分发需要最近文件路径；变化检测需要旧值）
    data: Arc<std::sync::Mutex<TrayMenuData>>,
}

impl TrayManager {
    /// 创建托盘图标 + 初始菜单。
    ///
    /// 图标使用**托盘专用图标** `icons/tray-icon.png`（透明底 + 黑底圆角方形 + 白色
    /// FM，16px 通知区缩放清晰；与窗口图标（同风格大尺寸）解耦，避免小尺寸糊化）。
    ///
    /// `visible`：show_tray 配置；false 时图标创建但隐藏（运行中可经
    /// [`TrayManager::set_enabled`] 随时显示，无需重建托盘）。
    pub fn new(app: &AppHandle, visible: bool) -> Result<Arc<Self>, String> {
        let icon = tauri::image::Image::from_bytes(include_bytes!("../../../icons/tray-icon.png"))
            .map_err(|e| format!("加载托盘图标失败: {}", e))?;
        let data = Arc::new(std::sync::Mutex::new(TrayMenuData::default()));
        // 锁中毒统一优雅降级（与 apply / open_recent_file 一致），setup 期不 panic
        let menu = build_menu(app, &data.lock().unwrap_or_else(|e| e.into_inner()))?;

        let data_for_events = data.clone();
        let app_for_icon_events = app.clone();
        let tray = TrayIconBuilder::with_id("missevan-recorder-tray")
            .icon(icon)
            .tooltip("Missevan 猫耳录制器")
            .menu(&menu)
            .show_menu_on_left_click(false)
            .on_menu_event(move |app, event| handle_menu_event(app, &data_for_events, event))
            // 规格 1.1（可选）：左键单击托盘图标 → 显示主窗口
            .on_tray_icon_event(move |_tray, event| {
                if let tauri::tray::TrayIconEvent::Click {
                    button: tauri::tray::MouseButton::Left,
                    ..
                } = event
                {
                    show_main_window(&app_for_icon_events);
                }
            })
            .build(app)
            .map_err(|e| format!("创建托盘失败: {}", e))?;
        // 启动双重 NIM_ADD 修复（tray-icon 0.24.1 vendored 源码核验）：
        // `TrayIcon::new` 即 register_tray_icon（NIM_ADD，userdata.visible 初始
        // true）；随后 `set_visible(true)` 经 WM_USER_SHOW_TRAYICON **无条件再次
        // NIM_ADD**（无 visible 状态守卫）——启动时双重注册是「偶尔出现多个
        // 图标」的候选根因之一（另一候选 = 崩溃残留幽灵图标，见 clean_exit
        // 标记）。修复：visible=true 时跳过 set_visible（build 时已可见）；
        // 仅 visible=false 才 set_visible(false)（NIM_DELETE 隐藏，运行中可经
        // set_enabled 随时显示，无需重建托盘）
        if !visible {
            tray.set_visible(false)
                .map_err(|e| format!("设置托盘图标可见性失败: {}", e))?;
        }

        Ok(Arc::new(Self {
            app: app.clone(),
            tray,
            enabled: Arc::new(AtomicBool::new(visible)),
            data,
        }))
    }

    /// 托盘图标是否可见
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// 显示/隐藏托盘图标（show_tray 设置开关；幂等）
    pub fn set_enabled(&self, visible: bool) {
        let changed = self.enabled.swap(visible, Ordering::SeqCst) != visible;
        if let Err(e) = self.tray.set_visible(visible) {
            tracing::error!("设置托盘图标可见性失败: {}", e);
        } else if changed {
            tracing::info!("托盘图标可见性: {}", visible);
        }
    }

    /// 后台轮询：每 2s 读取录制计数与最近文件，数据变化时重建菜单。
    /// 循环常驻（可见性切换无需重启轮询）；不可见时跳过无谓重建。
    pub fn spawn_refresher(self: &Arc<Self>, app_state: AppStateHandle) {
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            tracing::info!("托盘菜单轮询已启动（2s 间隔）");
            loop {
                tokio::time::sleep(REFRESH_INTERVAL).await;
                this.refresh(&app_state).await;
            }
        });
    }

    /// 读共享状态 → 变化检测 → 重建菜单
    async fn refresh(&self, app_state: &AppStateHandle) {
        if !self.enabled() {
            return;
        }
        let snapshot = {
            let state = app_state.lock().await;
            TrayMenuData {
                recording_count: state.active_count(),
                recent_files: recent_files_from_history(state.history.iter(), RECENT_LIMIT),
            }
        };
        self.apply(snapshot);
    }

    /// 数据变化才重建菜单（set_menu）
    fn apply(&self, new: TrayMenuData) {
        if !self.enabled() {
            return;
        }
        let mut guard = self.data.lock().unwrap_or_else(|e| e.into_inner());
        if *guard == new {
            return;
        }
        *guard = new.clone();
        drop(guard);
        match build_menu(&self.app, &new) {
            Ok(menu) => {
                if let Err(e) = self.tray.set_menu(Some(menu)) {
                    tracing::error!("更新托盘菜单失败: {}", e);
                }
            }
            Err(e) => tracing::error!("重建托盘菜单失败: {}", e),
        }
    }
}

/// 同步托盘图标可见性（save_config / import_config 后调用；幂等）。
///
/// 简化后（修复子代理 B）：可见性由 `close_behavior` 派生——调用方传
/// `should_hide_to_tray(&config.close_behavior)`（"tray" → 显示 / "exit" → 隐藏）。
pub fn reconcile_tray(app: &AppHandle, visible: bool) {
    if let Some(manager) = app.try_state::<Arc<TrayManager>>() {
        manager.set_enabled(visible);
    }
}

/// ── 上次退出干净度标记（托盘幽灵图标缓解，Windows）──
///
/// Windows shell 对**死进程**的通知区图标不主动清理（悬停通知区或 explorer
/// 重启才移除）——应用被强杀/崩溃后，旧图标残留并与新实例图标并存，表现为
/// 「偶尔出现多个图标」（悬停后旧图标消失，是幽灵图标特征）。代码层无法
/// 删除死进程的图标，只能缓解：
/// - 启动时检查 `{exe_dir}/.clean_exit`：存在 = 上次启动后未走统一优雅退出
///   （崩溃/强杀），提示用户「悬停通知区可清除残留图标」；
/// - 正常退出（[`request_shutdown`] → [`do_shutdown`]，全应用唯一退出路径：
///   托盘「退出」/ 主窗关闭 / 向导「退出」）时删除该标记。
///
/// 标记写入/删除均为尽力而为（失败仅记 warning）：安装目录只读（如 Program
/// Files 但非管理员）时检测功能优雅降级为不可用，不影响其余功能。
/// 标记文件放 `{exe_dir}` 与 config 目录同侧（ConfigManager 已在该目录写配置，
/// 不存在额外的权限假设）。
const CLEAN_EXIT_MARKER: &str = ".clean_exit";

/// 上次退出是否干净：true = 上次启动后未移除标记（崩溃/强杀）。
///
/// 注意：本函数**不删除**标记——标记在本次运行期持续存在，直到正常退出时
/// 移除；因此本次运行若再异常退出，下次启动仍能检测到。调用方（lib.rs setup）
/// 随后应 [`write_clean_exit_marker`] 确保标记在位。
pub fn clean_exit_marker_exists() -> bool {
    let path = crate::domain::tools::exe_dir().join(CLEAN_EXIT_MARKER);
    if path.exists() {
        tracing::warn!(
            "检测到上次异常退出（标记文件 {} 存在）：若托盘中残留旧图标，可鼠标悬停通知区清除",
            path.display()
        );
        true
    } else {
        false
    }
}

/// 启动时写入「本次运行尚未正常结束」标记（与 [`clean_exit_marker_exists`] 配套；
/// 幂等）。尽力而为，失败仅记日志。
pub fn write_clean_exit_marker() {
    let path = crate::domain::tools::exe_dir().join(CLEAN_EXIT_MARKER);
    if let Err(e) = std::fs::write(&path, b"") {
        tracing::warn!("写入退出标记失败（异常退出检测将不可用）: {}", e);
    }
}

/// 正常退出时移除「未结束」标记（[`do_shutdown`] 在 `app.exit(0)` 前调用）。
/// 尽力而为：文件不存在 / 删除失败均只记日志，绝不阻塞退出。
pub fn remove_clean_exit_marker() {
    let path = crate::domain::tools::exe_dir().join(CLEAN_EXIT_MARKER);
    if let Err(e) = std::fs::remove_file(&path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!("移除退出标记失败: {}", e);
        }
    }
}

/// 优雅退出入口（托盘「退出」、主窗关闭 close_behavior≠tray、向导「退出」共用）。
///
/// 流程：保存配置 → `shutdown_notify.notify_waiters()`（检测循环收到后立即停止，
/// 不再启动新录制）→ `global_cancel.cancel()` + 逐个 cancel 录制任务
/// → 等待 JoinHandle（统一 ≤5s 超时）→ `app.exit(0)`。
pub fn request_shutdown(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        do_shutdown(&app).await;
    });
}

async fn do_shutdown(app: &AppHandle) {
    tracing::info!("开始优雅退出");
    // 1. 保存配置（幂等；失败仅记日志，不阻塞退出）
    //    根因修复（修复子代理 B）：仅当配置文件**已存在**时才保存——首次运行
    //    向导中途退出（配置尚未产生）绝不落盘，避免「第 1 步退出也产生
    //    config.toml（默认 wizard_completed=true）→ 下次启动绕过向导」。
    if let Some(config_manager) = app.try_state::<Arc<ConfigManager>>() {
        if !should_persist_on_shutdown(&**config_manager) {
            tracing::debug!("配置不存在（首次运行向导未完成），退出不保存配置");
        } else {
            match config_manager.load() {
                Ok(config) => {
                    if let Err(e) = config_manager.save_global(&config.global) {
                        tracing::error!("退出前保存配置失败: {}", e);
                    }
                }
                Err(e) => tracing::warn!("退出前读取配置失败: {}", e),
            }
        }
    }

    // 2. 通知检测循环停止 + 取消所有录制任务
    let mut tasks_to_wait: Vec<Task> = Vec::new();
    if let Some(recorder_state) = app.try_state::<RecorderState>() {
        recorder_state.shutdown_notify.notify_waiters();
        let mut state = recorder_state.state.lock().await;
        state.global_cancel.cancel();
        tasks_to_wait.extend(state.tasks.drain().map(|(_, task)| task));
        drop(state);
        for task in &tasks_to_wait {
            task.cancel_token.cancel();
        }
    }

    // 3. 等待录制任务结束（统一 ≤5s 超时；超时则放弃等待直接退出）
    if !tasks_to_wait.is_empty() {
        tracing::info!("等待 {} 个录制任务结束（≤5s）", tasks_to_wait.len());
        let deadline = tokio::time::Instant::now() + SHUTDOWN_WAIT;
        for task in tasks_to_wait {
            let _ = tokio::time::timeout_at(deadline, task.handle).await;
        }
    }

    // 3.5 剩余录制进程强制终止兜底（B2）：上一步等待超时后，进程表可能仍有
    // 存活条目（ffmpeg 网络 IO 卡死等）。显式逐个走带超时的强制终止，确保
    // 退出后无孤儿 ffmpeg 进程（Linux 上孤儿进程持续占用录音文件句柄；Windows
    // 上遗留未收割句柄）。kill_on_drop 在此不可依赖：tauri 的全局异步运行时
    // 随进程退出直接终止，任务 future 不保证 drop——必须显式终止。
    if let Some(recorder) = app.try_state::<Arc<FfmpegRecorder>>() {
        let alive = recorder.active_anchor_ids();
        if !alive.is_empty() {
            tracing::info!("退出前强制终止 {} 个剩余录制进程", alive.len());
            recorder.force_terminate_all().await;
        }
    }

    tracing::info!("优雅退出完成");
    // 上次退出干净度标记：统一退出路径在此移除（崩溃/强杀时标记残留，下次
    // 启动据此提示用户清理托盘幽灵图标——见 clean_exit_marker_exists 注释）。
    // 应用退出后 TrayManager 由托管状态 drop → TrayIcon drop → NIM_DELETE，
    // 无需显式 drop（tray-icon Drop 实现 remove_tray_icon，vendored 源码核验）
    #[cfg(windows)]
    remove_clean_exit_marker();
    app.exit(0);
}

/// 构建托盘右键菜单（当前快照 → 全新 Menu）
///
/// ```text
/// 显示主窗口
/// ─────────
/// 录制中：N            (N=0 时禁用)
/// 最近录制 ▸           (子菜单，最多 5 条；空时显示禁用的「暂无最近录制」)
/// ─────────
/// 退出应用
/// ```
fn build_menu(app: &AppHandle, data: &TrayMenuData) -> Result<Menu<Wry>, String> {
    let show_main = MenuItem::with_id(app, MENU_SHOW_MAIN, "显示主窗口", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let sep1 = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;

    let recording = MenuItem::with_id(
        app,
        MENU_RECORDING,
        format!("录制中：{}", data.recording_count),
        data.recording_count > 0,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;

    let recent_submenu = build_recent_submenu(app, data)?;

    let sep2 = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
    let exit = MenuItem::with_id(app, MENU_EXIT, "退出应用", true, None::<&str>)
        .map_err(|e| e.to_string())?;

    let items: Vec<&dyn IsMenuItem<Wry>> =
        vec![&show_main, &sep1, &recording, &recent_submenu, &sep2, &exit];
    Menu::with_items(app, &items).map_err(|e| e.to_string())
}

/// 「最近录制」子菜单：最多 5 条（点击在资源管理器选中该文件）；空时显示禁用的占位项
fn build_recent_submenu(app: &AppHandle, data: &TrayMenuData) -> Result<Submenu<Wry>, String> {
    if data.recent_files.is_empty() {
        let placeholder =
            MenuItem::with_id(app, MENU_RECENT_EMPTY, "暂无最近录制", false, None::<&str>)
                .map_err(|e| e.to_string())?;
        return Submenu::with_items(app, "最近录制", true, &[&placeholder])
            .map_err(|e| e.to_string());
    }
    let items_owned: Vec<MenuItem<Wry>> = data
        .recent_files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            MenuItem::with_id(
                app,
                format!("{}{}", MENU_RECENT_PREFIX, i),
                f.label.clone(),
                true,
                None::<&str>,
            )
            .map_err(|e| e.to_string())
        })
        .collect::<Result<_, _>>()?;
    let items: Vec<&dyn IsMenuItem<Wry>> =
        items_owned.iter().map(|i| i as &dyn IsMenuItem<Wry>).collect();
    Submenu::with_items(app, "最近录制", true, &items).map_err(|e| e.to_string())
}

/// 菜单事件分发（on_menu_event 回调；主线程调用，禁止阻塞操作）
fn handle_menu_event(app: &AppHandle, data: &std::sync::Mutex<TrayMenuData>, event: MenuEvent) {
    let id = event.id().as_ref();
    match id {
        MENU_SHOW_MAIN => show_main_window(app),
        MENU_RECORDING => {
            // 规格 1.1：「点击可打开直播页面」——显示主窗 + emit 事件，
            // 前端已接线：App.vue 监听 `tray:open_live_page` 并导航到直播页（Task 20）
            show_main_window(app);
            let _ = app.emit("tray:open_live_page", ());
        }
        MENU_EXIT => request_shutdown(app),
        _ => {
            if let Some(index) = recent_index_from_menu_id(id) {
                open_recent_file(app, data, index);
            }
        }
    }
}

/// 显示并聚焦主窗口（取消最小化，避免托盘恢复后窗口仍是最小化状态）
fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// 打开最近录制文件所在文件夹（opener 插件 `reveal_item_in_dir`：
/// Windows 资源管理器中定位选中该文件，Linux 打开其所在目录）
fn open_recent_file(app: &AppHandle, data: &std::sync::Mutex<TrayMenuData>, index: usize) {
    let path = data
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .recent_files
        .get(index)
        .map(|f| f.path.clone());
    if let Some(path) = path {
        // tauri_plugin_opener::Opener::reveal_item_in_dir(p: impl AsRef<Path>)
        if let Err(e) = app.opener().reveal_item_in_dir(Path::new(&path)) {
            tracing::warn!("打开最近录制所在文件夹失败（{}）: {}", path, e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(path: &str) -> RecordingSummary {
        RecordingSummary {
            anchor_id: "a1".into(),
            anchor_name: "主播A".into(),
            room_id: "1".into(),
            output_path: path.into(),
            started_at: "2026-08-01T00:00:00Z".into(),
            duration_secs: 10,
            ended_at: "2026-08-01T01:00:00Z".into(),
        }
    }

    #[test]
    fn recent_files_take_latest_five() {
        let history: Vec<RecordingSummary> =
            (0..7).map(|i| summary(&format!("D:/rec/{}.m4a", i))).collect();
        let recent = recent_files_from_history(history.iter(), RECENT_LIMIT);
        assert_eq!(recent.len(), 5);
        // 历史最新在前，取前 5 条（路径 = 菜单打开目标）
        assert_eq!(recent[0].path, "D:/rec/0.m4a");
        assert_eq!(recent[4].path, "D:/rec/4.m4a");
    }

    #[test]
    fn recent_files_empty_history_yields_empty() {
        assert!(recent_files_from_history(std::iter::empty(), RECENT_LIMIT).is_empty());
    }

    #[test]
    fn recent_file_label_uses_basename() {
        let recent =
            recent_files_from_history(std::iter::once(&summary("D:/rec/主播A_20260801.m4a")), 5);
        assert_eq!(recent[0].label, "主播A_20260801.m4a");
    }

    #[test]
    fn recent_file_label_falls_back_to_full_path() {
        // 根路径无文件名分量（file_name() == None）时退回完整路径
        let recent = recent_files_from_history(std::iter::once(&summary("C:\\")), 5);
        assert_eq!(recent[0].label, "C:\\");
    }

    #[cfg(windows)]
    #[test]
    fn recent_file_label_escapes_ampersand() {
        // muda 将 & 视为 Windows 助记符前缀，须转义为 &&
        let recent =
            recent_files_from_history(std::iter::once(&summary("D:/rec/a&b_001.m4a")), 5);
        assert_eq!(recent[0].label, "a&&b_001.m4a");
    }

    #[cfg(not(windows))]
    #[test]
    fn recent_file_label_keeps_ampersand_literal() {
        // Linux/GTK 菜单 `&` 无助记符语义，转义反而显示 `&&` → 原样保留
        let recent =
            recent_files_from_history(std::iter::once(&summary("D:/rec/a&b_001.m4a")), 5);
        assert_eq!(recent[0].label, "a&b_001.m4a");
    }

    #[test]
    fn truncated_label_shortens_long_names() {
        let long = "很长的文件名".repeat(20); // 120 字符
        let t = truncated_label(&long, 40);
        assert!(t.chars().count() <= 40);
        assert!(t.ends_with("..."));
        // 未超限原样返回
        assert_eq!(truncated_label("短名.m4a", 40), "短名.m4a");
        // 边界：恰好 max_chars 不截断
        let exact = "a".repeat(40);
        assert_eq!(truncated_label(&exact, 40), exact);
        // 超短上限（1 字符）不 panic
        assert_eq!(truncated_label("abcd", 1), "...");
    }

    #[test]
    fn recent_index_from_menu_id_parses() {
        assert_eq!(recent_index_from_menu_id("tray-recent-0"), Some(0));
        assert_eq!(recent_index_from_menu_id("tray-recent-4"), Some(4));
        assert_eq!(recent_index_from_menu_id("tray-recent-"), None);
        assert_eq!(recent_index_from_menu_id("tray-recent-abc"), None);
        assert_eq!(recent_index_from_menu_id("tray-show-main"), None);
    }

    #[test]
    fn close_behavior_matrix() {
        // tray → 最小化到托盘（close_behavior 单一决定；show_tray 已并入派生逻辑）
        assert!(should_hide_to_tray("tray"));
        // exit → 直接退出
        assert!(!should_hide_to_tray("exit"));
        // 未知值按退出保守处理
        assert!(!should_hide_to_tray("anything"));
    }

    #[test]
    fn close_action_requires_live_enabled_tray() {
        use CloseAction::{Exit, HideToTray};
        // 托盘缺失（创建失败）：任何配置都退出，绝不隐藏窗口
        assert_eq!(decide_close_action("tray", None), Exit);
        assert_eq!(decide_close_action("exit", None), Exit);
        // 托盘存在但禁用（图标已隐藏）：退出
        assert_eq!(decide_close_action("tray", Some(false)), Exit);
        assert_eq!(decide_close_action("exit", Some(false)), Exit);
        // 托盘存在且启用：仅 close_behavior=tray 隐藏
        assert_eq!(decide_close_action("tray", Some(true)), HideToTray);
        assert_eq!(decide_close_action("exit", Some(true)), Exit);
    }

    #[test]
    fn should_persist_on_shutdown_only_when_config_exists() {
        use crate::domain::config::model::GlobalConfig;
        // 首次运行（无配置文件）：退出不落盘（中途退出不产生 config.toml）
        let dir = std::env::temp_dir().join(format!(
            "missevan-tray-shutdown-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let manager = ConfigManager::new(dir.clone());
        assert!(!should_persist_on_shutdown(&manager));
        // 配置写入后：退出保存（主窗口/设置页正常退出语义不变）
        manager.save_global(&GlobalConfig::default()).unwrap();
        assert!(should_persist_on_shutdown(&manager));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tray_visibility_derived_from_close_behavior() {
        // 托盘可见性派生 = should_hide_to_tray（reconcile_tray / 启动创建托盘共用）：
        // close_behavior=tray → 图标可见（隐藏窗口后可恢复）；exit → 隐藏
        assert!(should_hide_to_tray("tray"));
        assert!(!should_hide_to_tray("exit"));
    }

    #[test]
    fn clean_exit_marker_roundtrip() {
        // 写入 → 存在；移除 → 不存在；重复移除幂等。
        // 路径 = exe_dir（测试二进制所在目录 target/debug/deps），测试结束已清理
        remove_clean_exit_marker();
        assert!(!clean_exit_marker_exists(), "干净状态不应报异常退出");
        write_clean_exit_marker();
        assert!(clean_exit_marker_exists(), "标记写入后应检测到上次未正常退出");
        remove_clean_exit_marker();
        assert!(!clean_exit_marker_exists(), "移除后应恢复干净状态");
        remove_clean_exit_marker(); // 重复移除（NotFound）不 panic
    }

    #[test]
    fn menu_data_change_detection() {
        let a = TrayMenuData {
            recording_count: 1,
            recent_files: vec![RecentFile {
                label: "a.m4a".into(),
                path: "D:/rec/a.m4a".into(),
            }],
        };
        let same = TrayMenuData {
            recording_count: 1,
            recent_files: vec![RecentFile {
                label: "a.m4a".into(),
                path: "D:/rec/a.m4a".into(),
            }],
        };
        let diff_count = TrayMenuData {
            recording_count: 2,
            recent_files: a.recent_files.clone(),
        };
        let diff_files = TrayMenuData {
            recording_count: 1,
            recent_files: Vec::new(),
        };
        assert_eq!(a, same); // 无变化 → 不重建菜单
        assert_ne!(a, diff_count); // 录制数变化 → 重建
        assert_ne!(a, diff_files); // 最近文件变化 → 重建
    }
}
