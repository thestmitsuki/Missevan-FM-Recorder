/**
 * 文件缓存相关类型 —— 与后端 `src-tauri/src/domain/services/file_cache.rs` 对齐。
 *
 * 注意：后端 `created_at` 为 Rust `std::time::SystemTime`，serde 默认序列化为
 * `{ secs_since_epoch, nanos_since_epoch }`（见 serde core/ser/impls.rs），
 * 并非 ISO 字符串 —— 使用 `systemTimeToDate` 统一转换。
 */

/** Rust `std::time::SystemTime` 经 serde 默认序列化后的 JSON 形态 */
export interface SystemTimeJson {
  secs_since_epoch: number;
  nanos_since_epoch: number;
}

export interface RecordingFile {
  id: string;
  name: string;
  path: string;
  size: number;
  duration: number;
  anchor_name: string;
  created_at: SystemTimeJson;
  /** 是否正被录制引擎写入（后端对照活跃录制任务输出路径标记；前端禁删/禁重命名） */
  is_active?: boolean;
}

/** 文件夹树节点（后端 get_recording_files / recording_files_changed 顶层结构） */
export interface FileFolder {
  /** 主播名（anchor_name，已剥离 -房间号；输出目录根下文件为空字符串） */
  name: string;
  /** 磁盘文件夹路径（文件夹身份键） */
  path: string;
  /** 主播文件夹内全部音频文件（不再有分段组，全部平铺） */
  files: RecordingFile[];
}

/** 后端文件缓存变更事件载荷（文件夹树：录制目录 → 主播文件夹 → 音频文件） */
export interface RecordingFilesPayload {
  folders: FileFolder[];
}

/** 清理结果摘要（run_cleanup_now 返回；与后端 CleanupSummary 对齐） */
export interface CleanupSummary {
  /** 实际删除的文件数 */
  files_deleted: number;
  /** 释放的字节数 */
  bytes_freed: number;
  /** 清理后剩余文件数 */
  files_remaining: number;
  /** 清理后剩余字节数 */
  bytes_remaining: number;
}

/** 将后端 SystemTime JSON 转为 JS Date（秒 + 纳秒） */
export function systemTimeToDate(t: SystemTimeJson): Date {
  return new Date(t.secs_since_epoch * 1000 + t.nanos_since_epoch / 1e6);
}
