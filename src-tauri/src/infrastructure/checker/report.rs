use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub enum CheckStatus {
    Passed,
    Failed,
    Warning,
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub check_name: String,
    pub status: CheckStatus,
    pub message: String,
    pub details: Option<String>,
    pub suggestion: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticReport {
    pub results: Vec<CheckResult>,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub warnings: usize,
    pub timestamp: String,
}

impl DiagnosticReport {
    pub fn new(results: Vec<CheckResult>) -> Self {
        let total = results.len();
        let passed = results.iter().filter(|r| matches!(r.status, CheckStatus::Passed)).count();
        let failed = results.iter().filter(|r| matches!(r.status, CheckStatus::Failed)).count();
        let warnings = results.iter().filter(|r| matches!(r.status, CheckStatus::Warning)).count();

        Self {
            timestamp: chrono::Local::now().to_rfc3339(),
            results,
            total,
            passed,
            failed,
            warnings,
        }
    }
}
