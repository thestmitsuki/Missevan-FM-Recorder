import { createI18n } from "vue-i18n";
import zhCN from "./zh-CN";
import en from "./en";
import { api } from "@/services/api";

export type AppLocale = "zh-CN" | "en";

/** 语言偏好 localStorage 键（D5：语言存前端 localStorage） */
const LOCALE_KEY = "locale";

/** 同步语言到后端（浏览器调试环境无 Tauri 后端，失败仅警告不阻断） */
function syncBackendLocale(locale: AppLocale) {
  api
    .setLocale(locale)
    .catch((err) => console.warn("[i18n] 后端语言同步失败（非 Tauri 环境或 IPC 未就绪）:", err));
}

/** 应用挂载后再次同步（IPC 就绪双保险；幂等，仅用于启动补偿） */
export function syncBackendLocaleNow() {
  syncBackendLocale(readLocale());
}

function readLocale(): AppLocale {
  try {
    const saved = localStorage.getItem(LOCALE_KEY);
    if (saved === "zh-CN" || saved === "en") {
      return saved;
    }
  } catch {
    // localStorage 不可用时回退中文
  }
  return "zh-CN";
}

export const i18n = createI18n({
  legacy: false,
  locale: readLocale(),
  fallbackLocale: "en",
  messages: { "zh-CN": zhCN, en },
});

// 启动时同步当前语言到后端（通知/错误提示/日志语言一致）
syncBackendLocale(readLocale());

/** 切换应用语言（向导基本设置页使用），即时生效并持久化 */
export function setLocale(locale: AppLocale) {
  i18n.global.locale.value = locale;
  try {
    localStorage.setItem(LOCALE_KEY, locale);
  } catch {
    // 忽略持久化失败
  }
  syncBackendLocale(locale);
}
