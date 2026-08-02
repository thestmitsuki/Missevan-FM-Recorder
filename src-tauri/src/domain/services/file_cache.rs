use crate::domain::config::manager::ConfigManager;
use crate::infrastructure::error::types::AppError;
use crate::infrastructure::state::app_state::AppStateHandle;
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
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
        let output_dir = Path::new(&config.global.output_dir);

        // 活跃录制输出路径（先取路径集再锁缓存，避免与命令侧锁顺序反转造成死锁）
        let active_paths = {
            let state = app_state.lock().await;
            state.active_output_paths()
        };

        let mut files = Self::scan_output_dir(output_dir);
        mark_active(&mut files, &active_paths);

        // 按创建时间降序排序
        files.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        // 构建分段组
        let mut group_map: HashMap<String, Vec<RecordingFile>> = HashMap::new();
        let mut non_grouped = Vec::new();
        for file in files {
            if let Some(ref prefix) = file.group_prefix {
                group_map.entry(prefix.clone()).or_default().push(file);
            } else {
                non_grouped.push(file);
            }
        }

        let mut groups = Vec::new();
        for (prefix, mut group_files) in group_map {
            group_files.sort_by_key(|f| f.segment_index.unwrap_or(0));
            let total_size: u64 = group_files.iter().map(|f| f.size).sum();
            let total_duration: f64 = group_files.iter().map(|f| f.duration).sum();
            groups.push(FileGroup {
                prefix,
                files: group_files,
                total_size,
                total_duration,
            });
        }

        // 更新缓存
        let mut cache = self.cache.lock().await;
        let files_before = cache.files.len();
        cache.files = non_grouped;
        cache.groups = groups;
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

        // 推送事件到前端
        let payload = serde_json::json!({
            "files": cache.files,
            "groups": cache.groups,
        });
        let _ = self.window.emit("recording_files_changed", &payload);

        Ok(())
    }

    /// 递归扫描输出目录，收集所有录制音频文件
    /// （扩展名 m4a/aac/mp3/flac，大小写不敏感；输出目录不存在 → 空列表）。
    /// 主播名从文件所在父目录名推导（录制引擎输出结构 `{主播名}-{房间号}` →
    /// 剥离房间号后缀；输出目录根下的文件主播名为空）。
    /// 与清理服务 cleanup.rs 的扫描策略保持一致：单文件元数据错误跳过不中断整次扫描。
    fn scan_output_dir(root: &Path) -> Vec<RecordingFile> {
        let mut out = Vec::new();
        if !root.exists() {
            return out;
        }
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                // M7：用 DirEntry::file_type()（不跟随链接）判定，拒绝 junction/
                // 符号链接项——避免缓存列出输出目录外的文件路径
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if file_type.is_dir() {
                    stack.push(path);
                } else if file_type.is_file() {
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        if matches!(ext.to_lowercase().as_str(), "m4a" | "aac" | "mp3" | "flac") {
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
                        }
                    }
                }
            }
        }
        out
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

    /// 解析文件名中的分段信息：前缀_001.ext
    fn parse_segment_info(filename: &str) -> (Option<String>, Option<u32>) {
        // 简单匹配：xxx_001.ext
        if let Some(pos) = filename.rfind("_") {
            let (base, idx_part) = filename.split_at(pos);
            let idx_str = &idx_part[1..]; // 去掉下划线
            if let Some(dot_pos) = idx_str.find('.') {
                let num_str = &idx_str[..dot_pos];
                if num_str.len() == 3 && num_str.chars().all(|c| c.is_ascii_digit()) {
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

    #[test]
    fn scan_output_dir_missing_dir_returns_empty() {
        let root = scan_root("missing");
        assert!(FileCacheManager::scan_output_dir(&root).is_empty());
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
