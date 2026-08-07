//! 录制工具（FFmpeg / ffprobe）可执行文件路径解析。
//!
//! 首启向导下载的便携版 FFmpeg 位于 `{exe_dir}/ffmpeg/`（见 `api::wizard_cmds::download_ffmpeg`）。
//! 所有消费方可执行文件路径的地方（健康检查候选、录制引擎 spawn）必须遵循同一候选顺序：
//! **配置指定路径（若非空）→ `{exe_dir}/ffmpeg/<工具>.exe`（若存在）→ PATH**，
//! 否则可能出现「配置里没写路径 → 找不到已下载的 FFmpeg」的漏匹配。

use std::path::{Path, PathBuf};

use crate::infrastructure::error::types::AppError;

/// 用资源管理器打开/选中路径（文件或目录，Windows：`explorer /select,{path}`；
/// explorer 是 GUI 程序，spawn 后立即返回）。供 `open_output_dir` 命令、
/// 托盘「最近录制」菜单与录制后动作（post_record_action=open_folder）复用。
#[cfg(windows)]
pub fn open_in_explorer(path: &Path) -> Result<(), AppError> {
    std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn()
        .map_err(|e| {
            AppError::system(
                crate::infrastructure::error::types::INT_UNEXPECTED,
                "打开资源管理器失败",
            )
            .with_technical(e.to_string())
        })?;
    Ok(())
}

/// 非 Windows 平台：暂不支持
#[cfg(not(windows))]
pub fn open_in_explorer(_path: &Path) -> Result<(), AppError> {
    Err(AppError::internal("当前平台不支持打开资源管理器"))
}

/// Windows：为子进程设置 `CREATE_NO_WINDOW`（0x08000000），禁止新建控制台窗口。
///
/// 发布构建 `windows_subsystem = "windows"`（main.rs）父进程无控制台；此时 spawn
/// 控制台子系统子进程（ffmpeg / ffprobe / cmd）且未设此标志，Windows 会为子进程
/// **新建一个控制台窗口**（黑窗口闪现，子进程退出即消失）。录制引擎已内置同款
/// 处理（recorder/builder.rs）；本辅助供工具探测（debug_cmds / wizard_cmds /
/// checks）与录制后命令（monitor.rs）共用。tokio 命令调用方传入
/// `cmd.as_std_mut()`（tokio 的 Command 包装 std Command，spawn 时生效）。
///
/// 取舍说明：`CREATE_NO_WINDOW` 只影响**控制台窗口**的创建——`cmd /C <用户命令>`
/// 的控制台被隐藏，但用户命令里启动的 GUI 程序窗口仍正常显示（GUI 子系统进程
/// 本就不创建控制台）。
#[cfg(windows)]
pub fn apply_create_no_window(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

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

    /// Windows：应用 CREATE_NO_WINDOW 后进程仍可正常 spawn 并执行
    ///（控制台被隐藏不影响启动，仅丢弃控制台输出——`/C exit /b 0` 无输出）。
    #[cfg(windows)]
    #[test]
    fn apply_create_no_window_keeps_spawn_working() {
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/C", "exit /b 0"]);
        apply_create_no_window(&mut cmd);
        let status = cmd.status().expect("spawn 不应失败");
        assert!(status.success());
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
