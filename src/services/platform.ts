/**
 * 当前是否 Windows 平台（Tauri WebView 环境）。
 *
 * FFmpeg 选择对话框与路径校验按平台区分：
 * Windows 上可执行文件带 .exe/.bat/.cmd/.ps1 扩展名；
 * Linux/macOS 上则是无扩展名的 ELF/Mach-O 可执行文件（如 /usr/bin/ffmpeg）。
 *
 * 优先用 navigator.userAgent 判断（Windows NT / Linux / Macintosh 标记），
 * navigator.platform 兜底（该属性已废弃但 WebView 中仍可用）。
 */
export function isWindowsPlatform(): boolean {
  return /Windows/i.test(navigator.userAgent) || /^Win/i.test(navigator.platform);
}

/**
 * 当前是否 Linux 平台（Tauri WebView 环境）。
 *
 * 项目打包目标仅 Windows（nsis）+ Linux（deb/appimage），非 Windows 即 Linux；
 * 本函数用于 Linux 未集成系统托盘（后端决策 #2：不创建 TrayManager）时的
 * 托盘相关 UI 禁用与提示。判定方式与 isWindowsPlatform 同源（UA / platform）。
 */
export function isLinuxPlatform(): boolean {
  return /Linux/i.test(navigator.userAgent) || /^Linux/i.test(navigator.platform);
}
