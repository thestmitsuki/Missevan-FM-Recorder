import { createI18n } from "vue-i18n";
import zhCN from "./zh-CN";
import en from "./en";

export type AppLocale = "zh-CN" | "en";

/** 语言偏好 localStorage 键（D5：语言存前端 localStorage） */
const LOCALE_KEY = "locale";

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

/** 切换应用语言（向导基本设置页使用），即时生效并持久化 */
export function setLocale(locale: AppLocale) {
  i18n.global.locale.value = locale;
  try {
    localStorage.setItem(LOCALE_KEY, locale);
  } catch {
    // 忽略持久化失败
  }
}
