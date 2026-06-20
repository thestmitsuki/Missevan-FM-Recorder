use anyhow::Result;
use std::fs;
use std::path::PathBuf;

use super::model::{normalize_path, AnchorConfig, Config, GlobalConfig};

/// 配置存储管理 —— 全局配置 + 每主播独立文件
pub struct AnchorStore {
    pub config: Config,
}

impl AnchorStore {
    pub fn load() -> Result<Self> {
        let config_dir = config_dir_path();
        let global_path = global_config_path();

        // 确保目录存在
        fs::create_dir_all(&config_dir)?;
        fs::create_dir_all(anchors_dir_path())?;

        // 如果全局配置不存在，创建模板
        if !global_path.exists() {
            Self::create_default_config()?;
            anyhow::bail!("配置文件已创建模板，请编辑后重启程序");
        }

        let content = fs::read_to_string(&global_path)?;
        let raw: serde_json::Value = toml::from_str(&content)?;

        // 检测旧格式（含 anchors 数组）
        if raw.get("anchors").and_then(|a| a.as_array()).map_or(false, |a| !a.is_empty()) {
            // 旧格式迁移：加载完整配置，迁移到新结构
            let old_config: OldConfigFormat = toml::from_str(&content)?;
            return Self::migrate_from_old(old_config);
        }

        // 新格式：全局配置 + 独立 Anchor 文件
        let global: GlobalConfig = toml::from_str(&content)?;
        let anchors = Self::load_anchor_configs(&global.anchor_ids)?;

        let mut config = Config::from_global(global, anchors);

        // 清洗空字符串
        Self::sanitize_config(&mut config);

        Ok(AnchorStore { config })
    }

    /// 从旧格式迁移
    fn migrate_from_old(old: OldConfigFormat) -> Result<Self> {
        let mut anchors = old.anchors;

        // 写入每个主播的独立配置文件
        let anchors_dir = anchors_dir_path();
        for anchor in &anchors {
            let anchor_path = anchors_dir.join(format!("{}.toml", anchor.id));
            if !anchor_path.exists() {
                let content = toml::to_string_pretty(anchor)?;
                fs::write(&anchor_path, content)?;
            }
        }

        // 构建新全局配置
        let anchor_ids: Vec<String> = anchors.iter().map(|a| a.id.clone()).collect();
        let mut global = GlobalConfig {
            output_dir: old.output_dir,
            record_format: old.record_format,
            segment_seconds: old.segment_seconds,
            disk_space_limit_gb: old.disk_space_limit_gb,
            ffmpeg_path: old.ffmpeg_path,
            anchor_ids,
        };

        // 清洗配置
        Self::sanitize_global(&mut global);
        let global_content = toml::to_string_pretty(&global)?;
        fs::write(global_config_path(), global_content)?;

        Self::sanitize_anchors(&mut anchors);

        let config = Config::from_global(global, anchors);
        Ok(AnchorStore { config })
    }

    /// 加载所有主播独立配置
    fn load_anchor_configs(ids: &[String]) -> Result<Vec<AnchorConfig>> {
        let anchors_dir = anchors_dir_path();
        let mut anchors = Vec::new();

        for id in ids {
            let path = anchors_dir.join(format!("{}.toml", id));
            if path.exists() {
                match fs::read_to_string(&path) {
                    Ok(content) => {
                        match toml::from_str::<AnchorConfig>(&content) {
                            Ok(cfg) => anchors.push(cfg),
                            Err(e) => {
                                eprintln!("加载主播配置失败 ({}): {}", id, e);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("读取主播配置文件失败 ({}): {}", id, e);
                    }
                }
            } else {
                eprintln!("主播配置文件未找到 ({}), 跳过", id);
            }
        }

        Ok(anchors)
    }

    /// 保存全局配置
    pub fn save_global(&self) -> Result<()> {
        let global = self.config.to_global();
        let content = toml::to_string_pretty(&global)?;
        fs::write(global_config_path(), content)?;
        Ok(())
    }

    /// 保存单个主播配置到独立文件
    pub fn save_anchor(&self, anchor: &AnchorConfig) -> Result<()> {
        let anchors_dir = anchors_dir_path();
        fs::create_dir_all(&anchors_dir)?;
        let path = anchors_dir.join(format!("{}.toml", anchor.id));
        let content = toml::to_string_pretty(anchor)?;
        fs::write(path, content)?;
        Ok(())
    }

    /// 删除单个主播配置文件
    pub fn delete_anchor_file(id: &str) {
        let path = anchors_dir_path().join(format!("{}.toml", id));
        if path.exists() {
            let _ = fs::remove_file(path);
        }
    }

    // ---------- 内部辅助 ----------

    fn sanitize_config(config: &mut Config) {
        Self::sanitize_global_raw(config);
        Self::sanitize_anchors(&mut config.anchors);
    }

    fn sanitize_global_raw(config: &mut Config) {
        if config.ffmpeg_path.as_deref() == Some("") {
            config.ffmpeg_path = None;
        }
        if let Some(ref mut path) = config.ffmpeg_path {
            *path = normalize_path(path);
        }
        if config.anchors.is_empty() {
            config.anchors.push(AnchorConfig::new("默认主播", ""));
        }
    }

    fn sanitize_global(global: &mut GlobalConfig) {
        if global.ffmpeg_path.as_deref() == Some("") {
            global.ffmpeg_path = None;
        }
        if let Some(ref mut path) = global.ffmpeg_path {
            *path = normalize_path(path);
        }
    }

    fn sanitize_anchors(anchors: &mut [AnchorConfig]) {
        for anchor in anchors.iter_mut() {
            if anchor.proxy.as_deref() == Some("") {
                anchor.proxy = None;
            }
            if anchor.cookie.as_deref() == Some("") {
                anchor.cookie = None;
            }
        }
    }

    fn create_default_config() -> Result<()> {
        let default_global = GlobalConfig {
            anchor_ids: vec!["default".to_string()],
            ..Default::default()
        };
        let content = toml::to_string_pretty(&default_global)?;
        let header = "# 猫耳FM录制器全局配置\n".to_string() + &content;
        fs::write(global_config_path(), header)?;

        // 创建默认主播配置
        let default_anchor = AnchorConfig::new("默认主播", "");
        let anchor_content = toml::to_string_pretty(&default_anchor)?;
        let anchors_dir = anchors_dir_path();
        fs::create_dir_all(&anchors_dir)?;
        fs::write(anchors_dir.join("default.toml"), anchor_content)?;

        Ok(())
    }
}

// ---------- 路径辅助 ----------

fn config_dir_path() -> PathBuf {
    PathBuf::from("./config")
}

fn global_config_path() -> PathBuf {
    config_dir_path().join("config.toml")
}

fn anchors_dir_path() -> PathBuf {
    config_dir_path().join("anchors")
}

// ---------- 旧格式（迁移用） ----------

#[derive(serde::Deserialize)]
struct OldConfigFormat {
    output_dir: String,
    record_format: super::model::RecordFormat,
    segment_seconds: u64,
    disk_space_limit_gb: f64,
    ffmpeg_path: Option<String>,
    anchors: Vec<AnchorConfig>,
}

// ---------- 兼容旧 API （逐步淘汰） ----------

pub fn load_global_config() -> Result<Config> {
    let store = AnchorStore::load()?;
    Ok(store.config)
}

pub fn migrate_if_needed() -> Result<()> {
    AnchorStore::load()?;
    Ok(())
}

pub fn save_global_config(config: &Config) -> Result<()> {
    let store = AnchorStore { config: config.clone() };
    store.save_global()?;
    // 保存每个主播配置
    for anchor in &config.anchors {
        store.save_anchor(anchor)?;
    }
    Ok(())
}
