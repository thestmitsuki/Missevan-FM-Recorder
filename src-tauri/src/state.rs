use crate::config::model::Config;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// 单个主播的录制任务
pub struct AnchorTask {
    pub cancel_token: CancellationToken,
    pub handle: JoinHandle<()>,
}

/// 全局应用状态
pub struct AppState {
    pub config: Config,
    pub tasks: HashMap<String, AnchorTask>,
    /// 全局取消令牌 —— 应用退出时触发
    pub global_cancel: CancellationToken,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            tasks: HashMap::new(),
            global_cancel: CancellationToken::new(),
        }
    }
}

/// 包装为 Arc<Mutex<...>>，便于多线程共享
pub type AppStateHandle = Arc<Mutex<AppState>>;

/// 用于 Tauri 的 State 包装
pub struct RecorderState {
    pub state: AppStateHandle,
}

impl Default for RecorderState {
    fn default() -> Self {
        // 加载配置，若失败则创建空状态
        let config = crate::config::manager::load_global_config().unwrap_or_else(|_| {
            eprintln!("配置加载失败，使用空默认配置");
            Config {
                output_dir: "./downloads".to_string(),
                record_format: crate::config::model::RecordFormat::M4A,
                segment_seconds: 0,
                disk_space_limit_gb: 1.0,
                ffmpeg_path: None,
                anchors: vec![],
            }
        });
        Self {
            state: Arc::new(Mutex::new(AppState::new(config))),
        }
    }
}
