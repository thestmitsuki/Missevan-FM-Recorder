//! 录制工具（FFmpeg / ffprobe）可执行文件路径解析。
//!
//! 首启向导下载的便携版 FFmpeg 位于 `{exe_dir}/ffmpeg/`（见 `api::wizard_cmds::download_ffmpeg`）。
//! 所有消费方可执行文件路径的地方（健康检查候选、录制引擎 spawn）必须遵循同一候选顺序：
//! **配置指定路径（若非空）→ `{exe_dir}/ffmpeg/<工具>.exe`（若存在）→ PATH**，
//! 否则可能出现「配置里没写路径 → 找不到已下载的 FFmpeg」的漏匹配。

use std::path::PathBuf;

/// 可执行文件所在目录（与 lib.rs 中的 exe_dir 计算保持一致）
pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

/// FFmpeg 候选顺序：配置指定路径（若非空）→ `{exe_dir}/ffmpeg/ffmpeg.exe` → PATH（"ffmpeg"）
pub fn ffmpeg_candidates(config_ffmpeg_path: Option<&str>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(p) = config_ffmpeg_path {
        if !p.is_empty() {
            candidates.push(PathBuf::from(p));
        }
    }
    candidates.push(exe_dir().join("ffmpeg").join("ffmpeg.exe"));
    candidates.push(PathBuf::from("ffmpeg"));
    candidates
}

/// ffprobe 候选顺序：配置指定路径（若非空）→ `{exe_dir}/ffmpeg/ffprobe.exe` → PATH（"ffprobe"）
pub fn ffprobe_candidates(config_ffprobe_path: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if !config_ffprobe_path.is_empty() {
        candidates.push(PathBuf::from(config_ffprobe_path));
    }
    candidates.push(exe_dir().join("ffmpeg").join("ffprobe.exe"));
    candidates.push(PathBuf::from("ffprobe"));
    candidates
}

/// 解析实际用于 spawn 的 FFmpeg 可执行文件：候选顺序中第一个存在的文件；
/// 全部不存在（含仅配置了裸名、未安装任何版本）时退回 PATH 名 "ffmpeg"。
pub fn resolve_ffmpeg_executable(config_ffmpeg_path: Option<&str>) -> String {
    for cand in ffmpeg_candidates(config_ffmpeg_path) {
        if cand.exists() {
            return cand.to_string_lossy().into_owned();
        }
    }
    String::from("ffmpeg")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffmpeg_candidates_config_path_first_then_local_then_path() {
        let cands = ffmpeg_candidates(Some("C:\\tools\\ffmpeg.exe"));
        assert_eq!(cands.len(), 3);
        // 候选顺序：配置指定 → {exe_dir}/ffmpeg/ffmpeg.exe → PATH
        assert_eq!(cands[0], PathBuf::from("C:\\tools\\ffmpeg.exe"));
        assert_eq!(
            cands[1],
            exe_dir().join("ffmpeg").join("ffmpeg.exe")
        );
        assert_eq!(cands[2], PathBuf::from("ffmpeg"));
    }

    #[test]
    fn ffmpeg_candidates_skip_empty_config_path() {
        // 配置为空字符串 / None 时，候选从 {exe_dir}/ffmpeg/ffmpeg.exe 开始
        let cands = ffmpeg_candidates(None);
        assert_eq!(cands.len(), 2);
        assert_eq!(cands[0], exe_dir().join("ffmpeg").join("ffmpeg.exe"));
        assert_eq!(cands[1], PathBuf::from("ffmpeg"));

        let cands = ffmpeg_candidates(Some(""));
        assert_eq!(cands, vec![exe_dir().join("ffmpeg").join("ffmpeg.exe"), PathBuf::from("ffmpeg")]);
    }

    #[test]
    fn ffprobe_candidates_order_matches_ffmpeg() {
        let cands = ffprobe_candidates("C:\\tools\\ffprobe.exe");
        assert_eq!(cands.len(), 3);
        assert_eq!(cands[0], PathBuf::from("C:\\tools\\ffprobe.exe"));
        assert_eq!(cands[1], exe_dir().join("ffmpeg").join("ffprobe.exe"));
        assert_eq!(cands[2], PathBuf::from("ffprobe"));

        let cands = ffprobe_candidates("");
        assert_eq!(cands.len(), 2);
        assert_eq!(cands[0], exe_dir().join("ffmpeg").join("ffprobe.exe"));
        assert_eq!(cands[1], PathBuf::from("ffprobe"));
    }

    #[test]
    fn resolve_falls_back_to_path_when_nothing_installed() {
        // 测试环境下 {exe_dir}/ffmpeg/ffmpeg.exe 不存在，配置也空 → 退回 PATH 名
        assert_eq!(resolve_ffmpeg_executable(None), "ffmpeg");
        // 配置路径指向不存在的文件 → 同样退回
        let missing = std::env::temp_dir().join("missevan-recorder-test-missing-ffmpeg.exe");
        assert_eq!(
            resolve_ffmpeg_executable(Some(missing.to_str().unwrap())),
            "ffmpeg"
        );
    }
}
