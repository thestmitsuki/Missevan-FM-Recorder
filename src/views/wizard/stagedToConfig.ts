/**
 * 向导暂存配置 → save_config 载荷补丁（纯函数，便于单元测试）。
 *
 * 覆盖 WizardStaged 全部字段到配置落盘的映射：
 * - outputDir → output_dir；recordFormat → record_format；
 *   segmentSeconds → segment_seconds；diskThresholdGb → disk_space_limit_gb；
 *   autostart → autostart（布尔直通）；
 *   trayMinimize → close_behavior（语义映射：true=tray / false=exit；
 *   Linux 未集成系统托盘（决策 #2）时恒 exit，即使暂存值为 true）；
 *   ffmpegPath / ffprobePath → ffmpeg_path / ffprobe_path（下载后暂存；
 *   null / 空串 = 自动探测）；
 * - wizard_completed：默认 false；向导最后一步「完成」落盘时传
 *   { wizardCompleted: true }（与后端 finish_wizard 语义对齐——落盘即引导
 *   完成，下次启动不再进向导）。
 *
 * language / theme 为纯前端偏好（localStorage），由 BasicSettingsStep 在
 * 选择时即时提交（setLocale / themeStore.setMode），不走 save_config。
 *
 * 写入时机（修复子代理 B）：本补丁仅在第 4 步「完成」按钮点击时使用——
 * 配置文件的唯一写入点。向导第 1-3 步（欢迎/基本设置/环境检查）绝不落盘；
 * FFmpeg 下载也不再写盘（路径随 download_ffmpeg 返回值暂存到 staged）。
 */
import type { WizardStaged } from "@/stores/wizardStore";

/** save_config 载荷中由向导负责的字段子集（snake_case，与后端 GlobalConfig 对齐） */
export interface StagedConfigPatch {
  wizard_completed: boolean;
  output_dir: string;
  record_format: WizardStaged["recordFormat"];
  segment_seconds: number;
  disk_space_limit_gb: number;
  autostart: boolean;
  close_behavior: "tray" | "exit";
  ffmpeg_path: string | null;
  ffprobe_path: string;
}

/** 补丁构造选项 */
export interface StagedToConfigOptions {
  /** 向导完成落盘（第 4 步）时置 true；其余场景默认 false */
  wizardCompleted?: boolean;
}

/**
 * 由暂存值构造配置补丁。
 *
 * @param staged 向导第二页暂存值（wizardStore.staged）
 * @param linux  当前是否 Linux（无系统托盘 → close_behavior 恒 "exit"）
 * @param options 可选：wizardCompleted（完成落盘标记）
 */
export function stagedToConfigPatch(
  staged: WizardStaged,
  linux: boolean,
  options: StagedToConfigOptions = {},
): StagedConfigPatch {
  return {
    wizard_completed: options.wizardCompleted ?? false,
    output_dir: staged.outputDir.trim(),
    record_format: staged.recordFormat,
    segment_seconds: staged.segmentSeconds,
    disk_space_limit_gb: staged.diskThresholdGb,
    autostart: staged.autostart,
    // Linux 未集成系统托盘（决策 #2）：即使暂存值为 true 也恒写 exit
    close_behavior: !linux && staged.trayMinimize ? "tray" : "exit",
    ffmpeg_path: staged.ffmpegPath?.trim() ? staged.ffmpegPath : null,
    ffprobe_path: staged.ffprobePath?.trim() ?? "",
  };
}
