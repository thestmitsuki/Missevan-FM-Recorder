import { defineStore } from "pinia";
import { ref } from "vue";
import type { ThemeMode } from "@/stores/themeStore";
import type { GlobalConfig } from "@/types";

/** 向导完成标记的 localStorage 键（与后端 is_first_run 双重判定） */
const WIZARD_COMPLETED_KEY = "wizard_completed";

/** 第二页「基本设置」暂存的配置（校验通过后暂存内存，第三页通过后才写盘） */
export interface WizardStaged {
  language: "zh-CN" | "en";
  outputDir: string;
  recordFormat: "m4a" | "mp3";
  segmentSeconds: number;
  diskThresholdGb: number;
  autostart: boolean;
  trayMinimize: boolean;
  theme: ThemeMode;
}

function readCompleted(): boolean {
  try {
    return localStorage.getItem(WIZARD_COMPLETED_KEY) === "1";
  } catch {
    return false;
  }
}

export const useWizardStore = defineStore("wizard", () => {
  // 向导完成标记（localStorage）：当前无消费方——保留给未来「导入旧配置/重新引导」
  // 流程使用（首次运行判定以后端 is_first_run 为准，本标记仅作前端冗余记录）
  const wizardCompleted = ref(readCompleted());

  // ── 第二页暂存配置（内存，不写盘） ──
  const staged = ref<WizardStaged>({
    language: "zh-CN",
    outputDir: "",
    recordFormat: "m4a",
    segmentSeconds: 0,
    diskThresholdGb: 10,
    autostart: false,
    trayMinimize: true,
    theme: "system",
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
    staged.value.trayMinimize = config.close_behavior !== "exit";
    staged.value.theme = theme;
  }

  function setStaged(patch: Partial<WizardStaged>) {
    staged.value = { ...staged.value, ...patch };
  }

  function complete() {
    wizardCompleted.value = true;
    try {
      localStorage.setItem(WIZARD_COMPLETED_KEY, "1");
    } catch {
      // localStorage 不可用（非 Tauri 调试环境）时仅内存标记
    }
  }

  return { wizardCompleted, staged, initStaged, setStaged, complete };
});
