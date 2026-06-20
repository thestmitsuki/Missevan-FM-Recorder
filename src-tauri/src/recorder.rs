use anyhow::{Context, Result};
use chrono::Local;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::config::model::{Config, RecordFormat};
use crate::spider;
use crate::utils::clean_filename;

const FFMPEG_RESTART_DELAY: Duration = Duration::from_secs(5);
const MAX_RETRIES: usize = 3;

/// 开始录制直播流（支持自动重连，anchor_id 用于文件命名前缀）
pub async fn start_recording(
    config: &Config,
    anchor_name: &str,
    title: Option<&str>,
    ffmpeg_path: &PathBuf,
    cancel_token: CancellationToken,
    anchor_id: &str, // 新增参数
) -> Result<()> {
    let mut retry_count = 0;
    loop {
        let result = run_single_recording(
            config,
            anchor_name,
            title,
            ffmpeg_path,
            &cancel_token,
            anchor_id,
        )
        .await;
        match result {
            Ok(_) => {
                info!("录制正常结束 (主播 {})", anchor_name);
                return Ok(());
            }
            Err(e) => {
                if cancel_token.is_cancelled() {
                    info!("录制因取消信号而终止 (主播 {})", anchor_name);
                    return Ok(());
                }
                retry_count += 1;
                if retry_count > MAX_RETRIES {
                    error!("录制失败次数过多，放弃重试 (主播 {}): {}", anchor_name, e);
                    return Err(e);
                }
                warn!(
                    "录制进程崩溃，{:.1}秒后重试 ({}/{}) (主播 {}): {}",
                    FFMPEG_RESTART_DELAY.as_secs_f32(),
                    retry_count,
                    MAX_RETRIES,
                    anchor_name,
                    e
                );
                sleep(FFMPEG_RESTART_DELAY).await;
            }
        }
    }
}

async fn run_single_recording(
    config: &Config,
    anchor_name: &str,
    title: Option<&str>,
    ffmpeg_path: &PathBuf,
    cancel_token: &CancellationToken,
    anchor_id: &str,
) -> Result<()> {
    // 先查找主播配置（仅一次）
    let anchor_cfg = config
        .anchors
        .iter()
        .find(|a| a.id == anchor_id)
        .ok_or_else(|| anyhow::anyhow!("主播配置不存在 (id: {})", anchor_id))?;

    // 获取流地址（带重试）
    let mut stream_url = None;
    for attempt in 1..=3 {
        match spider::get_live_info(
            &anchor_cfg.url,
            anchor_cfg.proxy.as_deref(),
            anchor_cfg.cookie.as_deref(),
        )
        .await
        {
            Ok(info) => {
                if let Some(url) = info.stream_url {
                    stream_url = Some(url);
                    break;
                }
            }
            Err(e) => warn!("获取流地址失败 (尝试 {}/3): {}", attempt, e),
        }
        sleep(Duration::from_secs(3)).await;
    }
    let stream_url = stream_url.context("未获取到直播流地址")?;

    let now = Local::now();
    let date_str = now.format("%Y-%m-%d_%H-%M-%S").to_string();
    let safe_name = clean_filename(anchor_name);
    let title_part = title.unwrap_or("").to_string();
    let title_part = if !title_part.is_empty() {
        clean_filename(&title_part) + "_"
    } else {
        String::new()
    };

    // 按主播分类子目录
    let anchor_dir = PathBuf::from(&config.output_dir)
        .join("猫耳FM直播")
        .join(&safe_name);
    fs::create_dir_all(&anchor_dir)?;

    let extension = match config.record_format {
        RecordFormat::MP3 => "mp3",
        RecordFormat::M4A => "m4a",
    };
    let base_filename = format!("{}_{}{}", safe_name, title_part, date_str);
    let output_pattern = if config.segment_seconds > 0 {
        anchor_dir.join(format!("{}_%03d.{}", base_filename, extension))
    } else {
        anchor_dir.join(format!("{}.{}", base_filename, extension))
    };

    let mut cmd = Command::new(ffmpeg_path);
    cmd.arg("-y")
        .arg("-loglevel")
        .arg("error")
        .arg("-i")
        .arg(&stream_url)
        .arg("-vn")
        .arg("-c:a")
        .arg(if extension == "mp3" { "libmp3lame" } else { "aac" })
        .arg("-b:a")
        .arg("320k");

    if config.segment_seconds > 0 {
        cmd.arg("-f")
            .arg("segment")
            .arg("-segment_time")
            .arg(config.segment_seconds.to_string())
            .arg("-reset_timestamps")
            .arg("1");
        cmd.arg("-segment_format").arg(if extension == "m4a" { "mpegts" } else { "mp3" });
        let path_str = output_pattern
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("输出路径包含非 UTF-8 字符"))?;
        cmd.arg(path_str);
    } else {
        if extension == "m4a" {
            cmd.arg("-bsf:a").arg("aac_adtstoasc").arg("-movflags").arg("+faststart");
        }
        let path_str = output_pattern
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("输出路径包含非 UTF-8 字符"))?;
        cmd.arg(path_str);
    }

    info!("开始录制: {} (主播: {})", base_filename, anchor_name);
    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .spawn()
        .context("无法启动 ffmpeg")?;

    let check_interval = Duration::from_secs(10);

    let mut ffmpeg_exited = false;

    loop {
        tokio::select! {
            _ = sleep(check_interval) => {
                if let Err(e) = crate::utils::check_disk_space(&config.output_dir, config.disk_space_limit_gb) {
                    error!("磁盘空间不足，终止录制: {}", e);
                    break;
                }
                match spider::get_live_info(
                    &anchor_cfg.url,
                    anchor_cfg.proxy.as_deref(),
                    anchor_cfg.cookie.as_deref(),
                ).await {
                    Ok(info) => {
                        if !info.is_live {
                            info!("直播已结束，停止录制...");
                            break;
                        }
                    }
                    Err(e) => warn!("检测直播状态失败：{}，继续录制", e),
                }
            }
            _ = cancel_token.cancelled() => {
                info!("收到取消信号，结束录制...");
                break;
            }
            result = child.wait() => {
                ffmpeg_exited = true;
                match result {
                    Ok(status) => {
                        if !status.success() {
                            anyhow::bail!("ffmpeg 退出码: {}", status);
                        }
                        break;
                    }
                    Err(e) => anyhow::bail!("等待 ffmpeg 失败: {}", e),
                }
            }
        }
    }

    // ffmpeg 可能已自行退出，只有仍在运行时才发 q 信号
    if !ffmpeg_exited {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(b"q").await;
            let _ = stdin.flush().await;
        }
        let _ = child.wait().await;
    }

    info!("录制结束: {} (主播: {})", base_filename, anchor_name);
    Ok(())
}
