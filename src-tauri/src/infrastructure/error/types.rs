use serde::Serialize;

// ── 错误码 ──
/// 配置类错误
pub const CF_PARSE_FAIL: &str = "CF_PARSE_FAIL";
pub const CF_INVALID_FIELD: &str = "CF_INVALID_FIELD";
/// 系统类错误
pub const NF_FFMPEG_NOT_FOUND: &str = "NF_FFMPEG_NOT_FOUND";
pub const NF_FFMPEG_EXEC_FAIL: &str = "NF_FFMPEG_EXEC_FAIL";
/// 网络类错误
pub const NW_API_UNREACHABLE: &str = "NW_API_UNREACHABLE";
pub const NW_API_RESPONSE_ERR: &str = "NW_API_RESPONSE_ERR";
/// 录制类错误
pub const RC_PROCESS_CRASH: &str = "RC_PROCESS_CRASH";
pub const RC_STREAM_UNAVAILABLE: &str = "RC_STREAM_UNAVAILABLE";
/// 双录防御：同主播已有录制任务/进程在运行，拒绝重复启动
pub const RC_ALREADY_RECORDING: &str = "RC_ALREADY_RECORDING";
/// 并发录制上限（max_concurrent_recordings ≥ 1 时，活跃任务数达上限拒绝新录制）
pub const RC_CONCURRENCY_LIMIT: &str = "RC_CONCURRENCY_LIMIT";
/// 磁盘空间低于阈值（disk_space_limit_gb），拒绝启动录制（S2a 预检查）
pub const RC_DISK_LOW: &str = "RC_DISK_LOW";
/// IO 类错误
pub const IO_DISK_FULL: &str = "IO_DISK_FULL";
pub const IO_WRITE_FAIL: &str = "IO_WRITE_FAIL";
/// 内部类错误
pub const INT_STATE_CORRUPT: &str = "INT_STATE_CORRUPT";
pub const INT_UNEXPECTED: &str = "INT_UNEXPECTED";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ErrorCategory {
    Config,
    Network,
    System,
    Recording,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ErrorSeverity {
    Fatal,
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppError {
    pub code: &'static str,
    pub category: ErrorCategory,
    pub severity: ErrorSeverity,
    pub message: String,
    pub technical: Option<String>,
    pub suggestion: Option<String>,
    pub source: Option<String>,
}

impl AppError {
    pub fn config(message: impl Into<String>) -> Self {
        Self {
            code: CF_INVALID_FIELD,
            category: ErrorCategory::Config,
            severity: ErrorSeverity::Error,
            message: message.into(),
            technical: None,
            suggestion: None,
            source: None,
        }
    }

    pub fn network(message: impl Into<String>) -> Self {
        Self {
            code: NW_API_UNREACHABLE,
            category: ErrorCategory::Network,
            severity: ErrorSeverity::Error,
            message: message.into(),
            technical: None,
            suggestion: None,
            source: None,
        }
    }

    pub fn system(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            category: ErrorCategory::System,
            severity: ErrorSeverity::Fatal,
            message: message.into(),
            technical: None,
            suggestion: None,
            source: None,
        }
    }

    pub fn recording(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            category: ErrorCategory::Recording,
            severity: ErrorSeverity::Error,
            message: message.into(),
            technical: None,
            suggestion: None,
            source: None,
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: INT_UNEXPECTED,
            category: ErrorCategory::Internal,
            severity: ErrorSeverity::Fatal,
            message: message.into(),
            technical: None,
            suggestion: None,
            source: None,
        }
    }

    pub fn with_technical(mut self, msg: impl Into<String>) -> Self {
        self.technical = Some(msg.into());
        self
    }

    pub fn with_suggestion(mut self, msg: impl Into<String>) -> Self {
        self.suggestion = Some(msg.into());
        self
    }

    pub fn with_source(mut self, module: impl Into<String>) -> Self {
        self.source = Some(module.into());
        self
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {} — {}", self.code, self.message, self.category as u8)
    }
}

impl std::error::Error for AppError {}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        Self::system(IO_WRITE_FAIL, err.to_string())
            .with_technical(format!("io::Error kind: {:?}", err.kind()))
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        Self::internal(err.to_string())
    }
}
