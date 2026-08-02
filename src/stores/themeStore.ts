import { defineStore } from "pinia";
import { ref } from "vue";

export type ThemeMode = "light" | "dark" | "system";

/** 主题偏好 localStorage 键（D5：UI 偏好存前端 localStorage） */
const THEME_MODE_KEY = "theme_mode";

function readMode(): ThemeMode {
  try {
    const saved = localStorage.getItem(THEME_MODE_KEY);
    if (saved === "light" || saved === "dark" || saved === "system") {
      return saved;
    }
  } catch {
    // localStorage 不可用时回退系统跟随
  }
  return "system";
}

export const useThemeStore = defineStore("theme", () => {
  const mode = ref<ThemeMode>(readMode());
  const resolvedTheme = ref<"light" | "dark">("light");

  function getSystemTheme(): "light" | "dark" {
    return window.matchMedia("(prefers-color-scheme: dark)").matches
      ? "dark"
      : "light";
  }

  function resolveTheme(): "light" | "dark" {
    return mode.value === "system" ? getSystemTheme() : mode.value;
  }

  function applyTheme(theme: "light" | "dark") {
    const root = document.documentElement;
    // shadcn 组件走 .dark class 变体；旧 material 变量走 data-theme 属性，两者同设
    root.setAttribute("data-theme", theme);
    root.classList.toggle("dark", theme === "dark");
    resolvedTheme.value = theme;
  }

  function setMode(newMode: ThemeMode) {
    mode.value = newMode;
    applyTheme(resolveTheme());
    try {
      localStorage.setItem(THEME_MODE_KEY, newMode);
    } catch {
      // 忽略持久化失败
    }
  }

  // 监听系统主题变化（Tauri 为桌面客户端，window 始终可用）
  const mq = window.matchMedia("(prefers-color-scheme: dark)");
  mq.addEventListener("change", () => {
    if (mode.value === "system") {
      applyTheme(getSystemTheme());
    }
  });

  // 初始化
  applyTheme(resolveTheme());

  return {
    mode,
    resolvedTheme,
    setMode,
  };
});
