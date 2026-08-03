use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 全局配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GlobalConfig {
    pub output_dir: String,
    pub record_format: String, // "m4a" | "mp3"
    pub segment_seconds: u64,  // 0 = 不分割
    pub disk_space_limit_gb: u64,
    pub ffmpeg_path: Option<String>,
    pub anchor_ids: Vec<String>,  // 启用的主播 ID 列表
    pub check_interval_secs: u64, // 检测间隔（默认 120，规格 §7.8）
    pub max_retries: u32,         // 录制重试次数（默认 3）
    pub retry_delay_secs: u64,    // 重试间隔（默认 5）
    // —— 通用（§11.1）——
    pub autostart: bool,
    pub close_behavior: String, // "tray" | "exit"
    pub show_tray: bool,
    pub check_updates: bool,
    // —— 录制（§11.1）——
    pub bitrate_kbps: u32,
    pub audio_only: bool,
    pub filename_template: String,
    pub max_concurrent_recordings: u32,
    pub pre_record_delay_secs: u32,
    pub post_record_action: String, // none | open_folder | command
    pub post_record_command: String,
    // —— 文件（§11.1）——
    pub auto_cleanup_enabled: bool,
    pub retention_days: u32,
    pub max_total_gb: u32, // 0 = 不限制总大小
    pub cleanup_time: String,
    // —— 网络（§11.1）——
    pub proxy_type: String, // none | http | socks5
    pub proxy_addr: String,
    pub proxy_port: u16,
    pub proxy_auth: bool,
    pub proxy_username: String,
    pub proxy_password: String, // 加密存储
    pub api_timeout_secs: u32,
    pub stream_timeout_secs: u32,
    pub custom_dns: String, // 空 = 系统 DNS
    // —— 通知（§11.1）——
    pub notifications_enabled: bool,
    pub notify_recording_start: bool,
    pub notify_recording_end: bool,
    pub notify_recording_error: bool,
    pub notify_live_start: bool,
    pub notify_live_end: bool,
    pub notify_disk_warning: bool,
    pub notify_update: bool,
    pub notify_system: bool,
    pub notify_sound: bool,
    // —— 高级（§11.1）——
    pub log_level: String,
    pub detector_concurrency: u32,
    pub ffprobe_path: String, // 空 = 自动探测
    /// 检测随机抖动上限（秒，0 = 不抖动；Task 14 接线 detector/loop.rs）
    pub detector_jitter_secs: u32,
    // —— 快捷键（§11.2 set_shortcut；前端当前用 localStorage 展示，后端命令可选接线）——
    pub shortcuts: HashMap<String, String>, // id -> 组合键；空字符串值 = 未绑定
    // —— 引导完成标记（主 AGENT 引导逻辑分析修复）——
    /// 是否已完成引导。`is_first_run` 以此为准：config.toml 存在 ≠ 引导完成——
    /// 首次引导第 3 步环境检查通过即写盘（规格要求），若用户在写盘后、第 4 步
    /// 「进入应用」前退出，配置已存在但引导未完成，必须再次打开引导窗。
    /// **serde default = true**：老用户配置无此字段 → 视为已完成（无回归）；
    /// 首次引导写盘时前端显式传 false，finish_wizard 置 true。
    #[serde(default = "default_wizard_completed")]
    pub wizard_completed: bool,
}

fn default_wizard_completed() -> bool {
    true
}

/// 支持的录制格式白名单（M1 审查跟进：record_format 会被拼入输出文件扩展名，
/// 见 engine.rs 输出路径拼接——白名单拒绝 `../../` 等路径穿越注入）。
/// 只列后端真实支持的格式：builder.rs 编解码映射为 mp3→libmp3lame、其余→AAC
/// （m4a 容器）；前端（向导 + 设置页）也只提供 m4a/mp3 两个选项。
/// 不放开 flac/aac——会生成扩展名与容器不符的坏文件（如 .flac 里装 AAC）。
pub const RECORD_FORMAT_WHITELIST: [&str; 2] = ["m4a", "mp3"];

/// record_format 白名单校验（大小写敏感：前端与导出文件均为小写）
pub fn is_valid_record_format(format: &str) -> bool {
    RECORD_FORMAT_WHITELIST.contains(&format)
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            output_dir: String::from("./recordings"),
            record_format: String::from("m4a"),
            segment_seconds: 0,
            disk_space_limit_gb: 10,
            ffmpeg_path: None,
            anchor_ids: Vec::new(),
            check_interval_secs: 120,
            max_retries: 3,
            retry_delay_secs: 5,
            autostart: false,
            close_behavior: String::from("tray"),
            show_tray: true,
            check_updates: true,
            bitrate_kbps: 128,
            audio_only: true,
            filename_template: String::from("{anchor_name}/{date}_{time}_{anchor_name}.{ext}"),
            max_concurrent_recordings: 3,
            pre_record_delay_secs: 0,
            post_record_action: String::from("none"),
            post_record_command: String::new(),
            auto_cleanup_enabled: false,
            retention_days: 30,
            max_total_gb: 0,
            cleanup_time: String::from("03:00"),
            proxy_type: String::from("none"),
            proxy_addr: String::new(),
            proxy_port: 0,
            proxy_auth: false,
            proxy_username: String::new(),
            proxy_password: String::new(),
            api_timeout_secs: 10,
            stream_timeout_secs: 30,
            custom_dns: String::new(),
            notifications_enabled: true,
            notify_recording_start: true,
            notify_recording_end: true,
            notify_recording_error: true,
            notify_live_start: true,
            notify_live_end: true,
            notify_disk_warning: true,
            notify_update: true,
            notify_system: true,
            notify_sound: true,
            log_level: String::from("info"),
            detector_concurrency: 5,
            ffprobe_path: String::new(),
            detector_jitter_secs: 60,
            shortcuts: HashMap::new(),
            wizard_completed: true,
        }
    }
}

/// 主播配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnchorConfig {
    pub id: String,                 // UUID
    pub name: String,               // 主播名
    pub url: String,                // 猫耳 FM 直播间 URL
    pub room_id: String,            // 从 URL 提取的房间号
    pub proxy: Option<String>,      // 可选代理
    pub cookie: Option<String>,     // 可选 Cookie
    pub enable_check: bool,         // 是否启用检测
    pub avatar_url: Option<String>, //头像 不存配置
    /// 用户自定义标签（后端持久化落盘；serde default 兼容旧配置无 tags 字段）
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecordingStatus {
    pub anchor_id: String,
    pub is_recording: bool,
    pub is_live: bool,
}

//主播状态
#[derive(Debug, Clone, Serialize)]
pub struct AnchorStatusUpdate {
    pub anchor_id: String,
    pub is_live: bool,
    pub is_recording: bool,
}

/// 运行时完整配置 = 全局 + 主播列表
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub global: GlobalConfig,
    pub anchors: Vec<AnchorConfig>,
}

impl Config {
    pub fn is_valid(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        if self.global.output_dir.is_empty() {
            errors.push("输出目录不能为空".to_string());
        }
        if self.global.disk_space_limit_gb == 0 {
            errors.push("磁盘阈值必须大于 0".to_string());
        }
        if self.global.segment_seconds > 86400 {
            errors.push("分段秒数不能超过 86400".to_string());
        }
        if self.global.check_interval_secs < 5 {
            errors.push("检测间隔不能小于 5 秒".to_string());
        }
        if !is_valid_record_format(&self.global.record_format) {
            errors.push("录制格式不支持（仅支持 m4a / mp3）".to_string());
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            global: GlobalConfig::default(),
            anchors: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_config_without_new_fields_loads_with_defaults() {
        let old = r#"
output_dir = "./recordings"
record_format = "mp3"
segment_seconds = 0
"#;
        let cfg: GlobalConfig = toml::from_str(old).unwrap();
        assert_eq!(cfg.max_concurrent_recordings, 3);
        assert_eq!(cfg.close_behavior, "tray");
        assert_eq!(cfg.proxy_type, "none");
        // Task 14 新增字段：旧配置反序列化全部走 serde default
        assert_eq!(cfg.detector_jitter_secs, 60);
        assert!(cfg.shortcuts.is_empty());
    }

    #[test]
    fn default_matches_design_doc_11_1() {
        let cfg = GlobalConfig::default();
        // 通用
        assert!(!cfg.autostart);
        assert_eq!(cfg.close_behavior, "tray");
        assert!(cfg.show_tray);
        assert!(cfg.check_updates);
        // 录制
        assert_eq!(cfg.bitrate_kbps, 128);
        assert!(cfg.audio_only);
        assert_eq!(
            cfg.filename_template,
            "{anchor_name}/{date}_{time}_{anchor_name}.{ext}"
        );
        assert_eq!(cfg.max_concurrent_recordings, 3);
        assert_eq!(cfg.pre_record_delay_secs, 0);
        assert_eq!(cfg.post_record_action, "none");
        assert_eq!(cfg.post_record_command, "");
        // 文件
        assert!(!cfg.auto_cleanup_enabled);
        assert_eq!(cfg.retention_days, 30);
        assert_eq!(cfg.max_total_gb, 0);
        assert_eq!(cfg.cleanup_time, "03:00");
        // 网络
        assert_eq!(cfg.proxy_type, "none");
        assert_eq!(cfg.proxy_addr, "");
        assert_eq!(cfg.proxy_port, 0);
        assert!(!cfg.proxy_auth);
        assert_eq!(cfg.proxy_username, "");
        assert_eq!(cfg.proxy_password, "");
        assert_eq!(cfg.api_timeout_secs, 10);
        assert_eq!(cfg.stream_timeout_secs, 30);
        assert_eq!(cfg.custom_dns, "");
        // 通知
        assert!(cfg.notifications_enabled);
        assert!(cfg.notify_recording_start);
        assert!(cfg.notify_recording_end);
        assert!(cfg.notify_recording_error);
        assert!(cfg.notify_live_start);
        assert!(cfg.notify_live_end);
        assert!(cfg.notify_disk_warning);
        assert!(cfg.notify_update);
        assert!(cfg.notify_system);
        assert!(cfg.notify_sound);
        // 高级
        assert_eq!(cfg.log_level, "info");
        assert_eq!(cfg.detector_concurrency, 5);
        assert_eq!(cfg.ffprobe_path, "");
        assert_eq!(cfg.detector_jitter_secs, 60);
        assert!(cfg.shortcuts.is_empty());
    }

    #[test]
    fn check_interval_default_is_120_per_spec() {
        // 规格 §7.8 高级分类：检测间隔默认 120s（Task 8 遗留统一）
        let cfg = GlobalConfig::default();
        assert_eq!(cfg.check_interval_secs, 120);
    }

    #[test]
    fn shortcuts_roundtrip_via_toml() {
        let mut cfg = GlobalConfig::default();
        cfg.shortcuts
            .insert("toggle_recording".to_string(), "Ctrl+Alt+R".to_string());
        let s = toml::to_string(&cfg).unwrap();
        let back: GlobalConfig = toml::from_str(&s).unwrap();
        assert_eq!(
            back.shortcuts.get("toggle_recording").map(String::as_str),
            Some("Ctrl+Alt+R")
        );
    }

    // ── 主播 tags（Task A/3：后端落盘持久化）──

    fn sample_anchor() -> AnchorConfig {
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
        }
    }

    #[test]
    fn anchor_tags_toml_roundtrip() {
        let mut a = sample_anchor();
        a.tags = vec!["音乐".to_string(), "唱歌".to_string()];
        let s = toml::to_string(&a).unwrap();
        assert!(s.contains("tags = [\"音乐\", \"唱歌\"]"), "tags 应写入 toml: {}", s);
        let back: AnchorConfig = toml::from_str(&s).unwrap();
        assert_eq!(back.tags, vec!["音乐", "唱歌"]);
        assert_eq!(back.id, "a1");
    }

    #[test]
    fn old_anchor_config_without_tags_loads_with_empty_tags() {
        // 旧版主播 toml 无 tags 字段：serde default 兼容，不解析失败
        let old = r#"
id = "a1"
name = "主播A"
url = "https://fm.missevan.com/live/1"
room_id = "1"
enable_check = true
"#;
        let a: AnchorConfig = toml::from_str(old).unwrap();
        assert!(a.tags.is_empty(), "旧配置 tags 应默认为空数组");
    }

    #[test]
    fn empty_tags_roundtrip_via_toml() {
        let a = sample_anchor(); // tags 为空
        let s = toml::to_string(&a).unwrap();
        let back: AnchorConfig = toml::from_str(&s).unwrap();
        assert!(back.tags.is_empty());
    }

    // ── M1 审查跟进：record_format 白名单 ──

    #[test]
    fn record_format_whitelist_validates() {
        assert!(is_valid_record_format("m4a"));
        assert!(is_valid_record_format("mp3"));
        assert!(!is_valid_record_format("flac"));
        assert!(!is_valid_record_format("aac"));
        assert!(!is_valid_record_format("M4A")); // 大小写敏感（前端提交小写）
        assert!(!is_valid_record_format("../../evil"));
        assert!(!is_valid_record_format("m4a/../pwn"));
        assert!(!is_valid_record_format(""));
    }

    #[test]
    fn config_is_valid_rejects_bad_record_format() {
        let mut cfg = Config::default();
        assert!(cfg.is_valid().is_ok());
        cfg.global.record_format = "../../pwn".to_string();
        let errs = cfg.is_valid().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("录制格式")), "错误: {:?}", errs);
    }
}
