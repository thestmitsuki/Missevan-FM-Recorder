//! 录制工具（FFmpeg / ffprobe）可执行文件路径解析。
//!
//! Windows 首启向导下载的便携版 FFmpeg 位于 `{exe_dir}/ffmpeg/`（见
//! `api::wizard_cmds::download_ffmpeg`）；Linux 不内置便携版（向导提示用系统包
//! 安装，见 `download_ffmpeg` 的 Linux 分支），该目录候选不会命中，仅保持候选
//! 顺序与 Windows 一致。
//! 所有消费方可执行文件路径的地方（健康检查候选、录制引擎 spawn）必须遵循同一候选顺序：
//! **配置指定路径（若非空）→ `{exe_dir}/ffmpeg/<工具>[.exe]`（若存在）→ PATH**，
//! 否则可能出现「配置里没写路径 → 找不到已下载的 FFmpeg」的漏匹配。

use std::path::PathBuf;

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

/// 后台回收子进程（M4 僵尸进程修复）：spawn 后把 `Child` 交给独立线程 `wait()`，
/// 子进程退出后线程随即结束并回收——Linux 上消除「spawn 后丢弃 Child 且不 wait」
/// 导致的 defunct 僵尸进程累积（7×24 运行可达数百个）；Windows 无僵尸概念，
/// 但同一路径无害且顺手回收进程句柄。调用方不阻塞：命令「启动即返回」，
/// 录制结束流程 / 浏览器打开互不等待（与「spawn 后不等待」的既有语义一致）。
///
/// 返回 `Option<JoinHandle>`：线程创建失败（极罕见）时返回 None，Child 被 drop
/// ——Linux 上该子进程退出后会短暂成为僵尸，进程退出时由 init 收养，不构成
/// 长期累积；正常路径（可创建线程）均回收。
///
/// 不持有 JoinHandle（drop 即分离）：回收线程是 std 线程，main 返回时进程
/// 整体退出（Rust 不 join 非主线程），不会因长命子进程阻塞应用退出。
pub fn reap_in_background(
    mut child: std::process::Child,
) -> Option<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("child-reaper".to_string())
        .spawn(move || {
            let _ = child.wait();
        })
        .ok()
}

/// 可执行文件所在目录（与 lib.rs 中的 exe_dir 计算保持一致）
pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

/// 平台相关工具文件名：Windows 为 `{name}.exe`，其他平台为 `{name}`（无扩展名）。
/// 便携版 FFmpeg 目录内 Linux 不存放文件（向导不下载，见 wizard_cmds::download_ffmpeg），
/// 该候选在 Linux 上通常不命中，但保持候选顺序与 Windows 一致
///（配置指定 → 便携目录 → PATH）。
fn tool_exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{}.exe", name)
    } else {
        name.to_string()
    }
}

/// FFmpeg 候选顺序：配置指定路径（若非空）→ `{exe_dir}/ffmpeg/ffmpeg[.exe]` → PATH（"ffmpeg"）
pub fn ffmpeg_candidates(config_ffmpeg_path: Option<&str>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(p) = config_ffmpeg_path {
        if !p.is_empty() {
            candidates.push(PathBuf::from(p));
        }
    }
    candidates.push(exe_dir().join("ffmpeg").join(tool_exe_name("ffmpeg")));
    candidates.push(PathBuf::from("ffmpeg"));
    candidates
}

/// ffprobe 候选顺序：配置指定路径（若非空）→ `{exe_dir}/ffmpeg/ffprobe[.exe]` → PATH（"ffprobe"）
pub fn ffprobe_candidates(config_ffprobe_path: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if !config_ffprobe_path.is_empty() {
        candidates.push(PathBuf::from(config_ffprobe_path));
    }
    candidates.push(exe_dir().join("ffmpeg").join(tool_exe_name("ffprobe")));
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
        // 候选顺序：配置指定 → {exe_dir}/ffmpeg/ffmpeg[.exe] → PATH
        assert_eq!(cands[0], PathBuf::from("C:\\tools\\ffmpeg.exe"));
        assert_eq!(
            cands[1],
            exe_dir().join("ffmpeg").join(tool_exe_name("ffmpeg"))
        );
        assert_eq!(cands[2], PathBuf::from("ffmpeg"));
    }

    #[test]
    fn ffmpeg_candidates_skip_empty_config_path() {
        // 配置为空字符串 / None 时，候选从 {exe_dir}/ffmpeg/ffmpeg[.exe] 开始
        let cands = ffmpeg_candidates(None);
        assert_eq!(cands.len(), 2);
        assert_eq!(
            cands[0],
            exe_dir().join("ffmpeg").join(tool_exe_name("ffmpeg"))
        );
        assert_eq!(cands[1], PathBuf::from("ffmpeg"));

        let cands = ffmpeg_candidates(Some(""));
        assert_eq!(
            cands,
            vec![
                exe_dir().join("ffmpeg").join(tool_exe_name("ffmpeg")),
                PathBuf::from("ffmpeg")
            ]
        );
    }

    #[test]
    fn ffprobe_candidates_order_matches_ffmpeg() {
        let cands = ffprobe_candidates("C:\\tools\\ffprobe.exe");
        assert_eq!(cands.len(), 3);
        assert_eq!(cands[0], PathBuf::from("C:\\tools\\ffprobe.exe"));
        assert_eq!(
            cands[1],
            exe_dir().join("ffmpeg").join(tool_exe_name("ffprobe"))
        );
        assert_eq!(cands[2], PathBuf::from("ffprobe"));

        let cands = ffprobe_candidates("");
        assert_eq!(cands.len(), 2);
        assert_eq!(
            cands[0],
            exe_dir().join("ffmpeg").join(tool_exe_name("ffprobe"))
        );
        assert_eq!(cands[1], PathBuf::from("ffprobe"));
    }

    /// 平台文件名区分：Windows 带 .exe 扩展名，其他平台无扩展名
    #[test]
    fn tool_exe_name_differs_by_platform() {
        let ffmpeg = tool_exe_name("ffmpeg");
        let ffprobe = tool_exe_name("ffprobe");
        if cfg!(windows) {
            assert_eq!(ffmpeg, "ffmpeg.exe");
            assert_eq!(ffprobe, "ffprobe.exe");
        } else {
            assert_eq!(ffmpeg, "ffmpeg");
            assert_eq!(ffprobe, "ffprobe");
        }
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
        // 测试环境下 {exe_dir}/ffmpeg/ffmpeg[.exe] 不存在，配置也空 → 退回 PATH 名
        assert_eq!(resolve_ffmpeg_executable(None), "ffmpeg");
        // 配置路径指向不存在的文件 → 同样退回
        let missing = std::env::temp_dir().join("missevan-recorder-test-missing-ffmpeg.exe");
        assert_eq!(
            resolve_ffmpeg_executable(Some(missing.to_str().unwrap())),
            "ffmpeg"
        );
    }

    // ── M4：后台回收子进程（防 Linux 僵尸进程累积）──

    #[test]
    fn reap_in_background_reaps_short_lived_child() {
        // 短命子进程（Windows cmd /c exit、其他平台 true）秒退：回收线程
        // wait() 返回后线程结束——join 成功即证明「spawn → 后台 wait」路径
        // 可走通且不 panic（子进程句柄被回收，非泄漏）。
        // 僵尸进程消失本身需 Arch 真机验证（ps 查 defunct）；Windows 无僵尸
        // 概念，本测试仅验证代码路径可用。
        #[cfg(windows)]
        let mut cmd = std::process::Command::new("cmd");
        #[cfg(windows)]
        cmd.args(["/C", "exit /b 0"]);
        #[cfg(not(windows))]
        let mut cmd = std::process::Command::new("true");
        let child = cmd.spawn().expect("spawn 测试子进程失败");
        let handle = reap_in_background(child).expect("回收线程创建失败");
        handle.join().expect("回收线程不应 panic");
    }
}
