import { defineStore } from "pinia";
import { ref } from "vue";
import { isLinuxPlatform } from "@/services/platform";
import type { ThemeMode } from "@/stores/themeStore";
import type { GlobalConfig } from "@/types";

/** 第二页「基本设置」暂存的配置（校验通过后暂存内存，最后一步「完成」才写盘） */
export interface WizardStaged {
  language: "zh-CN" | "en";
  outputDir: string;
  recordFormat: "m4a" | "mp3";
  segmentSeconds: number;
  diskThresholdGb: number;
  autostart: boolean;
  trayMinimize: boolean;
  theme: ThemeMode;
  /** FFmpeg 可执行文件路径（下载后暂存；完成时随 stagedToConfigPatch 写入，null = 自动探测） */
  ffmpegPath: string | null;
  /** FFprobe 可执行文件路径（下载后暂存；空串 = 自动探测） */
  ffprobePath: string;
}

export const useWizardStore = defineStore("wizard", () => {
  // ── 第二页暂存配置（内存，不写盘） ──
  // 注（M7 审查跟进）：向导「完成标记」的 localStorage 逻辑已整体移除——该键
  // 只写不读、且写入点在 finish_wizard 销毁窗口之后（await 永不返回）不可达，
  // 属死代码。首次运行判定完全以后端 `GlobalConfig.wizard_completed` /
  // `is_first_run()` 为准（config.toml 落盘即持久化），前端无需冗余标记。
  const staged = ref<WizardStaged>({
    language: "zh-CN",
    outputDir: "",
    recordFormat: "m4a",
    segmentSeconds: 0,
    diskThresholdGb: 10,
    autostart: false,
    // Linux 未集成系统托盘（决策 #2）：初始值为 false（Windows 上由 initStaged 按配置覆盖）
    trayMinimize: false,
    theme: "system",
    // FFmpeg/ffprobe 路径：下载后暂存（不写盘），完成时随 save_config 落盘
    ffmpegPath: null,
    ffprobePath: "",
  });

  /** 用现有配置/偏好填充暂存默认值（只填空值，保留用户已编辑内容） */
  function initStaged(config: GlobalConfig, locale: string, theme: ThemeMode) {
    if (staged.value.language === "zh-CN" && locale === "en") {
      staged.value.language = "en";
    }
    if (!staged.value.outputDir && config.output_dir) {
      staged.value.outputDir = config.output_dir;
    }
    if (config.record_format === "mp3") {
      staged.value.recordFormat = "mp3";
    }
    if (config.segment_seconds > 0) {
      staged.value.segmentSeconds = config.segment_seconds;
    }
    if (config.disk_space_limit_gb > 0) {
      staged.value.diskThresholdGb = config.disk_space_limit_gb;
    }
    staged.value.autostart = config.autostart;
    // Linux 未集成系统托盘：恒 false（托盘选项已禁用，避免旧配置 close_behavior=tray 带入）
    staged.value.trayMinimize = isLinuxPlatform() ? false : config.close_behavior !== "exit";
    // FFmpeg/ffprobe 路径：只填空值（下载后已暂存则不覆盖）
    if (!staged.value.ffmpegPath && config.ffmpeg_path) {
      staged.value.ffmpegPath = config.ffmpeg_path;
    }
    if (!staged.value.ffprobePath && config.ffprobe_path) {
      staged.value.ffprobePath = config.ffprobe_path;
    }
    staged.value.theme = theme;
  }

  function setStaged(patch: Partial<WizardStaged>) {
    staged.value = { ...staged.value, ...patch };
  }

  return { staged, initStaged, setStaged };
});
