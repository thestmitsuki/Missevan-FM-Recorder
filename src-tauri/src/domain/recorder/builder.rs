use crate::domain::config::model::GlobalConfig;

/// FFmpeg 命令行构建器（Builder 模式）
pub struct FfmpegCommandBuilder {
    ffmpeg_path: String,
    input_url: String,
    output_path: String,
    format: String,
    segment_seconds: u64,
    /// 仅录制音频流（true = 传 `-vn`；false = 保留视频流）
    audio_only: bool,
    /// 音频编码码率 kbps（0 = 不传，由 ffmpeg 默认；Task 20 收尾接线 bitrate_kbps）
    bitrate_kbps: u32,
    /// 流媒体读超时秒数（0 = 不传 `-rw_timeout`；>0 → 传微秒值，
    /// 适用 HTTP/flv/m3u8 拉流——网络中断超过该时长时 ffmpeg 报错退出）
    stream_timeout_secs: u32,
}

impl FfmpegCommandBuilder {
    pub fn new() -> Self {
        Self {
            ffmpeg_path: String::from("ffmpeg"),
            input_url: String::new(),
            output_path: String::new(),
            format: String::from("m4a"),
            segment_seconds: 0,
            audio_only: true,
            bitrate_kbps: 0,
            stream_timeout_secs: 0,
        }
    }

    pub fn ffmpeg_path(mut self, path: &str) -> Self {
        self.ffmpeg_path = path.to_string();
        self
    }

    pub fn input_url(mut self, url: &str) -> Self {
        self.input_url = url.to_string();
        self
    }

    pub fn output_path(mut self, path: &str) -> Self {
        self.output_path = path.to_string();
        self
    }

    pub fn format(mut self, fmt: &str) -> Self {
        self.format = fmt.to_string();
        self
    }

    pub fn segment_seconds(mut self, secs: u64) -> Self {
        self.segment_seconds = secs;
        self
    }

    pub fn audio_only(mut self, on: bool) -> Self {
        self.audio_only = on;
        self
    }

    pub fn bitrate_kbps(mut self, kbps: u32) -> Self {
        self.bitrate_kbps = kbps;
        self
    }

    pub fn stream_timeout_secs(mut self, secs: u32) -> Self {
        self.stream_timeout_secs = secs;
        self
    }

    /// 构建 tokio::process::Command
    pub fn build(&self) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(&self.ffmpeg_path);
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());

        cmd.arg("-y"); // 覆盖已有文件

        // 流媒体读超时（stream_timeout_secs → `-rw_timeout` 微秒）：HTTP/flv/m3u8
        // 拉流下网络中断超过该时长，ffmpeg 报错退出（monitor 统一收尾），
        // 避免进程悬挂。0 = 不传（保持 ffmpeg 默认阻塞行为）。
        if self.stream_timeout_secs > 0 {
            cmd.arg("-rw_timeout")
                .arg((self.stream_timeout_secs as u64 * 1_000_000).to_string());
        }

        cmd.arg("-i").arg(&self.input_url);

        // audio_only=false 时不传 -vn（保留视频流；默认 true 与原行为一致）
        if self.audio_only {
            cmd.arg("-vn");
        }

        match self.format.as_str() {
            "mp3" => {
                cmd.arg("-c:a").arg("libmp3lame");
                if self.bitrate_kbps > 0 {
                    cmd.arg("-b:a").arg(format!("{}k", self.bitrate_kbps));
                } else {
                    cmd.arg("-q:a").arg("2"); // VBR 质量档兜底
                }
            }
            _ => {
                // m4a (AAC)
                cmd.arg("-c:a").arg("aac");
                if self.bitrate_kbps > 0 {
                    cmd.arg("-b:a").arg(format!("{}k", self.bitrate_kbps));
                }
            }
        }

        // 分段录制（M4：输出 pattern 必须带真实扩展名——`{prefix}_%03d.{ext}`，
        // 否则生成 `xxx_000.{}` 字面名，文件缓存/清理服务的扩展名白名单扫不到）。
        // L3/M1：`%03d` 是 printf「最小宽度 3、零填充」语义——段号 ≥1000 时
        // ffmpeg 自动扩展为 4 位（`_0999` → `_1000`），与 file_cache 的
        // 「≥3 位按需扩展」解析规则对齐；**不要**改成 `%05d`——那会改变
        // <1000 段的既有命名（`_001` → `_00001`），破坏文件归组/清理的向后兼容。
        if self.segment_seconds > 0 {
            cmd.arg("-f").arg("segment");
            cmd.arg("-segment_time")
                .arg(self.segment_seconds.to_string());
            cmd.arg("-reset_timestamps").arg("1");
            cmd.arg(format!(
                "{}_{}.{}",
                self.output_path, "%03d", self.format
            ));
        } else {
            cmd.arg(&self.output_path);
        }

        #[cfg(target_os = "windows")]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        cmd
    }

    /// 根据全局配置构建命令
    ///
    /// FFmpeg 可执行文件按候选顺序解析（与 `run_wizard_health_check` / `domain::tools` 一致）：
    /// 配置指定路径（若非空且存在）→ `{exe_dir}/ffmpeg/ffmpeg.exe`（首启向导下载结果）→ PATH。
    /// 若只认配置里的路径，首次运行下载的便携版 FFmpeg 在配置未落盘/被覆盖时会找不到。
    pub fn from_config(config: &GlobalConfig, stream_url: &str, output_path: &str) -> Self {
        let path = crate::domain::tools::resolve_ffmpeg_executable(config.ffmpeg_path.as_deref());
        Self::new()
            .ffmpeg_path(&path)
            .input_url(stream_url)
            .output_path(output_path)
            .format(&config.record_format)
            .segment_seconds(config.segment_seconds)
            // Task 20 收尾接线：audio_only → -vn 参数；bitrate_kbps → 音频编码码率
            .audio_only(config.audio_only)
            .bitrate_kbps(config.bitrate_kbps)
            // 网络分类：stream_timeout_secs → -rw_timeout（微秒；0 = 不传）
            .stream_timeout_secs(config.stream_timeout_secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::config::model::GlobalConfig;

    #[test]
    fn from_config_resolves_ffmpeg_path_with_fallback() {
        // 配置未设置 ffmpeg_path、本地也没有 {exe_dir}/ffmpeg/ffmpeg.exe 时，退回 PATH 名
        let config = GlobalConfig::default();
        let builder = FfmpegCommandBuilder::from_config(&config, "http://x", "out.m4a");
        assert_eq!(builder.ffmpeg_path, "ffmpeg");
    }

    #[test]
    fn from_config_wires_audio_only_and_bitrate() {
        let config = GlobalConfig::default(); // audio_only=true, bitrate_kbps=128
        let builder = FfmpegCommandBuilder::from_config(&config, "http://x", "out.m4a");
        assert!(builder.audio_only);
        assert_eq!(builder.bitrate_kbps, 128);

        let mut config2 = GlobalConfig::default();
        config2.audio_only = false;
        config2.bitrate_kbps = 256;
        let builder2 = FfmpegCommandBuilder::from_config(&config2, "http://x", "out.m4a");
        assert!(!builder2.audio_only);
        assert_eq!(builder2.bitrate_kbps, 256);
    }

    #[test]
    fn build_passes_vn_only_when_audio_only() {
        let args: Vec<String> = FfmpegCommandBuilder::new()
            .input_url("http://x")
            .output_path("out.m4a")
            .format("m4a")
            .build()
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"-vn".to_string()));

        let args2: Vec<String> = FfmpegCommandBuilder::new()
            .input_url("http://x")
            .output_path("out.m4a")
            .format("m4a")
            .audio_only(false)
            .build()
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(!args2.contains(&"-vn".to_string()));
    }

    #[test]
    fn build_applies_bitrate_to_aac_and_mp3() {
        let args: Vec<String> = FfmpegCommandBuilder::new()
            .input_url("http://x")
            .output_path("out.m4a")
            .format("m4a")
            .bitrate_kbps(192)
            .build()
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"-b:a".to_string()));
        assert!(args.contains(&"192k".to_string()));

        let args2: Vec<String> = FfmpegCommandBuilder::new()
            .input_url("http://x")
            .output_path("out.mp3")
            .format("mp3")
            .bitrate_kbps(128)
            .build()
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args2.contains(&"128k".to_string()));
    }

    #[test]
    fn build_segment_output_has_real_extension() {
        // M4 回归：分段 pattern 必须形如 `out_%03d.m4a`，而非字面 `out_%03d.{}`
        let args: Vec<String> = FfmpegCommandBuilder::new()
            .input_url("http://x")
            .output_path("D:/rec/主播A_20260801")
            .format("m4a")
            .segment_seconds(600)
            .build()
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(
            args.contains(&"D:/rec/主播A_20260801_%03d.m4a".to_string()),
            "分段输出必须带真实扩展名，实际: {:?}",
            args
        );
        assert!(
            !args.iter().any(|a| a.contains("{}")),
            "pattern 不得含字面 {{}}: {:?}",
            args
        );

        // mp3 同理
        let args2: Vec<String> = FfmpegCommandBuilder::new()
            .input_url("http://x")
            .output_path("out")
            .format("mp3")
            .segment_seconds(60)
            .build()
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args2.contains(&"out_%03d.mp3".to_string()));

        // 非分段模式保持原输出路径（不带 _%03d）
        let args3: Vec<String> = FfmpegCommandBuilder::new()
            .input_url("http://x")
            .output_path("out.m4a")
            .format("m4a")
            .build()
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args3.contains(&"out.m4a".to_string()));
        assert!(!args3.iter().any(|a| a.contains("%03d")));
    }

    #[test]
    fn build_applies_stream_timeout_as_rw_timeout_microseconds() {
        let args: Vec<String> = FfmpegCommandBuilder::new()
            .input_url("http://x/live.flv")
            .output_path("out.m4a")
            .format("m4a")
            .stream_timeout_secs(30)
            .build()
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"-rw_timeout".to_string()));
        assert!(
            args.contains(&"30000000".to_string()),
            "30s 应换算为 30_000_000 微秒: {:?}",
            args
        );
    }

    #[test]
    fn build_omits_rw_timeout_when_zero() {
        // 默认 0 → 不传 -rw_timeout（保持 ffmpeg 默认行为）
        let args: Vec<String> = FfmpegCommandBuilder::new()
            .input_url("http://x/live.flv")
            .output_path("out.m4a")
            .format("m4a")
            .build()
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(!args.contains(&"-rw_timeout".to_string()));
    }

    #[test]
    fn from_config_wires_stream_timeout() {
        let mut config = GlobalConfig::default();
        config.stream_timeout_secs = 15;
        let builder = FfmpegCommandBuilder::from_config(&config, "http://x", "out.m4a");
        assert_eq!(builder.stream_timeout_secs, 15);
    }

    #[test]
    fn build_bitrate_zero_keeps_mp3_quality_fallback() {
        let args: Vec<String> = FfmpegCommandBuilder::new()
            .input_url("http://x")
            .output_path("out.mp3")
            .format("mp3")
            .bitrate_kbps(0)
            .build()
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"-q:a".to_string()));
        assert!(args.contains(&"2".to_string()));
    }
}
