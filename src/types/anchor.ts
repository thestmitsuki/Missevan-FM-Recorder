/** 主播配置（与后端 AnchorConfig 对齐；tags 由后端持久化落盘，Task A/3） */
export interface AnchorConfig {
  id: string;
  name: string;
  url: string;
  room_id: string;
  proxy?: string | null;
  cookie?: string | null;
  enable_check: boolean;
  /** 头像 URL（后端动态获取，不落盘配置） */
  avatar_url?: string | null;
  /** 用户自定义标签（后端 toml 落盘持久化；兼容旧配置返回时兜底为空数组） */
  tags: string[];
}

/** 主播公开资料（get_anchor_profile 返回，供设置面板「主播简介」显示） */
export interface AnchorProfile {
  name: string;
  avatar_url: string;
  /** 主播简介（API 可能缺失） */
  introduction: string | null;
}

/** 录制状态（get_recording_status 返回） */
export interface RecordingStatus {
  anchor_id: string;
  is_recording: boolean;
  is_live: boolean;
}

/** 后端推送的主播状态更新（recording_status_changed 事件载荷） */
export interface AnchorStatusUpdate {
  anchor_id: string;
  is_live: boolean;
  is_recording: boolean;
}

/**
 * 模拟主播条目（与后端 MockStore::MockLiveData 对齐；Task 5 审查遗留：
 * 旧前端类型是 anchor_name/title，实际后端为 name/stream_url/local_file）
 */
export interface MockLiveData {
  room_id: string;
  /** 主播名称 */
  name: string;
  /** 是否直播中 */
  is_live: boolean;
  /** 流地址（默认 mock:// 占位；空串 = 故意无效地址，测试 FFmpeg 失败处理） */
  stream_url: string;
  /** 本地测试音视频文件路径（可选，供 FFmpeg 正常录制 mock 流） */
  local_file: string | null;
}
