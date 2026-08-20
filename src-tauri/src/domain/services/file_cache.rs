use crate::domain::config::manager::ConfigManager;
use crate::domain::services::fs_walk;
use crate::infrastructure::error::types::AppError;
use crate::infrastructure::state::app_state::AppStateHandle;
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;
use tauri::Emitter;
use tauri::WebviewWindow;
use tokio::sync::Mutex;

/// 扫描日志上限
const SCAN_LOG_LIMIT: usize = 20;

/// 路径键归一化：统一分隔符为 `/`（Windows 下 `\` 与 `/` 混用，
/// 录制引擎 output_path 用 `/` 拼接、缓存扫描经 Path 产出 `\`，直接比较会漏判）。
pub(crate) fn path_key(p: &str) -> String {
    p.replace('\\', "/")
}

// 文件信息结构
#[derive(Debug, Clone, Serialize)]
pub struct RecordingFile {
    pub id: String,
    pub name: String,
    pub path: String,
    pub size: u64,
    pub duration: f64,
    pub anchor_name: String,
    pub created_at: SystemTime,
    pub group_prefix: Option<String>,
    pub segment_index: Option<u32>,
    /// 是否正被录制引擎写入（对照活跃录制任务输出路径；前端显示「录制中」并禁删/禁重命名）
    pub is_active: bool,
}

// 分段组
#[derive(Debug, Clone, Serialize)]
pub struct FileGroup {
    pub prefix: String,
    pub files: Vec<RecordingFile>,
    pub total_size: u64,
    pub total_duration: f64,
}

/// 文件夹树节点（`get_recording_files` / `recording_files_changed` 载荷的顶层
/// 结构）：录制输出目录 → 主播文件夹 → 音频文件。
///
/// - `name`：主播名（沿用 `anchor_name`，已剥离 `-房间号`；输出目录根下的
///   文件为空字符串，前端显示「未分类」）
/// - `path`：磁盘文件夹路径（文件夹身份键；同主播名可能对应多个磁盘文件夹）
#[derive(Debug, Clone, Serialize)]
pub struct FolderNode {
    pub name: String,
    pub path: String,
    pub files: Vec<RecordingFile>,
}

/// 从扁平文件构建文件夹树：按文件父目录聚合。
/// 不再按文件名前缀分组（需求变更：文件页不再展示分段，主播文件夹内
/// 全部音频文件平铺展示，播放即播放该主播全部音频）。
pub(crate) fn build_folder_tree(files: &[RecordingFile]) -> Vec<FolderNode> {
    /// 文件夹身份键 = 文件父目录路径（兼容 Windows `\` 与 Unix `/`）
    fn folder_key_of(f: &RecordingFile) -> String {
        match f.path.rfind(['/', '\\']) {
            Some(idx) => f.path[..idx].to_string(),
            None => String::new(),
        }
    }

    fn folder_index(
        index: &mut HashMap<String, usize>,
        folders: &mut Vec<FolderNode>,
        key: &str,
        name: &str,
    ) -> usize {
        if let Some(&i) = index.get(key) {
            return i;
        }
        let i = folders.len();
        folders.push(FolderNode {
            name: name.to_string(),
            path: key.to_string(),
            files: Vec::new(),
        });
        index.insert(key.to_string(), i);
        i
    }

    let mut folders: Vec<FolderNode> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();

    for f in files {
        let key = folder_key_of(f);
        let i = folder_index(&mut index, &mut folders, &key, &f.anchor_name);
        folders[i].files.push(f.clone());
    }
    folders
}

/// 一次缓存扫描/清空的记录（调试页「文件缓存」模块扫描日志）
#[derive(Debug, Clone, Serialize)]
pub struct ScanLogEntry {
    /// RFC3339 时间戳
    pub timestamp: String,
    /// "scan" | "clear"
    pub kind: String,
    /// 耗时（毫秒；clear 为 0）
    pub duration_ms: u64,
    pub files_before: usize,
    pub files_after: usize,
    pub groups: usize,
}

/// 文件缓存状态（`get_file_cache_state` 返回值）
#[derive(Debug, Clone, Serialize)]
pub struct FileCacheState {
    /// 上次扫描时间（RFC3339）；从未扫描 = None
    pub last_scan_at: Option<String>,
    pub file_count: usize,
    pub group_count: usize,
    pub total_size_bytes: u64,
    /// 扫描日志（最新在前，上限 20）
    pub scan_log: Vec<ScanLogEntry>,
}

// 缓存数据
pub struct FileCache {
    pub files: Vec<RecordingFile>,
    pub groups: Vec<FileGroup>,
    pub scan_time: SystemTime,
    /// 扫描日志（最新在前，上限 20）
    pub scan_log: VecDeque<ScanLogEntry>,
}

impl FileCache {
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            groups: Vec::new(),
            scan_time: SystemTime::now(),
            scan_log: VecDeque::new(),
        }
    }

    /// 追加一条扫描/清空日志（最新在前，超限丢最旧）
    pub fn push_scan_log(&mut self, entry: ScanLogEntry) {
        self.scan_log.push_front(entry);
        if self.scan_log.len() > SCAN_LOG_LIMIT {
            self.scan_log.pop_back();
        }
    }

    /// 清除缓存（只清内存索引，不动磁盘文件）
    pub fn clear_cache(&mut self) {
        self.files.clear();
        self.groups.clear();
    }

    /// 调试页状态快照
    pub fn state(&self) -> FileCacheState {
        let dt: chrono::DateTime<chrono::Utc> = self.scan_time.into();
        FileCacheState {
            last_scan_at: Some(dt.to_rfc3339()),
            file_count: self.files.len(),
            group_count: self.groups.len(),
            total_size_bytes: self.files.iter().map(|f| f.size).sum(),
            scan_log: self.scan_log.iter().cloned().collect(),
        }
    }
}

/// 按活跃录制输出路径集合标记 `is_active`（路径经 path_key 归一化后比较）。
pub(crate) fn mark_active(files: &mut [RecordingFile], active_paths: &HashSet<String>) {
    for f in files {
        f.is_active = active_paths.contains(&path_key(&f.path));
    }
}

/// 单次目录遍历的扫描产物（O1 去双扫）：文件缓存条目 + 各文件修改时间。
/// `modified` 与 `files` 下标一一对应；获取修改时间失败为 `None`
///（仅清理侧消费，见 `scan_output_once` 注释）。
pub(crate) struct OutputScan {
    /// 文件缓存条目（与 `scan_output_dir` 产出完全一致）
    pub(crate) files: Vec<RecordingFile>,
    /// 每个文件对应的修改时间（清理服务规划删除用；文件缓存不消费）
    pub(crate) modified: Vec<Option<SystemTime>>,
}

pub struct FileCacheManager {
    window: WebviewWindow,
    cache: FileCacheHandle,
}

pub type FileCacheHandle = Arc<Mutex<FileCache>>;

impl FileCacheManager {
    pub fn new(window: WebviewWindow, cache: FileCacheHandle) -> Self {
        Self { window, cache }
    }

    /// 刷新缓存：递归扫描输出目录，收集全部录制音频文件并构建分段组。
    ///
    /// 回归修复（文件页空内容）：旧实现只扫描「当前配置主播」的
    /// `{主播名}-{房间号}` 子目录——主播从配置移除/配置重置后，磁盘上已录制
    /// 的音频文件立即从文件页消失（文件页显示空态）。改为与清理服务
    /// （cleanup.rs scan_recording_files）一致的输出目录递归扫描，
    /// 文件页展示输出目录内的全部录制文件（规格「文件页面功能规格」：
    /// 浏览、搜索、播放和管理**所有**已录制的音频文件）。
    ///
    /// `app_state`：活跃录制任务（FfmpegRecorder 输出路径集合）来源——
    /// 正在被写入的音频文件标记 `is_active`（前端显示「录制中」并禁删/禁重命名）。
    pub async fn refresh(
        &self,
        config_manager: &ConfigManager,
        app_state: &AppStateHandle,
    ) -> Result<(), AppError> {
        let scan_start = std::time::Instant::now();
        let config = config_manager.load()?;
        let output_dir = Path::new(&config.global.output_dir).to_path_buf();

        // 活跃录制输出路径（先取路径集再锁缓存，避免与命令侧锁顺序反转造成死锁）
        let active_paths = {
            let state = app_state.lock().await;
            state.active_output_paths()
        };

        // O1：同步递归扫描移出 tokio worker（spawn_blocking）——大目录/机械盘/
        // 网络盘上单次扫描可达百 ms~秒级，直接跑在 worker 上会阻塞同线程其他
        // async 任务（录制结束刷新缓存时 UI/检测循环被卡）。
        let files = match tauri::async_runtime::spawn_blocking(move || {
            Self::scan_output_dir(&output_dir)
        })
        .await
        {
            // 根目录读取失败 / 任务异常 → 空列表（与旧 scan_output_dir 语义一致）
            Ok(files) => files,
            Err(_) => Vec::new(),
        };

        self.apply_scan(files, &active_paths, scan_start).await
    }

    /// 用「清理路径单次扫描的产物」刷新缓存（O1 去双扫）：
    /// `run_cleanup` 删除后基于同一份扫描产物重建缓存（已删除文件在调用侧
    /// 过滤），不再二次全量扫描输出目录。行为与 `refresh` 的扫描后阶段完全
    /// 一致：标记活跃 → 排序 → 分组 → 更新缓存 → emit `recording_files_changed`。
    pub(crate) async fn refresh_from_files(
        &self,
        files: Vec<RecordingFile>,
        active_paths: HashSet<String>,
    ) -> Result<(), AppError> {
        self.apply_scan(files, &active_paths, std::time::Instant::now())
            .await
    }

    /// 扫描后处理（refresh / refresh_from_files 共用）：标记活跃 → 按创建时间
    /// 降序排序 → 更新缓存 → 推送 `recording_files_changed`。
    /// `scan_start` 用于扫描日志耗时（含扫描等待时间）。
    ///
    /// 不再按文件名前缀构建分段组（需求变更：文件页不分段展示，主播文件夹
    /// 内全部音频文件平铺；播放即播放该主播全部音频）。`cache.groups` 恒为空。
    async fn apply_scan(
        &self,
        mut files: Vec<RecordingFile>,
        active_paths: &HashSet<String>,
        scan_start: std::time::Instant,
    ) -> Result<(), AppError> {
        mark_active(&mut files, active_paths);

        // 按创建时间降序排序
        files.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        // 更新缓存
        let mut cache = self.cache.lock().await;
        let files_before = cache.files.len();
        cache.files = files;
        cache.groups = Vec::new();
        cache.scan_time = SystemTime::now();
        let files_after = cache.files.len();
        let group_count = cache.groups.len();
        cache.push_scan_log(ScanLogEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            kind: "scan".into(),
            duration_ms: scan_start.elapsed().as_millis() as u64,
            files_before,
            files_after,
            groups: group_count,
        });

        // 推送事件到前端（文件夹树：输出目录 → 主播文件夹 → 音频文件）
        let payload = serde_json::json!({
            "folders": build_folder_tree(&cache.files),
        });
        let _ = self.window.emit("recording_files_changed", &payload);

        Ok(())
    }

    /// 递归扫描输出目录，收集所有录制音频文件（`scan_output_once` 的缓存侧
    /// 视图：扩展名 m4a/aac/mp3/flac，大小写不敏感；输出目录不存在 → 空列表）。
    fn scan_output_dir(root: &Path) -> Vec<RecordingFile> {
        // 根目录读取失败（如不存在/指向普通文件）→ 空列表（与旧实现一致）
        match Self::scan_output_once(root) {
            Ok(scan) => scan.files,
            Err(_) => Vec::new(),
        }
    }

    /// 单次遍历同时产出「文件缓存条目 + 清理候选元数据」（O1 去双扫）。
    ///
    /// 背景：清理路径（cleanup.rs run_cleanup）原先先 `scan_recording_files`
    /// 全量遍历一次、删除后 `FileCacheManager::refresh` 又全量遍历第二次——
    /// 同一目录连续扫两遍。本函数只遍历一次：`safe_walk_files` 收集路径 +
    /// 每文件一次 `metadata()`，缓存条目与清理候选（修改时间）由同一份
    /// 元数据派生，两处各自构建自己的视图。
    ///
    /// 主播名从文件所在父目录名推导（录制引擎输出结构 `{主播名}-{房间号}` →
    /// 剥离房间号后缀；输出目录根下的文件主播名为空）。
    /// 错误语义：根目录读取失败 → `Err`（cleanup 透传为 AppError、refresh
    /// 退回空列表，与两处旧行为一致）；子树/单文件失败 → 跳过，不中断整次扫描。
    /// 返回的 `files` 与 `modified` 下标一一对应；修改时间获取失败为 `None`
    ///（仅清理侧消费：与旧 `scan_recording_files` 的 modified 失败跳过语义
    /// 一致；缓存侧不受影响——created 失败仍走 unwrap_or(now) 兜底）。
    ///
    /// S1：遍历统一走 `fs_walk::safe_walk_files`（条目经 symlink_metadata
    /// 不跟随链接判定，符号链接 / Windows junction 一律跳过），目录内链接
    /// 不会把缓存/清理候选带出目录外。
    pub(crate) fn scan_output_once(root: &Path) -> io::Result<OutputScan> {
        let files = fs_walk::safe_walk_files(root)?;
        let mut out = Vec::new();
        let mut modified = Vec::new();
        for path in files {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if matches!(ext.to_lowercase().as_str(), "m4a" | "aac" | "mp3" | "flac") {
                    // safe_walk_files 已保证 path 是普通文件（非链接），
                    // metadata() 不会跟随链接解析到目录外
                    let Ok(metadata) = path.metadata() else {
                        continue;
                    };
                    let name = path.file_name().unwrap().to_string_lossy().to_string();
                    let (group_prefix, segment_index) = Self::parse_segment_info(&name);
                    out.push(RecordingFile {
                        id: path.to_string_lossy().to_string(),
                        name: name.clone(),
                        path: path.to_string_lossy().to_string(),
                        size: metadata.len(),
                        duration: 0.0, // 后续可通过 ffprobe 异步填充
                        anchor_name: Self::anchor_name_of(&path, root),
                        created_at: metadata.created().unwrap_or(SystemTime::now()),
                        group_prefix,
                        segment_index,
                        is_active: false, // 由 refresh 按活跃录制任务标记
                    });
                    modified.push(metadata.modified().ok());
                }
            }
        }
        Ok(OutputScan {
            files: out,
            modified,
        })
    }

    /// 从文件路径推导主播名：取父目录名并剥离尾部 `-{房间号}` 后缀
    /// （录制引擎输出结构 `{主播名}-{房间号}`）；输出目录根下的文件返回空字符串。
    fn anchor_name_of(path: &Path, output_dir: &Path) -> String {
        let Some(parent) = path.parent() else {
            return String::new();
        };
        if parent == output_dir {
            return String::new();
        }
        let Some(dir_name) = parent.file_name().and_then(|n| n.to_str()) else {
            return String::new();
        };
        if let Some(pos) = dir_name.rfind('-') {
            let tail = &dir_name[pos + 1..];
            if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) {
                return dir_name[..pos].to_string();
            }
        }
        dir_name.to_string()
    }

    /// 解析文件名中的分段信息：前缀_001.ext。
    ///
    /// 段号必须是**计数器形态**（`%03d` 最小宽度 3 的补零语义）：
    /// 1. 全数字且 ≥3 位（`_001`…`_999`、`_1000`…，1-2 位数字后缀不归组）；
    /// 2. 4 位及以上不允许前导 0 —— 段号 ≥1000 时 `%03d` 不会补前导 0，
    ///    而时间戳（HHMMSS）恒为 6 位且常带前导 0（如 `_003019`），必须排除，
    ///    否则 `主播名_日期_时间.m4a` 会被误判为分段（文件页出现「主播名_日期」
    ///    伪分段组）；
    /// 3. 数值上限 99_999 —— 时间 HHMMSS（≤235959）与房间号等长数字串不是
    ///    段号，实际录制段数远低于此。
    ///
    /// 兜底：`apply_scan` 会把「只有 1 个文件的分段组」降级为普通文件，
    /// 消除残余误判（如无前导 0 的 6 位时间 `_123456`）。
    fn parse_segment_info(filename: &str) -> (Option<String>, Option<u32>) {
        if let Some(pos) = filename.rfind("_") {
            let (base, idx_part) = filename.split_at(pos);
            let idx_str = &idx_part[1..]; // 去掉下划线
            if let Some(dot_pos) = idx_str.find('.') {
                let num_str = &idx_str[..dot_pos];
                let is_counter = num_str.len() >= 3
                    && num_str.chars().all(|c| c.is_ascii_digit())
                    && !(num_str.len() >= 4 && num_str.starts_with('0'))
                    && num_str
                        .parse::<u32>()
                        .map_or(false, |n| n <= 99_999);
                if is_counter {
                    if let Ok(num) = num_str.parse::<u32>() {
                        return (Some(base.to_string()), Some(num));
                    }
                }
            }
        }
        (None, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan_entry(kind: &str) -> ScanLogEntry {
        ScanLogEntry {
            timestamp: "2026-08-01T00:00:00Z".to_string(),
            kind: kind.to_string(),
            duration_ms: 12,
            files_before: 1,
            files_after: 2,
            groups: 1,
        }
    }

    #[test]
    fn scan_log_newest_first_and_capped() {
        let mut cache = FileCache::new();
        for _ in 0..(SCAN_LOG_LIMIT + 5) {
            cache.push_scan_log(scan_entry("scan"));
        }
        assert_eq!(cache.scan_log.len(), SCAN_LOG_LIMIT);
        assert_eq!(cache.state().scan_log.len(), SCAN_LOG_LIMIT);
    }

    #[test]
    fn clear_cache_empties_index_keeps_scan_log() {
        let mut cache = FileCache::new();
        cache.push_scan_log(scan_entry("scan"));
        cache.clear_cache();
        let state = cache.state();
        assert_eq!(state.file_count, 0);
        assert_eq!(state.group_count, 0);
        assert_eq!(state.total_size_bytes, 0);
        assert_eq!(state.scan_log.len(), 1);
    }

    #[test]
    fn state_reports_counts_and_total_size() {
        let mut cache = FileCache::new();
        cache.files.push(RecordingFile {
            id: "1".into(),
            name: "a.m4a".into(),
            path: "/out/a.m4a".into(),
            size: 100,
            duration: 60.0,
            anchor_name: "主播A".into(),
            created_at: SystemTime::now(),
            group_prefix: None,
            segment_index: None,
            is_active: false,
        });
        cache.files.push(RecordingFile {
            id: "2".into(),
            name: "b.m4a".into(),
            path: "/out/b.m4a".into(),
            size: 200,
            duration: 30.0,
            anchor_name: "主播A".into(),
            created_at: SystemTime::now(),
            group_prefix: None,
            segment_index: None,
            is_active: false,
        });
        let state = cache.state();
        assert_eq!(state.file_count, 2);
        assert_eq!(state.total_size_bytes, 300);
        assert!(state.last_scan_at.is_some());
    }

    fn scan_root(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "missevan-test-filecache-scan-{}-{}-{}",
            tag,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// 回归：文件页空内容——扫描不得依赖「当前配置主播」，输出目录内
    /// 任意子目录/根目录的音频文件都应被发现（主播移除后旧录音仍可见）。
    #[test]
    fn scan_output_dir_finds_files_in_subdirs_and_root() {
        let root = scan_root("all");
        std::fs::create_dir_all(root.join("主播A-100000001")).unwrap();
        std::fs::create_dir_all(root.join("主播B-100000002")).unwrap();
        std::fs::create_dir_all(root.join("nested/deep")).unwrap();
        std::fs::write(
            root.join("主播A-100000001/主播A_20260722_200600.m4a"),
            b"x",
        )
        .unwrap();
        std::fs::write(
            root.join("主播B-100000002/主播B_20260722_202154.m4a"),
            b"x",
        )
        .unwrap();
        std::fs::write(root.join("nested/deep/seg_001.mp3"), b"x").unwrap();
        std::fs::write(root.join("root-level.MP3"), b"x").unwrap();
        std::fs::write(root.join("notes.txt"), b"x").unwrap();

        let files = FileCacheManager::scan_output_dir(&root);
        let mut names: Vec<String> = files.iter().map(|f| f.name.clone()).collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "root-level.MP3",
                "seg_001.mp3",
                "主播A_20260722_200600.m4a",
                "主播B_20260722_202154.m4a"
            ]
        );

        // 主播名：`{主播名}-{房间号}` 子目录 → 剥离房间号
        let third = files
            .iter()
            .find(|f| f.name.starts_with("主播A"))
            .unwrap();
        assert_eq!(third.anchor_name, "主播A");
        // 输出目录根下的文件 → 主播名为空
        let root_file = files
            .iter()
            .find(|f| f.name == "root-level.MP3")
            .unwrap();
        assert_eq!(root_file.anchor_name, "");
        // 非录制引擎结构目录 → 原样取父目录名；分段文件名正常解析
        let seg = files.iter().find(|f| f.name.contains("seg_001")).unwrap();
        assert_eq!(seg.anchor_name, "deep");
        assert_eq!(seg.group_prefix.as_deref(), Some("seg"));
        assert_eq!(seg.segment_index, Some(1));

        let _ = std::fs::remove_dir_all(&root);
    }

    // ── L3/M1：分段段号 ≥1000（`_1000` 4 位）归组回归 ──

    #[test]
    fn parse_segment_info_handles_3_digit_segments() {
        // 旧格式回归：_001…_999 仍按 3 位段号归组（逐字节兼容）
        assert_eq!(
            FileCacheManager::parse_segment_info("2026-08-07_12-30-45_主播A_001_001.m4a"),
            (Some("2026-08-07_12-30-45_主播A_001".to_string()), Some(1))
        );
        assert_eq!(
            FileCacheManager::parse_segment_info("base_999.m4a"),
            (Some("base".to_string()), Some(999))
        );
    }

    #[test]
    fn parse_segment_info_handles_4_plus_digit_segments() {
        // L3/M1：段号 ≥1000（4 位及以上）必须归组——第 1000 段起不再落入
        // 「非分段」分支
        assert_eq!(
            FileCacheManager::parse_segment_info("base_1000.m4a"),
            (Some("base".to_string()), Some(1000))
        );
        assert_eq!(
            FileCacheManager::parse_segment_info("base_1001.m4a"),
            (Some("base".to_string()), Some(1001))
        );
        assert_eq!(
            FileCacheManager::parse_segment_info("2026-08-07_12-30-45_主播A_001_1000.m4a"),
            (
                Some("2026-08-07_12-30-45_主播A_001".to_string()),
                Some(1000)
            )
        );
    }

    #[test]
    fn parse_segment_info_keeps_old_boundaries() {
        // 3 位以下数字后缀依旧不归组（行为不变）：_1 / _01
        assert_eq!(
            FileCacheManager::parse_segment_info("base_1.m4a"),
            (None, None)
        );
        assert_eq!(
            FileCacheManager::parse_segment_info("base_01.m4a"),
            (None, None)
        );
        // 非数字 / 无下划线 → 不归组
        assert_eq!(
            FileCacheManager::parse_segment_info("base_abc.m4a"),
            (None, None)
        );
        assert_eq!(
            FileCacheManager::parse_segment_info("nounderscore.m4a"),
            (None, None)
        );
    }

    /// 文件页「主播名成为伪分段组」回归：`主播名_日期_时间.m4a` 的时间戳
    /// （HHMMSS）不得误判为段号。
    #[test]
    fn parse_segment_info_rejects_time_stamps() {
        // 前导 0 时间（003019 = 00:30:19）：4 位以上前导 0 不是 %03d 计数器形态
        assert_eq!(
            FileCacheManager::parse_segment_info("一口77呀吼眠神冠_20260803_003019.m4a"),
            (None, None)
        );
        assert_eq!(
            FileCacheManager::parse_segment_info("惠子_㋇㏠三周年顺利_20260802_010533.m4a"),
            (None, None)
        );
        // 无前导 0 的时间（180252 > 99_999 上限）：同样排除
        assert_eq!(
            FileCacheManager::parse_segment_info("奕境__20260802_180252.mp3"),
            (None, None)
        );
        // 上限边界：99999 以内仍是段号，100000 起不是
        assert_eq!(
            FileCacheManager::parse_segment_info("base_99999.m4a"),
            (Some("base".to_string()), Some(99999))
        );
        assert_eq!(
            FileCacheManager::parse_segment_info("base_100000.m4a"),
            (None, None)
        );
        // 4 位以上前导 0（_0999）不是 %03d 输出（999 → "999"），不归组
        assert_eq!(
            FileCacheManager::parse_segment_info("base_0999.m4a"),
            (None, None)
        );
        // 真分段：时间戳后的 _001 仍是段号（默认模板 + 分段录制）
        assert_eq!(
            FileCacheManager::parse_segment_info("2026-08-20_16-00-06_主播A_001.m4a"),
            (Some("2026-08-20_16-00-06_主播A".to_string()), Some(1))
        );
        assert_eq!(
            FileCacheManager::parse_segment_info("主播A_20260820_160006_002.m4a"),
            (Some("主播A_20260820_160006".to_string()), Some(2))
        );
    }

    /// 文件夹树聚合：文件按父目录分组、显示名取主播名（剥离 -房间号）、
    /// 未分类（根下文件）name 为空。不再有分段组（全部文件平铺）。
    #[test]
    fn build_folder_tree_groups_by_parent_dir() {
        let mk = |name: &str, path: &str, anchor: &str| RecordingFile {
            id: path.into(),
            name: name.into(),
            path: path.into(),
            size: 100,
            duration: 10.0,
            anchor_name: anchor.into(),
            created_at: SystemTime::now(),
            group_prefix: None,
            segment_index: None,
            is_active: false,
        };
        let files = vec![
            mk(
                "一口77呀吼眠神冠_20260803_003019.m4a",
                "D:/rec/一口77呀吼眠神冠-869021004/一口77呀吼眠神冠_20260803_003019.m4a",
                "一口77呀吼眠神冠",
            ),
            mk(
                "主播A_20260820_160006_001.m4a",
                "D:/rec/主播A-1001/主播A_20260820_160006_001.m4a",
                "主播A",
            ),
            mk(
                "主播A_20260820_160006_002.m4a",
                "D:/rec/主播A-1001/主播A_20260820_160006_002.m4a",
                "主播A",
            ),
            mk(
                "root.m4a",
                "D:/rec/root.m4a",
                "",
            ),
        ];

        let folders = build_folder_tree(&files);
        let by_path = |p: &str| folders.iter().find(|f| f.path == p).unwrap();
        let f1 = by_path("D:/rec/一口77呀吼眠神冠-869021004");
        assert_eq!(f1.name, "一口77呀吼眠神冠");
        assert_eq!(f1.files.len(), 1);

        let root = by_path("D:/rec");
        assert_eq!(root.name, "", "输出目录根下文件 → 未分类（name 空）");
        assert_eq!(root.files.len(), 1);

        let a1 = by_path("D:/rec/主播A-1001");
        assert_eq!(a1.name, "主播A");
        assert_eq!(a1.files.len(), 2, "主播文件夹内全部音频文件平铺");
    }

    /// L3/M1 回归：扫描产物中 4 位段号文件必须带 segment_index（文件页归组入口）
    #[test]
    fn scan_output_once_parses_4_digit_segments() {
        let root = scan_root("seg4");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("live_0999.m4a"), b"x").unwrap();
        std::fs::write(root.join("live_1000.m4a"), b"x").unwrap();
        std::fs::write(root.join("live_1001.m4a"), b"x").unwrap();
        std::fs::write(root.join("single_001.m4a"), b"x").unwrap();

        let scan = FileCacheManager::scan_output_once(&root).unwrap();
        let idx = |name: &str| {
            scan.files
                .iter()
                .find(|f| f.name == name)
                .map(|f| (f.group_prefix.clone(), f.segment_index))
        };
        assert_eq!(
            idx("live_0999.m4a"),
            Some((None, None)),
            "0999 前导 0 不是 %03d 输出（999 → \"999\"），不归组"
        );
        assert_eq!(
            idx("live_1000.m4a"),
            Some((Some("live".to_string()), Some(1000))),
            "4 位段号必须归组（L3/M1）"
        );
        assert_eq!(
            idx("live_1001.m4a"),
            Some((Some("live".to_string()), Some(1001)))
        );
        assert_eq!(
            idx("single_001.m4a"),
            Some((Some("single".to_string()), Some(1))),
            "独立 3 位文件正常归组"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_output_dir_missing_dir_returns_empty() {
        let root = scan_root("missing");
        assert!(FileCacheManager::scan_output_dir(&root).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// O1：单次遍历产物中 modified 与 files 下标一一对应（清理候选消费同一份
    /// 元数据），且与 scan_output_dir 的缓存侧视图产出一致
    #[test]
    fn scan_output_once_aligns_files_and_modified() {
        let root = scan_root("once");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("a.m4a"), b"x").unwrap();
        std::fs::write(root.join("sub/b.mp3"), b"x").unwrap();
        std::fs::write(root.join("c.txt"), b"x").unwrap();

        let scan = FileCacheManager::scan_output_once(&root).unwrap();
        assert_eq!(scan.files.len(), 2, "仅录制扩展名文件入产物");
        assert_eq!(
            scan.modified.len(),
            scan.files.len(),
            "modified 与 files 一一对应"
        );
        assert!(
            scan.modified.iter().all(|m| m.is_some()),
            "新建文件的修改时间应可读"
        );

        // 与 scan_output_dir（缓存侧视图）产出一致：同一份元数据的两种派生
        let via_dir = FileCacheManager::scan_output_dir(&root);
        assert_eq!(scan.files.len(), via_dir.len());
        for (a, b) in scan.files.iter().zip(via_dir.iter()) {
            assert_eq!(a.path, b.path, "路径集合应一致");
            assert_eq!(a.size, b.size, "文件大小应一致");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_output_dir_does_not_follow_symlink_or_junction() {
        // M7 回归：输出目录内指向目录外位置的链接不得入缓存——否则缓存会列出
        // 目录外文件路径（可被 delete/rename 消费的越界面）
        let root = scan_root("nolink");
        std::fs::create_dir_all(&root).unwrap();
        let outside = std::env::temp_dir().join(format!(
            "missevan-test-filecache-outside-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.m4a"), b"x").unwrap();
        std::fs::write(root.join("real.m4a"), b"x").unwrap();
        #[cfg(unix)]
        let link_ok = std::os::unix::fs::symlink(&outside, root.join("linked")).is_ok();
        #[cfg(windows)]
        let link_ok = std::os::windows::fs::symlink_dir(&outside, root.join("linked")).is_ok();
        if !link_ok {
            let _ = std::fs::remove_dir_all(&root);
            let _ = std::fs::remove_dir_all(&outside);
            return;
        }
        let files = FileCacheManager::scan_output_dir(&root);
        let names: Vec<String> = files.iter().map(|f| f.name.clone()).collect();
        assert_eq!(names, vec!["real.m4a"], "链接指向的目录外文件不得进入缓存");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn anchor_name_of_strips_only_numeric_room_suffix() {
        let root = scan_root("anchor-name");
        // 非数字后缀不剥离
        let p = root.join("主播名-abc/audio.m4a");
        assert_eq!(
            FileCacheManager::anchor_name_of(&p, &root),
            "主播名-abc"
        );
        // 数字后缀剥离
        let p2 = root.join("主播名-12345/audio.m4a");
        assert_eq!(FileCacheManager::anchor_name_of(&p2, &root), "主播名");
        // 根目录文件 → 空
        let p3 = root.join("audio.m4a");
        assert_eq!(FileCacheManager::anchor_name_of(&p3, &root), "");
        let _ = std::fs::remove_dir_all(&root);
    }

    fn sample_file(id: &str, path: &str) -> RecordingFile {
        RecordingFile {
            id: id.into(),
            name: format!("{}.m4a", id),
            path: path.into(),
            size: 1,
            duration: 1.0,
            anchor_name: "主播".into(),
            created_at: SystemTime::now(),
            group_prefix: None,
            segment_index: None,
            is_active: false,
        }
    }

    /// 录制中标记：路径键归一化（Windows 下 `\` 与 `/` 混用）后正确命中活跃输出路径
    #[test]
    fn mark_active_matches_normalized_paths() {
        let mut files = vec![
            sample_file("a", "D:/rec/主播-1/a.m4a"),          // 缓存侧 /
            sample_file("b", "D:\\rec\\主播-1\\b.m4a"),        // 缓存侧 \
            sample_file("c", "D:/rec/主播-1/c.m4a"),          // 非活跃
        ];
        let mut active = HashSet::new();
        active.insert(path_key("D:\\rec\\主播-1\\a.m4a")); // 引擎侧 \（归一化后入集）
        active.insert(path_key("D:/rec/主播-1/b.m4a"));    // 引擎侧 /
        mark_active(&mut files, &active);
        assert!(files[0].is_active);
        assert!(files[1].is_active);
        assert!(!files[2].is_active);
    }
}
