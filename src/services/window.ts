import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

/**
 * 当前窗口是否为设置向导窗口（wizard）。
 *
 * 双窗口架构中，主窗口与向导窗口加载同一 URL，前端靠窗口 label 区分自身角色。
 * 非 Tauri 环境（纯浏览器调试）下无法获取窗口，默认视为主窗口。
 */
export function isWizardWindow(): boolean {
  try {
    return getCurrentWebviewWindow().label === "wizard";
  } catch {
    return false;
  }
}
