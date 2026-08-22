/**
 * 设置页校验逻辑单元测试：normalizeConfig / cloneForm / 分类 validators。
 *
 * 覆盖点：
 * - normalizeConfig：shortcuts / anchor_ids / 前端偏好字段剔除；ffmpeg_path 空值归一
 * - cloneForm：ffmpeg_path null → ""，深拷贝
 * - validators：数值上下限（各分类关键字段）、必填、代理联动、自定义 DNS 空白校验
 *
 * 说明：isWindowsPlatform 通过 vi.mock 控制（Windows/Linux 两分支均覆盖），
 * 被测逻辑（normalizeConfig / validators）不做任何 mock。
 */
import { describe, it, expect } from "vitest";
import { vi } from "vitest";

// 平台判定可控（hoisted 保证 vi.mock 工厂可读）
const platformMock = vi.hoisted(() => ({ isWindows: true }));
vi.mock("@/services/platform", () => ({
  isWindowsPlatform: () => platformMock.isWindows,
}));

import {
  normalizeConfig,
  cloneForm,
  validateCategory,
  validateAll,
  type SettingsForm,
} from "./validation";
import type { GlobalConfig } from "@/types";

/** i18n stub：直接返回 key，断言错误字段是否存在即可 */
const t = (key: string) => key;

/** 构造合法表单（全部字段满足校验），测试内按需覆盖个别字段 */
function makeForm(overrides: Partial<SettingsForm> = {}): SettingsForm {
  const base: SettingsForm = {
    output_dir: "C:\\recordings",
    record_format: "m4a",
    segment_seconds: 0,
    disk_space_limit_gb: 10,
    ffmpeg_path: "",
    anchor_ids: [],
    check_interval_secs: 60,
    max_retries: 3,
    retry_delay_secs: 5,
    autostart: false,
    close_behavior: "tray",
    show_tray: true,
    check_updates: true,
    bitrate_kbps: 128,
    audio_only: false,
    filename_template: "{date}_{time}_{anchor_name}.{ext}",
    max_concurrent_recordings: 2,
    pre_record_delay_secs: 0,
    post_record_action: "none",
    post_record_command: "",
    auto_cleanup_enabled: false,
    retention_days: 30,
    max_total_gb: 0,
    cleanup_time: "",
    proxy_type: "none",
    proxy_addr: "",
    proxy_port: 0,
    proxy_auth: false,
    proxy_username: "",
    proxy_password: "",
    api_timeout_secs: 30,
    stream_timeout_secs: 300,
    custom_dns: "",
    notifications_enabled: true,
    notify_recording_start: true,
    notify_recording_end: true,
    notify_recording_error: true,
    notify_live_start: true,
    notify_live_end: true,
    notify_disk_warning: true,
    notify_update: true,
    notify_system: true,
    notify_sound: true,
    log_level: "info",
    detector_concurrency: 2,
    ffprobe_path: "",
    detector_jitter_secs: 0,
    shortcuts: { toggle_record: "Ctrl+R" },
    wizard_completed: true,
    locale: "zh-CN",
    theme: "system",
    appearance: {
      accent: "#2563eb",
      density: "standard",
      fontSize: "medium",
      cardShowAvatar: true,
      cardShowTags: true,
      cardShowRoomId: true,
      cardShowStatusIcon: true,
    },
  };
  return { ...base, ...overrides };
}

describe("normalizeConfig", () => {
  it("剔除 shortcuts（H2 快捷键未启用，不入落盘）", () => {
    const result = normalizeConfig(makeForm());
    expect("shortcuts" in result).toBe(false);
  });

  it("剔除 anchor_ids（Important-2：由 Live 页维护，保存不透传）", () => {
    const result = normalizeConfig(makeForm());
    expect("anchor_ids" in result).toBe(false);
  });

  it("剔除纯前端偏好 locale/theme/appearance（I6）", () => {
    const result = normalizeConfig(makeForm());
    expect("locale" in result).toBe(false);
    expect("theme" in result).toBe(false);
    expect("appearance" in result).toBe(false);
  });

  it("ffmpeg_path 空串/空白/null 归一为 null（自动探测）", () => {
    expect(normalizeConfig(makeForm({ ffmpeg_path: "" })).ffmpeg_path).toBeNull();
    expect(normalizeConfig(makeForm({ ffmpeg_path: "   " })).ffmpeg_path).toBeNull();
    // Critical-1：null 兜底，不抛 TypeError
    expect(
      normalizeConfig(makeForm({ ffmpeg_path: null as unknown as string })).ffmpeg_path,
    ).toBeNull();
  });

  it("ffmpeg_path 非空保留原值（仅按 trim 判断是否为空，不做裁剪）", () => {
    const p = "C:\\ffmpeg\\bin\\ffmpeg.exe";
    expect(normalizeConfig(makeForm({ ffmpeg_path: p })).ffmpeg_path).toBe(p);
    const padded = "  ffmpeg  ";
    expect(normalizeConfig(makeForm({ ffmpeg_path: padded })).ffmpeg_path).toBe(padded);
  });

  it("其余字段原样透传", () => {
    const form = makeForm({ output_dir: "D:\\rec", segment_seconds: 3600 });
    const result = normalizeConfig(form) as Record<string, unknown>;
    expect(result.output_dir).toBe("D:\\rec");
    expect(result.segment_seconds).toBe(3600);
    expect(result.record_format).toBe("m4a");
  });

  it("返回新对象，不修改入参", () => {
    const form = makeForm();
    const before = JSON.stringify(form);
    normalizeConfig(form);
    expect(JSON.stringify(form)).toBe(before);
  });
});

describe("cloneForm", () => {
  it("ffmpeg_path null → 空串（表单形态，保存时再转回 null）", () => {
    const config: GlobalConfig = {
      ...(makeForm() as unknown as GlobalConfig),
      ffmpeg_path: null,
    };
    expect(cloneForm(config).ffmpeg_path).toBe("");
  });

  it("ffmpeg_path 有值原样保留", () => {
    const config = { ...(makeForm() as unknown as GlobalConfig), ffmpeg_path: "x" };
    expect(cloneForm(config).ffmpeg_path).toBe("x");
  });

  it("深拷贝：修改表单不影响源配置", () => {
    const config = { ...(makeForm() as unknown as GlobalConfig), ffmpeg_path: null };
    const form = cloneForm(config) as Record<string, unknown>;
    form.output_dir = "CHANGED";
    expect(config.output_dir).not.toBe("CHANGED");
  });
});

describe("validators · recording（数值上下限）", () => {
  it("output_dir 必填", () => {
    expect(validateCategory("recording", makeForm({ output_dir: "" }), t).output_dir).toBeTruthy();
    expect(validateCategory("recording", makeForm({ output_dir: "  " }), t).output_dir).toBeTruthy();
    expect(validateCategory("recording", makeForm(), t).output_dir).toBeUndefined();
  });

  it("segment_seconds 范围 [0, 86400]（0 = 不分割，合法）", () => {
    expect(validateCategory("recording", makeForm({ segment_seconds: -1 }), t).segment_seconds).toBeTruthy();
    expect(validateCategory("recording", makeForm({ segment_seconds: 86401 }), t).segment_seconds).toBeTruthy();
    expect(validateCategory("recording", makeForm({ segment_seconds: 0 }), t).segment_seconds).toBeUndefined();
    expect(validateCategory("recording", makeForm({ segment_seconds: 3600 }), t).segment_seconds).toBeUndefined();
  });

  it("max_concurrent_recordings 范围 [1, 32]", () => {
    expect(validateCategory("recording", makeForm({ max_concurrent_recordings: 0 }), t).max_concurrent_recordings).toBeTruthy();
    expect(validateCategory("recording", makeForm({ max_concurrent_recordings: 33 }), t).max_concurrent_recordings).toBeTruthy();
    expect(validateCategory("recording", makeForm({ max_concurrent_recordings: 32 }), t).max_concurrent_recordings).toBeUndefined();
  });

  it("bitrate_kbps 仅允许 64/128/192/256/320", () => {
    expect(validateCategory("recording", makeForm({ bitrate_kbps: 96 }), t).bitrate_kbps).toBeTruthy();
    expect(validateCategory("recording", makeForm({ bitrate_kbps: 320 }), t).bitrate_kbps).toBeUndefined();
  });

  it("post_record_action=command 时命令必填", () => {
    const form = makeForm({ post_record_action: "command", post_record_command: "" });
    expect(validateCategory("recording", form, t).post_record_command).toBeTruthy();
    const ok = makeForm({ post_record_action: "command", post_record_command: "explorer" });
    expect(validateCategory("recording", ok, t).post_record_command).toBeUndefined();
  });
});

describe("validators · files（数值上下限）", () => {
  it("retention_days 范围 [1, 3650]", () => {
    expect(validateCategory("files", makeForm({ retention_days: 0 }), t).retention_days).toBeTruthy();
    expect(validateCategory("files", makeForm({ retention_days: 3651 }), t).retention_days).toBeTruthy();
    expect(validateCategory("files", makeForm({ retention_days: 1 }), t).retention_days).toBeUndefined();
  });

  it("max_total_gb 范围 [0, 100000]（0 = 不限，合法）", () => {
    expect(validateCategory("files", makeForm({ max_total_gb: -1 }), t).max_total_gb).toBeTruthy();
    expect(validateCategory("files", makeForm({ max_total_gb: 100001 }), t).max_total_gb).toBeTruthy();
    expect(validateCategory("files", makeForm({ max_total_gb: 0 }), t).max_total_gb).toBeUndefined();
  });

  it("disk_space_limit_gb 范围 [1, 100000]（后端要求 > 0）", () => {
    expect(validateCategory("files", makeForm({ disk_space_limit_gb: 0 }), t).disk_space_limit_gb).toBeTruthy();
    expect(validateCategory("files", makeForm({ disk_space_limit_gb: 1 }), t).disk_space_limit_gb).toBeUndefined();
  });
});

describe("validators · network（数值上下限与联动）", () => {
  it("api_timeout_secs 范围 [1, 600]", () => {
    expect(validateCategory("network", makeForm({ api_timeout_secs: 0 }), t).api_timeout_secs).toBeTruthy();
    expect(validateCategory("network", makeForm({ api_timeout_secs: 601 }), t).api_timeout_secs).toBeTruthy();
    expect(validateCategory("network", makeForm({ api_timeout_secs: 600 }), t).api_timeout_secs).toBeUndefined();
  });

  it("stream_timeout_secs 范围 [1, 3600]", () => {
    expect(validateCategory("network", makeForm({ stream_timeout_secs: 3601 }), t).stream_timeout_secs).toBeTruthy();
    expect(validateCategory("network", makeForm({ stream_timeout_secs: 1 }), t).stream_timeout_secs).toBeUndefined();
  });

  it("启用代理时：地址必填、端口须为 [1, 65535]", () => {
    const noAddr = makeForm({ proxy_type: "http", proxy_addr: "", proxy_port: 8080 });
    expect(validateCategory("network", noAddr, t).proxy_addr).toBeTruthy();

    const badPort = makeForm({ proxy_type: "http", proxy_addr: "127.0.0.1", proxy_port: 0 });
    expect(validateCategory("network", badPort, t).proxy_port).toBeTruthy();

    const ok = makeForm({ proxy_type: "http", proxy_addr: "127.0.0.1", proxy_port: 8080 });
    const e = validateCategory("network", ok, t);
    expect(e.proxy_addr).toBeUndefined();
    expect(e.proxy_port).toBeUndefined();
  });

  it("启用代理鉴权时用户名必填", () => {
    const noUser = makeForm({ proxy_type: "http", proxy_addr: "127.0.0.1", proxy_port: 8080, proxy_auth: true, proxy_username: "" });
    expect(validateCategory("network", noUser, t).proxy_username).toBeTruthy();
    const ok = makeForm({ proxy_type: "http", proxy_addr: "127.0.0.1", proxy_port: 8080, proxy_auth: true, proxy_username: "u" });
    expect(validateCategory("network", ok, t).proxy_username).toBeUndefined();
  });

  it("custom_dns 含空白时报错", () => {
    expect(validateCategory("network", makeForm({ custom_dns: "8.8.8.8 1.1.1.1" }), t).custom_dns).toBeTruthy();
    expect(validateCategory("network", makeForm({ custom_dns: "8.8.8.8" }), t).custom_dns).toBeUndefined();
  });
});

describe("validators · advanced（数值上下限与平台分支）", () => {
  it("detector_concurrency 范围 [1, 64]", () => {
    expect(validateCategory("advanced", makeForm({ detector_concurrency: 0 }), t).detector_concurrency).toBeTruthy();
    expect(validateCategory("advanced", makeForm({ detector_concurrency: 65 }), t).detector_concurrency).toBeTruthy();
    expect(validateCategory("advanced", makeForm({ detector_concurrency: 64 }), t).detector_concurrency).toBeUndefined();
  });

  it("check_interval_secs 范围 [5, 86400]（后端要求 >= 5）", () => {
    expect(validateCategory("advanced", makeForm({ check_interval_secs: 4 }), t).check_interval_secs).toBeTruthy();
    expect(validateCategory("advanced", makeForm({ check_interval_secs: 5 }), t).check_interval_secs).toBeUndefined();
  });

  it("detector_jitter_secs 范围 [0, 86400]（0 = 不抖动，合法）", () => {
    expect(validateCategory("advanced", makeForm({ detector_jitter_secs: -1 }), t).detector_jitter_secs).toBeTruthy();
    expect(validateCategory("advanced", makeForm({ detector_jitter_secs: 0 }), t).detector_jitter_secs).toBeUndefined();
  });

  it("Windows：ffmpeg/ffprobe 路径须带可执行扩展名", () => {
    platformMock.isWindows = true;
    const badFfmpeg = makeForm({ ffmpeg_path: "C:\\tools\\ffmpeg" });
    expect(validateCategory("advanced", badFfmpeg, t).ffmpeg_path).toBeTruthy();
    const okFfmpeg = makeForm({ ffmpeg_path: "C:\\tools\\ffmpeg.exe" });
    expect(validateCategory("advanced", okFfmpeg, t).ffmpeg_path).toBeUndefined();
    const badFfprobe = makeForm({ ffprobe_path: "C:\\tools\\ffprobe" });
    expect(validateCategory("advanced", badFfprobe, t).ffprobe_path).toBeTruthy();
  });

  it("非 Windows：无扩展名可执行文件合法（ELF/Mach-O）", () => {
    platformMock.isWindows = false;
    const linux = makeForm({ ffmpeg_path: "/usr/bin/ffmpeg", ffprobe_path: "/usr/bin/ffprobe" });
    const e = validateCategory("advanced", linux, t);
    expect(e.ffmpeg_path).toBeUndefined();
    expect(e.ffprobe_path).toBeUndefined();
  });
});

describe("validateAll", () => {
  it("返回全部 8 个分类，合法表单无错误", () => {
    const all = validateAll(makeForm(), t);
    expect(Object.keys(all).sort()).toEqual([
      "advanced",
      "appearance",
      "files",
      "general",
      "network",
      "notification",
      "recording",
      "shortcuts",
    ]);
    for (const errors of Object.values(all)) {
      expect(errors).toEqual({});
    }
  });

  it("非法表单仅对应分类报错", () => {
    const all = validateAll(makeForm({ segment_seconds: -1, retention_days: 0 }), t);
    expect(all.recording.segment_seconds).toBeTruthy();
    expect(all.files.retention_days).toBeTruthy();
    expect(all.network).toEqual({});
  });
});
