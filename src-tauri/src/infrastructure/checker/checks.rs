use async_trait::async_trait;

use crate::tr;
use super::report::CheckResult;
use crate::infrastructure::checker::report::CheckStatus;

/// 健康检查项 trait——所有检查项实现此接口
#[async_trait]
pub trait HealthCheck: Send + Sync {
    fn name(&self) -> &'static str;
    async fn run(&self) -> CheckResult;
}

/// FFmpeg 存在性检查
pub struct FfmpegCheck {
    pub ffmpeg_path: Option<String>,
}

/// FFmpeg 候选准备：tools.rs 候选顺序（配置指定路径 → `{exe_dir}/ffmpeg/ffmpeg.exe`
/// → PATH 裸名）+ 逐候选 `clean_path` 清洗。与录制引擎 spawn（resolve_ffmpeg_executable）
/// 及 run_wizard_health_check / get_debug_info 的候选口径完全一致。
fn prepared_candidates(config_ffmpeg_path: Option<&str>) -> Vec<std::path::PathBuf> {
    crate::domain::tools::ffmpeg_candidates(config_ffmpeg_path)
        .into_iter()
        .map(|c| {
            let s = clean_path(&c.to_string_lossy());
            if s.is_empty() {
                c
            } else {
                std::path::PathBuf::from(s)
            }
        })
        .collect()
}

/// 清洗路径：移除不可见控制字符与 bidi 格式字符（U+200E/U+200F/U+202A–U+202E/
/// U+2066–U+2069），**保留空白字符**——路径中的空格（如 `C:\Program Files`）合法，
/// 此前误剥全部空白导致含空格路径 `exists()` 失败、健康检查假阴性
/// （Task 20 Important-1 回归修复）。
fn clean_path(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(
                    *c,
                    // bidi 格式字符（Unicode Cf 类，is_control() 不覆盖）：
                    // U+200E/U+200F LTR/RTL mark；U+202A–U+202E 嵌入/覆盖/弹出；
                    // U+2066–U+2069 隔离符（LRI/RLI/FSI/PDI）
                    '\u{200e}'
                        | '\u{200f}'
                        | '\u{202a}'
                        | '\u{202b}'
                        | '\u{202c}'
                        | '\u{202d}'
                        | '\u{202e}'
                        | '\u{2066}'
                        | '\u{2067}'
                        | '\u{2068}'
                        | '\u{2069}'
                )
        })
        .collect()
}

#[async_trait]
impl HealthCheck for FfmpegCheck {
    fn name(&self) -> &'static str {
        tr!("debug.ffmpeg_check_name")
    }

    async fn run(&self) -> CheckResult {
        let start = std::time::Instant::now();

        // 发布前修复（与 run_wizard_health_check / get_debug_info::probe_tool 同语义）：
        // 候选顺序 = tools.rs（配置指定路径（清洗后）→ `{exe_dir}/ffmpeg/ffmpeg.exe`（首启
        // 向导下载结果）→ PATH 裸名）。此前实现先 `c.exists()` 再试运行——裸名候选
        // （"ffmpeg"，≤1 个路径分量）相对 cwd 的 exists() 为 false 不代表 PATH 不可用，
        // 导致「录制引擎能 spawn（resolve_ffmpeg_executable 退回 PATH 名）、健康检查却
        // 报失败」的漏匹配。修复：裸名候选跳过 exists() gate，直接 `-version` 试运行
        // 验证；逐候选尝试，任一命中即 Passed。
        let candidates = prepared_candidates(self.ffmpeg_path.as_deref());
        let checked = candidates
            .iter()
            .map(|c| c.display().to_string())
            .collect::<Vec<_>>();

        let mut last_failure: Option<String> = None;
        for path in &candidates {
            let is_bare = path.components().count() <= 1;
            if !is_bare && !path.exists() {
                continue;
            }
            // 试运行 `-version` 验证可执行性；5s 超时防损坏/挂起的可执行文件卡住
            // 整个检查（与 debug_cmds::probe_tool 的 TOOL_PROBE_TIMEOUT 一致）。
            // 隐藏控制台（tools.rs::apply_create_no_window）：健康检查 spawn 的
            // ffmpeg 是控制台子系统，发布构建无控制台时会弹黑窗口
            let mut probe = tokio::process::Command::new(path);
            probe.arg("-version");
            #[cfg(windows)]
            crate::domain::tools::apply_create_no_window(probe.as_std_mut());
            let probe = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                probe.output(),
            )
            .await;
            match probe {
                Ok(Ok(output)) if output.status.success() => {
                    // 成功执行，FFmpeg 可用
                    let version = String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .next()
                        .unwrap_or("")
                        .to_string();
                    return CheckResult {
                        check_name: self.name().to_string(),
                        status: CheckStatus::Passed,
                        message: tr!(
                            "wizard.tool_available",
                            name = "FFmpeg",
                            version = version
                        ),
                        details: Some(tr!("wizard.tool_path", path = path.display())),
                        suggestion: None,
                        duration_ms: start.elapsed().as_millis() as u64,
                    };
                }
                Ok(Ok(output)) => {
                    // 候选存在但执行失败（非零退出码）——继续尝试下一个
                    last_failure = Some(tr!(
                        "debug.ffmpeg_exec_failed",
                        path = path.display(),
                        code = output.status.code().unwrap_or(-1)
                    ));
                }
                Ok(Err(e)) => {
                    // 无法启动进程（如权限不足）——继续尝试下一个
                    last_failure = Some(tr!(
                        "debug.ffmpeg_not_executable",
                        path = path.display(),
                        err = e
                    ));
                }
                Err(_) => {
                    // 试运行超时（可执行文件挂起）——继续尝试下一个
                    last_failure = Some(tr!(
                        "debug.ffmpeg_probe_timeout",
                        path = path.display()
                    ));
                }
            }
        }

        let details = match &last_failure {
            Some(f) => tr!(
                "debug.candidates_checked_failed",
                candidates = checked.join("、"),
                failure = f
            ),
            None => tr!(
                "debug.candidates_checked",
                candidates = checked.join("、")
            ),
        };
        CheckResult {
            check_name: self.name().to_string(),
            status: CheckStatus::Failed,
            message: tr!("debug.ffmpeg_not_found").to_string(),
            details: Some(details),
            suggestion: Some(tr!("debug.ffmpeg_not_found_suggestion").into()),
            duration_ms: start.elapsed().as_millis() as u64,
        }
    }
}

/// 磁盘空间检查
pub struct DiskSpaceCheck {
    pub output_dir: String,
    pub threshold_gb: u64,
}

#[async_trait]
impl HealthCheck for DiskSpaceCheck {
    fn name(&self) -> &'static str {
        tr!("debug.disk_space")
    }

    async fn run(&self) -> CheckResult {
        let start = std::time::Instant::now();

        match fs2::available_space(std::path::Path::new(&self.output_dir)) {
            Ok(available) => {
                let available_gb = available / (1024 * 1024 * 1024);
                if available_gb < self.threshold_gb {
                    CheckResult {
                        check_name: self.name().to_string(),
                        status: super::report::CheckStatus::Failed,
                        message: tr!(
                            "debug.disk_low",
                            available = available_gb,
                            threshold = self.threshold_gb
                        ),
                        details: None,
                        suggestion: Some(tr!("debug.disk_low_suggestion").into()),
                        duration_ms: start.elapsed().as_millis() as u64,
                    }
                } else {
                    CheckResult {
                        check_name: self.name().to_string(),
                        status: super::report::CheckStatus::Passed,
                        message: tr!("debug.disk_ok", available = available_gb),
                        details: None,
                        suggestion: None,
                        duration_ms: start.elapsed().as_millis() as u64,
                    }
                }
            }
            Err(e) => CheckResult {
                check_name: self.name().to_string(),
                status: super::report::CheckStatus::Warning,
                message: tr!("debug.disk_check_failed", err = e),
                details: None,
                suggestion: None,
                duration_ms: start.elapsed().as_millis() as u64,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_candidates_order_matches_tools_and_cleans_paths() {
        // 发布前修复回归：候选顺序与 domain::tools 完全一致
        //（配置指定路径（清洗后）→ {exe_dir}/ffmpeg/ffmpeg.exe → PATH 裸名）
        let cands = prepared_candidates(Some("C:\\tools\\ff\u{7}mpeg.exe"));
        assert_eq!(cands.len(), 3);
        assert_eq!(cands[0], std::path::PathBuf::from("C:\\tools\\ffmpeg.exe"));
        assert_eq!(
            cands[1],
            crate::domain::tools::exe_dir().join("ffmpeg").join("ffmpeg.exe")
        );
        assert_eq!(cands[2], std::path::PathBuf::from("ffmpeg"));
        // 空配置 → 从 {exe_dir}/ffmpeg/ffmpeg.exe 开始
        let cands = prepared_candidates(None);
        assert_eq!(cands.len(), 2);
        assert_eq!(
            cands[0],
            crate::domain::tools::exe_dir().join("ffmpeg").join("ffmpeg.exe")
        );
        assert_eq!(cands[1], std::path::PathBuf::from("ffmpeg"));
    }

    #[test]
    fn clean_path_keeps_spaces_in_windows_paths() {
        // Task 20 Important-1 回归：含空格路径不得被破坏（此前空白全剥 →
        // `C:\Program Files\...` 变 `C:\ProgramFiles\...` → exists() 失败 → 假阴性）
        assert_eq!(
            clean_path(r"C:\Program Files\ffmpeg\ffmpeg.exe"),
            r"C:\Program Files\ffmpeg\ffmpeg.exe"
        );
        assert_eq!(clean_path("D:/my recordings/live"), "D:/my recordings/live");
    }

    #[test]
    fn clean_path_strips_control_and_bidi_chars() {
        // 控制字符（含 \t \n \r）仍被剥离
        assert_eq!(clean_path("C:\u{7}ffmpeg\u{0}bin"), "C:ffmpegbin");
        // bidi 格式字符仍被剥离：LTR/RTL mark（U+200E/U+200F）、
        // 嵌入/覆盖/弹出（U+202A–U+202E）、隔离符（U+2066–U+2069）
        assert_eq!(clean_path("a\u{200e}b\u{200f}c\u{202a}d\u{202e}e"), "abcde");
        assert_eq!(clean_path("f\u{2066}g\u{2069}h"), "fgh");
    }
}
