use super::checks::HealthCheck;
use super::report::{CheckResult, DiagnosticReport};

/// 检查调度器——管理检查项集合，支持不同运行模式
pub struct CheckRunner {
    checks: Vec<Box<dyn HealthCheck>>,
}

impl CheckRunner {
    pub fn new() -> Self {
        Self { checks: Vec::new() }
    }

    pub fn register(&mut self, check: Box<dyn HealthCheck>) {
        self.checks.push(check);
    }

    /// 运行所有已注册的检查项
    pub async fn run_all(&self) -> DiagnosticReport {
        let mut results = Vec::with_capacity(self.checks.len());
        for check in &self.checks {
            let result = check.run().await;
            results.push(result);
        }
        DiagnosticReport::new(results)
    }

    /// 运行指定名称的检查项
    pub async fn run_named(&self, name: &str) -> Option<CheckResult> {
        for check in &self.checks {
            if check.name() == name {
                return Some(check.run().await);
            }
        }
        None
    }
}
