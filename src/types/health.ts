/**
 * 健康检查 DTO，与后端 `infrastructure/checker/report.rs` 序列化结果对齐。
 */
export type CheckStatus = "Passed" | "Failed" | "Warning" | "Skipped";

export interface CheckResult {
  check_name: string;
  status: CheckStatus;
  message: string;
  details: string | null;
  suggestion: string | null;
  duration_ms: number;
}

export interface DiagnosticReport {
  results: CheckResult[];
  total: number;
  passed: number;
  failed: number;
  warnings: number;
  timestamp: string;
}

export interface DiagnosticFullReport {
  health: DiagnosticReport;
  config_exists: boolean;
  config_valid: boolean;
  config_errors: string[];
}

/** `download:progress` 事件载荷（wizard_cmds.rs DownloadProgress） */
export interface DownloadProgress {
  percent: number;
  stage: "connecting" | "downloading" | "extracting" | "verifying" | "done";
}

/** `download_ffmpeg` 命令返回值（wizard_cmds.rs DownloadFfmpegResult） */
export interface DownloadFfmpegResult {
  /** 下载完成后重新触发的 FFmpeg 检查结果 */
  check: CheckResult;
  /** 下载的 ffmpeg.exe 绝对路径（null = 未下载/自动探测） */
  ffmpeg_path: string | null;
  /** 下载的 ffprobe.exe 绝对路径（空串 = 自动探测） */
  ffprobe_path: string;
}
