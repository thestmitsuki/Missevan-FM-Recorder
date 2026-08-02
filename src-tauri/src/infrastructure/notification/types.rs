use chrono::{DateTime, Local};
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum NotificationLevel {
    Info,
    Warning,
    Error,
    Critical,
}

#[derive(Debug, Clone, Serialize)]
pub struct Notification {
    pub id: String,
    pub code: String,
    pub level: NotificationLevel,
    pub title: String,
    pub message: String,
    pub suggestion: Option<String>,
    pub source: String,
    pub timestamp: DateTime<Local>,
    pub actionable: bool,
}

impl Notification {
    pub fn new(
        code: impl Into<String>,
        level: NotificationLevel,
        title: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            code: code.into(),
            level,
            title: title.into(),
            message: message.into(),
            suggestion: None,
            source: String::from("system"),
            timestamp: Local::now(),
            actionable: false,
        }
    }

    pub fn with_suggestion(mut self, text: impl Into<String>) -> Self {
        self.suggestion = Some(text.into());
        self
    }

    pub fn with_source(mut self, module: impl Into<String>) -> Self {
        self.source = module.into();
        self
    }

    pub fn with_actionable(mut self) -> Self {
        self.actionable = true;
        self
    }
}
