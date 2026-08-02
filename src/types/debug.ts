/**
 * 调试页 DTO，与后端 Task 15/16 序列化结果逐字段对齐：
 * - debug_cmds.rs：DebugInfo / get_logs / get_network_logs / get_detector_stats /
 *   get_recorder_state / get_file_cache_state / get_mock_state / export_diagnostic_report
 * - infrastructure/logging/buffer.rs：LogEntry（`debug:log` 事件载荷）
 * - infrastructure/logging/network.rs：NetworkLog
 * - domain/detector/stats.rs：DetectorStatsSnapshot
 * - infrastructure/state/app_state.rs：ActiveRecording / RecordingSummary / RecorderStateInfo
 * - domain/services/file_cache.rs：FileCacheState / ScanLogEntry
 * - api/mock_cmds.rs：MockStatusChanged（`mock:status_changed` 事件载荷）
 */

/** FFmpeg / ffprobe 可执行文件状态（`get_debug_info.ffmpeg_status` / `ffprobe_status`） */
export interface ToolStatus {
  /** 是否找到可执行的工具（解析顺序：配置路径 → {exe_dir}/ffmpeg/ → PATH） */
  found: boolean;
  /** 实际解析到的路径（裸名走 PATH 时为工具名本身） */
  path: string;
  /** `-version` 首行版本信息；找到但取版本失败时为 null */
  version: string | null;
}

/** `get_debug_info` 返回值：概览模块（活跃录制数 / Mock 模式 / 版本 / 统计汇总） */
export interface DebugInfo {
  active_recordings: number;
  mock_mode: boolean;
  app_version: string;
  rust_version: string;
  tauri_version: string;
  os: string;
  detector_running: boolean;
  total_checks: number;
  success_checks: number;
  failed_checks: number;
  enabled_anchors: number;
  live_anchors: number;
  recording_anchors: number;
  file_count: number;
  ffmpeg_status: ToolStatus;
  ffprobe_status: ToolStatus;
}

/** 单条日志（`debug:log` 事件载荷；level 为小写 trace/debug/info/warn/error） */
export interface LogEntry {
  timestamp: string;
  level: string;
  /** tracing target（模块路径，如 domain::detector::loop） */
  module: string;
  /** 日志消息（后端已脱敏） */
  message: string;
}

/** 单条网络请求记录（get_network_logs 返回） */
export interface NetworkLog {
  timestamp: string;
  method: string;
  /** 请求 URL（后端已脱敏） */
  url: string;
  /** HTTP 状态码；0 = 请求失败 */
  status: number;
  duration_ms: number;
  /** 关联主播 room_id（非主播请求为 null） */
  anchor_id: string | null;
  /** 失败原因（后端已脱敏）；成功为 null */
  error: string | null;
}

/** 检测循环统计快照（get_detector_stats 返回） */
export interface DetectorStatsSnapshot {
  running: boolean;
  /** 上次检测开始时间（RFC3339）；从未检测 = null */
  last_check_at: string | null;
  total_checks: number;
  success_checks: number;
  failed_checks: number;
  /** 状态「未知」次数（5XX/429/网络错误/格式变化/冷却跳过；计入 failed_checks） */
  unknown_checks: number;
  enabled_anchors: number;
  live_anchors: number;
  recording_anchors: number;
}

/** 活跃录制任务（get_recorder_state.active 元素） */
export interface ActiveRecording {
  anchor_id: string;
  anchor_name: string;
  room_id: string;
  /** "recording" */
  status: string;
  /** 已录时长（秒） */
  duration_secs: number;
  output_path: string;
  pid: number | null;
}

/** 已结束录制摘要（get_recorder_state.history 元素） */
export interface RecordingSummary {
  anchor_id: string;
  anchor_name: string;
  room_id: string;
  output_path: string;
  started_at: string;
  duration_secs: number;
  ended_at: string;
}

/** 录制引擎状态快照（get_recorder_state 返回） */
export interface RecorderStateInfo {
  active: ActiveRecording[];
  history: RecordingSummary[];
}

/** 一次文件扫描/清除记录（get_file_cache_state.scan_log 元素） */
export interface ScanLogEntry {
  timestamp: string;
  /** "scan" | "clear" */
  kind: string;
  duration_ms: number;
  files_before: number;
  files_after: number;
  groups: number;
}

/** 文件缓存状态（get_file_cache_state 返回） */
export interface FileCacheState {
  last_scan_at: string | null;
  file_count: number;
  group_count: number;
  total_size_bytes: number;
  /** 扫描日志（最新在前，上限 20） */
  scan_log: ScanLogEntry[];
}

/** Mock 面板状态（get_mock_state 返回；`mock:status_changed` 事件载荷） */
export interface MockStatusChanged {
  enabled: boolean;
  count: number;
}
