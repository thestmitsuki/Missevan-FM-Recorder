/**
 * 全局配置，与后端 `src-tauri/src/domain/config/model.rs` 的 GlobalConfig 对齐。
 * 后端 `#[serde(default)]` 保证缺省字段无损升级旧配置文件。
 *
 * ⚠️ 与后端逐字对齐：所有 key 为 snake_case，字符串枚举值与后端 TOML 完全一致
 * （close_behavior "tray"/"exit"、post_record_action "none"/"open_folder"/"command"、
 * proxy_type "none"/"http"/"socks5"、log_level 小写 "info"）。
 */

/** close_behavior 合法值（与后端枚举字符串一致） */
export type CloseBehavior = "tray" | "exit";
/** post_record_action 合法值（与后端枚举字符串一致） */
export type PostRecordAction = "none" | "open_folder" | "command";
/** proxy_type 合法值（与后端枚举字符串一致） */
export type ProxyType = "none" | "http" | "socks5";
/** record_format 合法值（后端仅支持 m4a/mp3） */
export type RecordFormat = "m4a" | "mp3";
/** log_level 合法值（小写，与后端 tracing Level 字符串一致） */
export type LogLevel = "error" | "warn" | "info" | "debug" | "trace";

export interface GlobalConfig {
  output_dir: string;
  record_format: string; // "m4a" | "mp3"
  segment_seconds: number; // 0 = 不分割
  disk_space_limit_gb: number;
  ffmpeg_path?: string | null;
  anchor_ids: string[];
  check_interval_secs: number;
  max_retries: number;
  retry_delay_secs: number;
  // —— 通用（§11.1）——
  autostart: boolean;
  close_behavior: string; // "tray" | "exit"
  show_tray: boolean;
  check_updates: boolean;
  // —— 录制（§11.1）——
  bitrate_kbps: number;
  audio_only: boolean;
  filename_template: string;
  max_concurrent_recordings: number;
  pre_record_delay_secs: number;
  post_record_action: string; // none | open_folder | command
  post_record_command: string;
  // —— 文件（§11.1）——
  auto_cleanup_enabled: boolean;
  retention_days: number;
  max_total_gb: number;
  /**
   * 已废弃：原「每日定时清理」时间（HH:MM）。定时调度已移除，自动清理改为
   * 每次录制结束时触发（后端 monitor.rs cleanup_on_recording_end）；字段保留
   * 仅为后端 serde 兼容旧配置文件，前端不再展示/使用。
   */
  cleanup_time: string;
  // —— 网络（§11.1）——
  proxy_type: string; // none | http | socks5
  proxy_addr: string;
  proxy_port: number;
  proxy_auth: boolean;
  proxy_username: string;
  proxy_password: string;
  api_timeout_secs: number;
  stream_timeout_secs: number;
  custom_dns: string;
  // —— 通知（§11.1）——
  notifications_enabled: boolean;
  notify_recording_start: boolean;
  notify_recording_end: boolean;
  notify_recording_error: boolean;
  notify_live_start: boolean;
  notify_live_end: boolean;
  notify_disk_warning: boolean;
  notify_update: boolean;
  notify_system: boolean;
  notify_sound: boolean;
  // —— 高级（§11.1）——
  log_level: string;
  detector_concurrency: number;
  ffprobe_path: string;
  /** 检测随机抖动上限（秒，0 = 不抖动；Task 14 后端接线，Task 20 前端表单迁移） */
  detector_jitter_secs: number;
  /** 快捷键映射（id → 组合键；由 ShortcutSection 经 set_shortcut 命令落盘，设置页保存不透传） */
  shortcuts: Record<string, string>;
  /** 引导完成标记（默认 true=已完成；首次向导写盘时显式 false，finish_wizard 置 true） */
  wizard_completed: boolean;
}

/** `import_config` 命令返回的导入汇总（与后端 manager.rs ImportSummary 对齐） */
export interface ImportSummary {
  mode: string;
  /** replace 模式下 global 是否已全替换 */
  global_replaced: boolean;
  anchors_added: number;
  anchors_removed: number;
  /** merge 模式下重复 id 跳过数（保留本地） */
  anchors_skipped: number;
  anchors_total: number;
}
