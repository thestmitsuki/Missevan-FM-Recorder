// ============================================================
// 猫耳FM录制器 — 全局类型定义
// ============================================================

// ---------- 主播配置 ----------
export interface AnchorConfig {
  id: string
  name: string
  url: string
  proxy: string | null
  cookie: string | null
  enable_check: boolean
  check_interval_secs: number
}

// ---------- 主播展示信息 ----------
export interface AnchorInfo {
  id: string
  name: string
  avatar: string | null
  is_live: boolean
  is_recording: boolean
  room_url: string
  title: string
  introduction: string
  config: AnchorConfig
}

// ---------- 录音文件 ----------
export interface AudioFileInfo {
  name: string
  path: string
  size: number
  modified: string
  folder: string
}

// ---------- 全局配置 ----------
export interface GlobalConfig {
  output_dir: string
  record_format: 'M4A' | 'MP3'
  segment_seconds: number
  disk_space_limit_gb: number
  ffmpeg_path: string | null
  anchors: AnchorConfig[]
}

// ---------- 通知 ----------
export type NotificationLevel = 'info' | 'warning' | 'error'

export interface NotificationItem {
  id: number
  message: string
  level: NotificationLevel
  duration: number
  visible: boolean
}

// ---------- 标签页 ----------
export type TabKey = 'live' | 'files' | 'settings'

export interface TabItem {
  key: TabKey
  icon: string
}
