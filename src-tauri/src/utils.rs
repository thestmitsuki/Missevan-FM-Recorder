use anyhow::{bail, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::env;
use std::path::{Path, PathBuf};
use sysinfo::Disks;
use tokio::process::Command;
use tracing::warn;
use which::which;

// 预编译正则表达式
static ILLEGAL_CHARS_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"[\\/:*?"<>|&#.。,， ~！· ]"#).unwrap());
static EMOJI_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[\p{Emoji}]").unwrap());

/// 清理文件名中的非法字符和表情符号
pub fn clean_filename(name: &str) -> String {
    let cleaned = ILLEGAL_CHARS_RE.replace_all(name, "_").to_string();
    let cleaned = cleaned.replace("（", "(").replace("）", ")");
    let cleaned = EMOJI_RE.replace_all(&cleaned, "").to_string();
    if cleaned.trim().is_empty() {
        "未知主播".to_string()
    } else {
        cleaned
    }
}

/// 检查磁盘剩余空间是否低于阈值（返回 Ok 表示空间足够，Err 表示不足）
pub fn check_disk_space(path: &str, min_gb: f64) -> Result<()> {
    let disks = Disks::new_with_refreshed_list();
    let target_path = Path::new(path);
    for disk in &disks {
        let mount_point = disk.mount_point();
        if target_path.starts_with(mount_point) {
            let available_gb = disk.available_space() as f64 / (1024.0 * 1024.0 * 1024.0);
            if available_gb < min_gb {
                bail!(
                    "磁盘剩余空间 {:.2} GB 低于阈值 {:.2} GB",
                    available_gb,
                    min_gb
                );
            }
            return Ok(());
        }
    }
    // 回退：尝试获取当前目录所在磁盘的空间
    if let Ok(current_dir) = env::current_dir() {
        if let Some(disk) = disks
            .iter()
            .find(|d| current_dir.starts_with(d.mount_point()))
        {
            let available_gb = disk.available_space() as f64 / (1024.0 * 1024.0 * 1024.0);
            if available_gb < min_gb {
                bail!(
                    "磁盘剩余空间 {:.2} GB 低于阈值 {:.2} GB",
                    available_gb,
                    min_gb
                );
            }
        }
    }
    Ok(())
}

/// 检测可用的 FFmpeg，支持自定义路径、本地文件夹、系统 PATH
pub async fn check_ffmpeg(custom_path: Option<&str>) -> Result<PathBuf> {
    let mut candidates = Vec::new();

    // 1. 自定义路径
    if let Some(path) = custom_path {
        let p = PathBuf::from(path);
        if p.exists() {
            candidates.push(p);
        } else {
            warn!("配置的 ffmpeg_path 不存在: {}", p.display());
        }
    }

    // 2. 程序同级目录下的 ffmpeg 文件夹
    let current_exe = env::current_exe()?;
    let base_dir = current_exe.parent().context("无法获取可执行文件目录")?;
    let local_ffmpeg = if cfg!(windows) {
        base_dir.join("ffmpeg").join("ffmpeg.exe")
    } else {
        base_dir.join("ffmpeg").join("ffmpeg")
    };
    if local_ffmpeg.exists() {
        candidates.push(local_ffmpeg);
    }

    // 3. 系统 PATH
    if let Ok(system_ffmpeg) = which("ffmpeg") {
        if system_ffmpeg.exists() {
            candidates.push(system_ffmpeg);
        }
    }

    candidates.dedup();

    for candidate in candidates {
        if test_ffmpeg_exec(&candidate).await {
            tracing::info!("使用 FFmpeg: {}", candidate.display());
            return Ok(candidate);
        }
    }

    bail!(
        "未找到可用的 FFmpeg。请将 ffmpeg 放置在以下位置之一：\n  1. 配置文件 ffmpeg_path 指定的路径\n  2. 程序所在目录下的 ffmpeg/ 文件夹内\n  3. 系统 PATH 环境变量中"
    )
}

async fn test_ffmpeg_exec<P: AsRef<Path>>(path: P) -> bool {
    match Command::new(path.as_ref()).arg("-version").output().await {
        Ok(out) => out.status.success(),
        Err(_) => false,
    }
}
