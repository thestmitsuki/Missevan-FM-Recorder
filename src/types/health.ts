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
