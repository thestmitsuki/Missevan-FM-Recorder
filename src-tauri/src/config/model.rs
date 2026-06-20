use serde::{Deserialize, Serialize};

// ============================================================
// 主播独立配置
// ============================================================
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AnchorConfig {
    pub id: String,
    pub name: String,
    pub url: String,
    pub proxy: Option<String>,
    pub cookie: Option<String>,
    pub enable_check: bool,
    pub check_interval_secs: u64,
}

impl AnchorConfig {
    pub fn new(name: &str, url: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            url: url.to_string(),
            proxy: None,
            cookie: None,
            enable_check: true,
            check_interval_secs: 30,
        }
    }
}

// ============================================================
// 录制格式
// ============================================================
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum RecordFormat {
    M4A,
    MP3,
}

impl Default for RecordFormat {
    fn default() -> Self {
        RecordFormat::M4A
    }
}

// ============================================================
// 全局配置（不含主播列表）
// ============================================================
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GlobalConfig {
    pub output_dir: String,
    pub record_format: RecordFormat,
    pub segment_seconds: u64,
    pub disk_space_limit_gb: f64,
    pub ffmpeg_path: Option<String>,
    #[serde(default)]
    pub anchor_ids: Vec<String>,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            output_dir: "./downloads".to_string(),
            record_format: RecordFormat::M4A,
            segment_seconds: 0,
            disk_space_limit_gb: 1.0,
            ffmpeg_path: None,
            anchor_ids: Vec::new(),
        }
    }
}

// ============================================================
// 完整配置（全局 + 主播列表，内部使用）
// ============================================================
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub output_dir: String,
    pub record_format: RecordFormat,
    pub segment_seconds: u64,
    pub disk_space_limit_gb: f64,
    pub ffmpeg_path: Option<String>,
    pub anchors: Vec<AnchorConfig>,
}

impl Config {
    pub fn from_global(global: GlobalConfig, anchors: Vec<AnchorConfig>) -> Self {
        Self {
            output_dir: global.output_dir,
            record_format: global.record_format,
            segment_seconds: global.segment_seconds,
            disk_space_limit_gb: global.disk_space_limit_gb,
            ffmpeg_path: global.ffmpeg_path,
            anchors,
        }
    }

    pub fn to_global(&self) -> GlobalConfig {
        GlobalConfig {
            output_dir: self.output_dir.clone(),
            record_format: self.record_format,
            segment_seconds: self.segment_seconds,
            disk_space_limit_gb: self.disk_space_limit_gb,
            ffmpeg_path: self.ffmpeg_path.clone(),
            anchor_ids: self.anchors.iter().map(|a| a.id.clone()).collect(),
        }
    }

    pub fn get_anchor(&self, id: &str) -> Option<&AnchorConfig> {
        self.anchors.iter().find(|a| a.id == id)
    }

    pub fn get_anchor_mut(&mut self, id: &str) -> Option<&mut AnchorConfig> {
        self.anchors.iter_mut().find(|a| a.id == id)
    }

    pub fn add_anchor(&mut self, name: &str, url: &str) -> String {
        let anchor = AnchorConfig::new(name, url);
        let id = anchor.id.clone();
        self.anchors.push(anchor);
        id
    }

    pub fn remove_anchor(&mut self, id: &str) -> bool {
        let len = self.anchors.len();
        self.anchors.retain(|a| a.id != id);
        self.anchors.len() < len
    }
}

pub fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}
