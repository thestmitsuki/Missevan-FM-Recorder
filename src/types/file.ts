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
  group_prefix?: string;
  segment_index?: number;
  /** 是否正被录制引擎写入（后端对照活跃录制任务输出路径标记；前端禁删/禁重命名） */
  is_active?: boolean;
}

export interface FileGroup {
  prefix: string;
  files: RecordingFile[];
  total_size: number;
  total_duration: number;
}

/** 后端文件缓存变更事件载荷 */
export interface RecordingFilesPayload {
  files: RecordingFile[];
  groups: FileGroup[];
}

/** 将后端 SystemTime JSON 转为 JS Date（秒 + 纳秒） */
export function systemTimeToDate(t: SystemTimeJson): Date {
  return new Date(t.secs_since_epoch * 1000 + t.nanos_since_epoch / 1e6);
}
