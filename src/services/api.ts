import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { isWindowsPlatform } from "@/services/platform";
import type {
  AnchorConfig,
  AnchorProfile,
  GlobalConfig,
  RecordingStatus,
  MockLiveData,
  DebugInfo,
  LogEntry,
  NetworkLog,
  DetectorStatsSnapshot,
  RecorderStateInfo,
  FileCacheState,
  MockStatusChanged,
  UpdateInfo,
  AppInfo,
  ImportSummary,
  RecordingFilesPayload,
  CleanupSummary,
} from "@/types";
import type {
  DiagnosticFullReport,
  DiagnosticReport,
  DownloadFfmpegResult,
} from "@/types/health";

// ── Anchor ──
export const api = {
  getAnchors: () => invoke<AnchorConfig[]>("get_anchors"),
  addAnchor: (anchor: AnchorConfig) => invoke<void>("add_anchor", { anchor }),
  removeAnchor: (id: string) => invoke<void>("remove_anchor", { id }),
  updateAnchor: (anchorId: string, anchor: AnchorConfig) =>
    invoke<void>("update_anchor", { anchorId, anchor }),
  refreshAnchor: (anchorId: string) =>
    invoke<AnchorConfig>("refresh_anchor", { anchorId }),
  /** 获取主播公开资料（名称/头像/简介），供设置面板简介区显示 */
  getAnchorProfile: (roomId: string) =>
    invoke<AnchorProfile>("get_anchor_profile", { roomId }),

  // ── Config ──
  getConfig: () => invoke<GlobalConfig>("get_config"),
  /** 同步前端语言到后端（后端通知/错误提示/日志按此语言输出；语言存前端 localStorage） */
  setLocale: (locale: string) => invoke<void>("set_locale", { locale }),
  saveConfig: (config: GlobalConfig) => invoke<void>("save_config", { config }),
  /** 开机自启注册表写入（save_config 只落盘字段、不写注册表；Task 14 Concern，Task 20 前端接线） */
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  /** 快捷键映射落盘 GlobalConfig.shortcuts（空 keys = 解绑） */
  setShortcut: (id: string, keys: string) =>
    invoke<void>("set_shortcut", { id, keys }),
  /** 导出配置 JSON（含全局配置 + 主播列表；敏感字段 proxy_password/cookie 已置空） */
  exportConfig: () => invoke<string>("export_config"),
  /** 导入配置（replace：全替换；merge：字段合并 + 主播按 id 去重合并）；成功返回导入汇总 */
  importConfig: (json: string, mode: "replace" | "merge") =>
    invoke<ImportSummary>("import_config", { json, mode }),
  /** 重置所有设置：删除配置目录后重启应用——命令不返回，调用方不得 await 返回值 */
  resetConfig: () => invoke<null>("reset_config"),

  // ── Update / About（Task 20：规格 §2.1）──
  /** 检查更新：GitHub Releases API 最新版本；失败抛「检查更新失败」错误 */
  checkUpdate: () => invoke<UpdateInfo>("check_update"),
  /** 关于窗口静态信息（应用名/版本/构建日期/OS/Rust/Tauri） */
  getAppInfo: () => invoke<AppInfo>("get_app_info"),
  /** 用默认浏览器打开 URL（仅 http/https） */
  openBrowser: (url: string) => invoke<void>("open_browser", { url }),

  // ── Recording ──
  getRecordingStatus: () => invoke<RecordingStatus[]>("get_recording_status"),
  stopRecording: (anchorId: string) =>
    invoke<void>("stop_recording", { anchorId }),
  /** 停止指定主播的录制（含 pre_record_delay 延迟窗口内的启动；主播操作入口用此命令） */
  stopAnchorsRecording: (anchorId: string) =>
    invoke<void>("stop_anchors_recording", { anchorId }),

  // ── Debug（Task 15 全部命令；返回 DTO 见 types/debug.ts）──
  getDebugInfo: () => invoke<DebugInfo>("get_debug_info"),
  runHealthCheck: () => invoke<DiagnosticFullReport>("run_health_check"),
  /** 获取日志（level 小写精确匹配；source 为 module 子串，均可省略） */
  getLogs: (level?: string, source?: string) =>
    invoke<LogEntry[]>("get_logs", { level: level ?? null, source: source ?? null }),
  clearLogs: () => invoke<void>("clear_logs"),
  getNetworkLogs: () => invoke<NetworkLog[]>("get_network_logs"),
  clearNetworkLogs: () => invoke<void>("clear_network_logs"),
  getDetectorStats: () => invoke<DetectorStatsSnapshot>("get_detector_stats"),
  triggerDetectionNow: () => invoke<void>("trigger_detection_now"),
  resetDetectorStats: () => invoke<void>("reset_detector_stats"),
  getRecorderState: () => invoke<RecorderStateInfo>("get_recorder_state"),
  getFileCacheState: () => invoke<FileCacheState>("get_file_cache_state"),
  clearFileCache: () => invoke<void>("clear_file_cache"),
  /** 导出诊断报告（概览 + 配置脱敏 + 日志 + 网络记录 + 统计），返回 JSON 字符串 */
  exportDiagnosticReport: () => invoke<string>("export_diagnostic_report"),
  getMockState: () => invoke<MockStatusChanged>("get_mock_state"),

  // ── Wizard ──
  /** 向导环境检查：基于暂存的输出目录/磁盘阈值检测 FFmpeg/ffprobe/磁盘/写入权限 */
  runWizardHealthCheck: (outputDir: string, diskThresholdGb: number) =>
    invoke<DiagnosticReport>("run_wizard_health_check", {
      outputDir,
      diskThresholdGb,
    }),
  /** 下载 FFmpeg 便携版到 {exe_dir}/ffmpeg/，返回重检结果 + 下载路径（不写配置） */
  downloadFfmpeg: () => invoke<DownloadFfmpegResult>("download_ffmpeg"),
  /** 退出应用（向导「不同意」/ 关闭确认「是」） */
  exitApp: () => invoke<void>("exit_app"),
  /** 向导完成：关向导窗、显主窗、刷新文件缓存、触发立即检测 */
  finishWizard: () => invoke<void>("finish_wizard"),

  // ── File ──
  pickOutputDir: () => invoke<string | null>("pick_output_dir"),
  /** 获取录制文件列表（search 模糊匹配文件名/主播名，省略 = 全量） */
  getRecordingFiles: (search?: string) =>
    invoke<RecordingFilesPayload>("get_recording_files", { search }),
  /** 立即扫描输出目录重建文件缓存（调试页「强制刷新文件缓存」/「立即扫描」） */
  refreshRecordingFiles: () => invoke<void>("refresh_recording_files"),
  /** 打开输出目录（opener 插件：资源管理器/xdg-open 打开目录本身；目录不存在时自动创建） */
  openOutputDir: () => invoke<void>("open_output_dir"),
  /** 立即执行一次录制文件清理（按保留天数/总量上限删旧文件；内部刷新文件缓存并 emit recording_files_changed） */
  runCleanupNow: () => invoke<CleanupSummary>("run_cleanup_now"),
  /** 重命名录制文件（oldPath → 新文件名） */
  renameRecordingFile: (oldPath: string, newName: string) =>
    invoke<void>("rename_recording_file", { oldPath, newName }),
  /** 删除录制文件 */
  deleteRecordingFile: (path: string) =>
    invoke<void>("delete_recording_file", { path }),
  /** 获取播放 URL（play_recording_file 返回 file:// URL，供外部播放器使用） */
  playRecordingFile: (path: string) =>
    invoke<string>("play_recording_file", { path }),
  /** 系统文件选择对话框（Tauri plugin-dialog，capability `dialog:default`；Task 14 可替换为 pick_ffmpeg 命令） */
  openExecutableDialog: () =>
    openDialog({
      multiple: false,
      directory: false,
      // Windows 上可执行文件限定 exe/bat/cmd/ps1；Linux/macOS 上 FFmpeg 是无扩展名的
      // ELF/Mach-O 可执行文件（如 /usr/bin/ffmpeg），不传 filters 即允许选择所有文件
      ...(isWindowsPlatform()
        ? { filters: [{ name: "Executable", extensions: ["exe", "bat", "cmd", "ps1"] }] }
        : {}),
    }).then((r) => (typeof r === "string" ? r : null)),

  // ── Mock（Task 16 补全命令：CRUD + 批量 + 重置）──
  setMockLiveData: (data: MockLiveData) =>
    invoke<void>("set_mock_live_data", { data }),
  setMockMode: (enable: boolean) => invoke<void>("set_mock_mode", { enable }),
  listMockAnchors: () => invoke<MockLiveData[]>("list_mock_anchors"),
  addMockAnchor: (anchor: MockLiveData) =>
    invoke<void>("add_mock_anchor", { anchor }),
  updateMockAnchor: (anchor: MockLiveData) =>
    invoke<void>("update_mock_anchor", { anchor }),
  removeMockAnchor: (roomId: string) =>
    invoke<void>("remove_mock_anchor", { roomId }),
  setAllMockLive: (live: boolean) =>
    invoke<void>("set_all_mock_live", { live }),
  resetMock: () => invoke<void>("reset_mock"),
};
