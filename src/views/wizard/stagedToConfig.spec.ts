/**
 * stagedToConfigPatch 单元测试：
 * 验证向导暂存值 → save_config 载荷补丁的字段映射完整性——
 * 尤其是「开机自启」「小托盘」两个布尔值（H1 回归防护）与 Linux 无托盘语义，
 * 以及完成落盘载荷（wizard_completed=true + FFmpeg 下载路径）。
 */
import { describe, expect, it } from "vitest";
import { stagedToConfigPatch } from "./stagedToConfig";
import type { WizardStaged } from "@/stores/wizardStore";

/** 全字段有值的基础暂存（outputDir 带首尾空格，验证 trim） */
const base: WizardStaged = {
  language: "zh-CN",
  outputDir: "  D:/recordings  ",
  recordFormat: "m4a",
  segmentSeconds: 0,
  diskThresholdGb: 10,
  autostart: true,
  trayMinimize: true,
  theme: "system",
  ffmpegPath: null,
  ffprobePath: "",
};

describe("stagedToConfigPatch", () => {
  it("maps every WizardStaged field to the GlobalConfig snake_case patch", () => {
    const patch = stagedToConfigPatch(base, false);
    expect(patch).toEqual({
      wizard_completed: false,
      output_dir: "D:/recordings",
      record_format: "m4a",
      segment_seconds: 0,
      disk_space_limit_gb: 10,
      autostart: true,
      close_behavior: "tray",
      ffmpeg_path: null,
      ffprobe_path: "",
    });
  });

  it("writes autostart verbatim (true → true, false → false)", () => {
    expect(stagedToConfigPatch({ ...base, autostart: true }, false).autostart).toBe(
      true,
    );
    expect(stagedToConfigPatch({ ...base, autostart: false }, false).autostart).toBe(
      false,
    );
  });

  it("maps trayMinimize=true to close_behavior=tray on non-Linux", () => {
    expect(
      stagedToConfigPatch({ ...base, trayMinimize: true }, false).close_behavior,
    ).toBe("tray");
  });

  it("maps trayMinimize=false to close_behavior=exit", () => {
    expect(
      stagedToConfigPatch({ ...base, trayMinimize: false }, false).close_behavior,
    ).toBe("exit");
  });

  it("forces close_behavior=exit on Linux even when trayMinimize=true", () => {
    expect(
      stagedToConfigPatch({ ...base, trayMinimize: true }, true).close_behavior,
    ).toBe("exit");
  });

  it("trims outputDir whitespace", () => {
    expect(stagedToConfigPatch(base, false).output_dir).toBe("D:/recordings");
  });

  it("passes record_format / segment_seconds / disk_space_limit_gb through", () => {
    const patch = stagedToConfigPatch(
      { ...base, recordFormat: "mp3", segmentSeconds: 600, diskThresholdGb: 42 },
      false,
    );
    expect(patch.record_format).toBe("mp3");
    expect(patch.segment_seconds).toBe(600);
    expect(patch.disk_space_limit_gb).toBe(42);
  });

  it("defaults wizard_completed=false outside completion", () => {
    expect(stagedToConfigPatch(base, false).wizard_completed).toBe(false);
  });

  it("completion payload carries wizard_completed=true with non-default booleans", () => {
    // 向导最后一步「完成」落盘：autostart / trayMinimize→close_behavior /
    // wizard_completed 全部以非默认值写入（H1 回归防护）
    const patch = stagedToConfigPatch(
      {
        ...base,
        autostart: true,
        trayMinimize: true,
      },
      false,
      { wizardCompleted: true },
    );
    expect(patch.wizard_completed).toBe(true);
    expect(patch.autostart).toBe(true);
    expect(patch.close_behavior).toBe("tray");
  });

  it("completion payload includes downloaded ffmpeg paths verbatim", () => {
    const patch = stagedToConfigPatch(
      {
        ...base,
        ffmpegPath: "D:/app/ffmpeg/ffmpeg.exe",
        ffprobePath: "D:/app/ffmpeg/ffprobe.exe",
      },
      false,
      { wizardCompleted: true },
    );
    expect(patch.ffmpeg_path).toBe("D:/app/ffmpeg/ffmpeg.exe");
    expect(patch.ffprobe_path).toBe("D:/app/ffmpeg/ffprobe.exe");
  });

  it("normalizes empty ffmpeg paths to auto-detect sentinels", () => {
    const patch = stagedToConfigPatch(
      { ...base, ffmpegPath: "   ", ffprobePath: "  " },
      false,
    );
    expect(patch.ffmpeg_path).toBeNull();
    expect(patch.ffprobe_path).toBe("");
  });
});
