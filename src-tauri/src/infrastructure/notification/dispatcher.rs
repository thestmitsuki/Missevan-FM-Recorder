use std::sync::{Arc, RwLock};
use tauri::Emitter;
use tokio::sync::Mutex;
use tracing;

use super::buffer::RingBuffer;
use super::types::{Notification, NotificationLevel};
use crate::tr;

const RING_BUFFER_CAPACITY: usize = 500;

/// 通知设置快照（与 GlobalConfig 通知字段同步，Task 18）：
/// `enabled` = notifications_enabled 总开关；`system`/`sound` = notify_system/notify_sound；
/// 其余 7 项为各事件类型勾选（规格 5「事件通知选择」）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifySettings {
    pub enabled: bool,
    pub system: bool,
    pub sound: bool,
    pub recording_start: bool,
    pub recording_end: bool,
    pub recording_error: bool,
    pub live_start: bool,
    pub live_end: bool,
    pub disk_warning: bool,
    pub update: bool,
}

impl Default for NotifySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            system: true,
            sound: true,
            recording_start: true,
            recording_end: true,
            recording_error: true,
            live_start: true,
            live_end: true,
            disk_warning: true,
            update: true,
        }
    }
}

impl NotifySettings {
    /// 从 GlobalConfig 通知字段映射（纯函数——可独立单测；
    /// 注意：测试中不要构造 NotificationDispatcher 本身，见 sync 测试注释）
    pub fn from_config(config: &crate::domain::config::model::GlobalConfig) -> Self {
        Self {
            enabled: config.notifications_enabled,
            system: config.notify_system,
            sound: config.notify_sound,
            recording_start: config.notify_recording_start,
            recording_end: config.notify_recording_end,
            recording_error: config.notify_recording_error,
            live_start: config.notify_live_start,
            live_end: config.notify_live_end,
            disk_warning: config.notify_disk_warning,
            update: config.notify_update,
        }
    }
}

/// 通知分发器——全应用唯一的通知通道。
/// 每个通知同时发送到：Tauri 事件 → 前端、环形缓冲区 → 调试面板、tracing → 日志文件；
/// 当全局通知开启、系统通知开启且事件类型勾选时，额外发送 OS 原生通知
/// （tauri-plugin-notification，Task 18）。
pub struct NotificationDispatcher {
    app_handle: Arc<Mutex<Option<tauri::AppHandle>>>,
    buffer: Arc<Mutex<RingBuffer>>,
    /// 系统通知开关与事件勾选（save_config 时经 sync_from_config 同步）
    settings: RwLock<NotifySettings>,
}

impl NotificationDispatcher {
    pub fn new() -> Self {
        Self {
            app_handle: Arc::new(Mutex::new(None)),
            buffer: Arc::new(Mutex::new(RingBuffer::new(RING_BUFFER_CAPACITY))),
            settings: RwLock::new(NotifySettings::default()),
        }
    }

    /// 从 GlobalConfig 同步通知设置（save_config 成功后由 ConfigManager 调用；
    /// 同步方法——只在内部短暂持锁，可安全从同步上下文调用）
    pub fn sync_from_config(&self, config: &crate::domain::config::model::GlobalConfig) {
        let mut s = self.settings.write().unwrap_or_else(|p| p.into_inner());
        *s = NotifySettings::from_config(config);
    }

    /// 在 setup() 中调用，注入 AppHandle
    pub async fn set_app_handle(&self, handle: tauri::AppHandle) {
        *self.app_handle.lock().await = Some(handle);
    }

    /// 分发通知到所有通道
    pub async fn dispatch(&self, notification: Notification) {
        // 1. 写入环形缓冲区
        self.buffer.lock().await.push(notification.clone());

        // 2. 通过 tracing 记录
        match notification.level {
            NotificationLevel::Critical | NotificationLevel::Error => {
                tracing::error!(
                    notification_id = %notification.id,
                    code = %notification.code,
                    source = %notification.source,
                    "{} — {}", notification.title, notification.message
                );
            }
            NotificationLevel::Warning => {
                tracing::warn!(
                    notification_id = %notification.id,
                    code = %notification.code,
                    source = %notification.source,
                    "{} — {}", notification.title, notification.message
                );
            }
            NotificationLevel::Info => {
                tracing::info!(
                    notification_id = %notification.id,
                    code = %notification.code,
                    source = %notification.source,
                    "{} — {}", notification.title, notification.message
                );
            }
        }

        // 3. 发送 Tauri 事件到前端
        if let Some(handle) = self.app_handle.lock().await.as_ref() {
            let _ = handle.emit("app:notification", &notification);
        }

        // 4. 系统原生通知（Task 18）：总开关 × 系统开关 × 事件类型勾选
        let settings = self.settings.read().unwrap_or_else(|p| p.into_inner()).clone();
        if should_send_system(&settings, &notification) {
            self.send_system_notification(&notification, settings.sound).await;
        }
    }

    /// 发送 OS 原生通知（Windows：自实现 WinRT toast；macOS/Linux：插件）。
    /// 注意：本方法仅在生产构建可达（测试构建不链接 dispatcher 代码——
    /// tauri 运行时代码入测试二进制会触发本机加载器问题，见测试注释）。
    ///
    /// Windows 链路（组 C/3「通知不要使用 POWERSHELL 而是应用注册通知」）：
    /// 插件 → notify-rust 在开发模式（未安装）下会把 AUMID 回退为
    /// PowerShell（Toast::POWERSHELL_APP_ID），toast 以 PowerShell 身份显示，
    /// 因此 Windows 上完全绕开插件，改用 tauri-winrt-notification 直接以本应用
    /// AUMID（com.missevan-recorder.app，启动时 ensure_aumid_registered 注册
    /// “开始菜单”快捷方式）发送 toast；提示音 = Sound::Default（系统默认提示音，
    /// 与 notify_sound 开关联动；注意插件 API 的 sound("default") 小写值在
    /// Windows 端解析失败会被当作静音处理——已弃用）。
    #[cfg(windows)]
    async fn send_system_notification(&self, n: &Notification, sound: bool) {
        use crate::infrastructure::notification::windows_toast;
        if let Err(e) = windows_toast::show_toast(&n.title, &n.message, sound) {
            tracing::warn!(
                "{}",
                tr!(
                    "log.system_notify_failed",
                    code = n.code,
                    err = e,
                    aumid = windows_toast::AUMID
                )
            );
        }
    }

    /// macOS/Linux：tauri-plugin-notification（原链路）。
    #[cfg(not(windows))]
    async fn send_system_notification(&self, n: &Notification, sound: bool) {
        use tauri_plugin_notification::NotificationExt;
        let Some(handle) = self.app_handle.lock().await.as_ref().cloned() else {
            return; // AppHandle 未注入（如测试环境）
        };
        let mut builder = handle.notification().builder().title(&n.title).body(&n.message);
        if sound {
            builder = builder.sound("default");
        }
        if let Err(e) = builder.show() {
            tracing::warn!(
                "{}",
                tr!("log.system_notify_failed_short", code = n.code, err = e)
            );
        }
    }

    /// 快速创建 Info 通知并分发
    pub async fn info(&self, code: impl Into<String>, title: impl Into<String>, message: impl Into<String>) {
        let n = Notification::new(code, NotificationLevel::Info, title, message);
        self.dispatch(n).await;
    }

    /// 快速创建 Error 通知并分发
    pub async fn error(&self, code: impl Into<String>, title: impl Into<String>, message: impl Into<String>) {
        let n = Notification::new(code, NotificationLevel::Error, title, message);
        self.dispatch(n).await;
    }

    /// 快速创建 Warning 通知并分发
    pub async fn warning(&self, code: impl Into<String>, title: impl Into<String>, message: impl Into<String>) {
        let n = Notification::new(code, NotificationLevel::Warning, title, message);
        self.dispatch(n).await;
    }

    /// 获取最近通知列表（供调试面板查询）
    pub async fn recent_notifications(&self) -> Vec<Notification> {
        self.buffer.lock().await.all()
    }
}

/// 系统通知事件类别（规格 5「事件通知选择」7 项）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyCategory {
    RecordingStart,
    RecordingEnd,
    RecordingError,
    LiveStart,
    LiveEnd,
    DiskWarning,
    Update,
}

/// 系统通知过滤矩阵：notifications_enabled ∧ notify_system ∧ 事件类型勾选
pub fn should_send_system(settings: &NotifySettings, n: &Notification) -> bool {
    if !settings.enabled || !settings.system {
        return false;
    }
    match category_for_code(&n.code) {
        Some(NotifyCategory::RecordingStart) => settings.recording_start,
        Some(NotifyCategory::RecordingEnd) => settings.recording_end,
        Some(NotifyCategory::RecordingError) => settings.recording_error,
        Some(NotifyCategory::LiveStart) => settings.live_start,
        Some(NotifyCategory::LiveEnd) => settings.live_end,
        Some(NotifyCategory::DiskWarning) => settings.disk_warning,
        Some(NotifyCategory::Update) => settings.update,
        None => false,
    }
}

/// 通知代码 → 系统通知类别。
pub fn category_for_code(code: &str) -> Option<NotifyCategory> {
    let c = code.to_ascii_uppercase();
    if c == "REC_START" || c.contains("RECORDING_START") {
        Some(NotifyCategory::RecordingStart)
    } else if c.starts_with("REC_END") || c.starts_with("REC_STOP") || c.contains("RECORDING_END") {
        Some(NotifyCategory::RecordingEnd)
    } else if c.starts_with("REC_") || c.contains("RECORDING_ERR") {
        Some(NotifyCategory::RecordingError)
    } else if c.contains("LIVE_START") || c.contains("LIVE_OPEN") {
        Some(NotifyCategory::LiveStart)
    } else if c.contains("LIVE_END") || c.contains("LIVE_CLOSE") || c.contains("LIVE_OFF") {
        Some(NotifyCategory::LiveEnd)
    } else if c.contains("DISK") {
        Some(NotifyCategory::DiskWarning)
    } else if c.contains("UPDATE") {
        Some(NotifyCategory::Update)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_maps_rec_codes() {
        assert_eq!(category_for_code("REC_START"), Some(NotifyCategory::RecordingStart));
        assert_eq!(category_for_code("REC_ENDED"), Some(NotifyCategory::RecordingEnd));
        assert_eq!(category_for_code("REC_STOP"), Some(NotifyCategory::RecordingEnd));
        assert_eq!(category_for_code("REC_TIMEOUT"), Some(NotifyCategory::RecordingError));
        assert_eq!(category_for_code("REC_API_ERR"), Some(NotifyCategory::RecordingError));
        assert_eq!(category_for_code("REC_API_FAILED"), Some(NotifyCategory::RecordingError));
    }

    #[test]
    fn category_maps_future_codes() {
        assert_eq!(category_for_code("LIVE_START"), Some(NotifyCategory::LiveStart));
        assert_eq!(category_for_code("LIVE_END"), Some(NotifyCategory::LiveEnd));
        assert_eq!(category_for_code("DISK_LOW"), Some(NotifyCategory::DiskWarning));
        assert_eq!(category_for_code("UPDATE_AVAILABLE"), Some(NotifyCategory::Update));
        assert_eq!(category_for_code("live_start"), Some(NotifyCategory::LiveStart));
        assert_eq!(category_for_code("rec_start"), Some(NotifyCategory::RecordingStart));
    }

    #[test]
    fn category_returns_none_for_app_internal_codes() {
        assert_eq!(category_for_code("config_save_ok"), None);
        assert_eq!(category_for_code("anchor_add_ok"), None);
        assert_eq!(category_for_code("config_recovered"), None);
        assert_eq!(category_for_code(""), None);
    }

    fn notify(code: &str) -> Notification {
        Notification::new(code, NotificationLevel::Info, "t", "m")
    }

    #[test]
    fn system_notification_matrix() {
        let s = NotifySettings::default();
        assert!(should_send_system(&s, &notify("REC_START")));
        assert!(should_send_system(&s, &notify("REC_ENDED")));
        assert!(should_send_system(&s, &notify("REC_API_FAILED")));
        assert!(!should_send_system(&s, &notify("config_save_ok")));
    }

    #[test]
    fn system_notification_respects_switches() {
        let mut s = NotifySettings::default();
        s.enabled = false;
        assert!(!should_send_system(&s, &notify("REC_START")));
        let mut s = NotifySettings::default();
        s.system = false;
        assert!(!should_send_system(&s, &notify("REC_START")));
        let mut s = NotifySettings::default();
        s.recording_start = false;
        assert!(!should_send_system(&s, &notify("REC_START")));
        assert!(should_send_system(&s, &notify("REC_ENDED")));
    }

    #[test]
    fn notify_settings_from_config_maps_all_fields() {
        // 注意：不要在测试中构造 NotificationDispatcher——
        // 其 AppHandle 字段会迫使链接器把 tauri 运行时代码（user32/gdi32 等）
        // 链入测试二进制，在本机 rust-lld + Windows 组合下测试可执行文件
        // 无法加载（0xC0000139）。映射逻辑抽为纯函数 NotifySettings::from_config 单测。
        let mut cfg = crate::domain::config::model::GlobalConfig::default();
        cfg.notifications_enabled = false;
        cfg.notify_system = false;
        cfg.notify_sound = false;
        cfg.notify_recording_error = false;
        cfg.notify_live_start = false;
        cfg.notify_update = false;
        let s = NotifySettings::from_config(&cfg);
        assert!(!s.enabled);
        assert!(!s.system);
        assert!(!s.sound);
        assert!(!s.recording_error);
        assert!(!s.live_start);
        assert!(!s.update);
        // 未改字段保持默认
        assert!(s.recording_start);
        assert!(s.recording_end);
        assert!(s.live_end);
        assert!(s.disk_warning);
        // 映射结果接入过滤矩阵
        assert!(!should_send_system(&s, &notify("REC_API_FAILED")));
        assert!(!should_send_system(&s, &notify("REC_START")));
    }
}
