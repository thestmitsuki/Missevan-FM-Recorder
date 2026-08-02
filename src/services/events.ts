import { listen } from "@tauri-apps/api/event";
import type {
  AnchorStatusUpdate,
  LogEntry,
  MockStatusChanged,
  Notification,
  RecordingFilesPayload,
} from "@/types";
import type { DownloadProgress } from "@/types/health";

type NotificationHandler = (notification: Notification) => void;
type RecordingStatusHandler = (update: AnchorStatusUpdate) => void;
type RecordingFilesHandler = (payload: RecordingFilesPayload) => void;
type DebugLogHandler = (entry: LogEntry) => void;
type DownloadProgressHandler = (payload: DownloadProgress) => void;
type MockStatusHandler = (payload: MockStatusChanged) => void;
type TrayOpenLivePageHandler = () => void;

const notificationHandlers = new Set<NotificationHandler>();
const recordingStatusHandlers = new Set<RecordingStatusHandler>();
const recordingFilesHandlers = new Set<RecordingFilesHandler>();
const debugLogHandlers = new Set<DebugLogHandler>();
const downloadProgressHandlers = new Set<DownloadProgressHandler>();
const mockStatusHandlers = new Set<MockStatusHandler>();
const trayOpenLivePageHandlers = new Set<TrayOpenLivePageHandler>();

/**
 * 事件统一监听层（规格 11.3）：
 * 所有 tauri 事件在此集中 listen，业务侧通过 onXxx 注册处理器。
 * 页面/组件不得直接调用 listen。
 */
export async function setupEventListeners() {
  await listen<Notification>("app:notification", (event) => {
    const notification = event.payload;
    notificationHandlers.forEach((h) => h(notification));
  });

  await listen<AnchorStatusUpdate>("recording_status_changed", (event) => {
    recordingStatusHandlers.forEach((h) => h(event.payload));
  });

  await listen<RecordingFilesPayload>("recording_files_changed", (event) => {
    recordingFilesHandlers.forEach((h) => h(event.payload));
  });

  await listen<LogEntry>("debug:log", (event) => {
    debugLogHandlers.forEach((h) => h(event.payload));
  });

  await listen<DownloadProgress>("download:progress", (event) => {
    downloadProgressHandlers.forEach((h) => h(event.payload));
  });

  await listen<MockStatusChanged>("mock:status_changed", (event) => {
    mockStatusHandlers.forEach((h) => h(event.payload));
  });

  // 托盘「录制中：N」点击 → 导航到直播页（Task 17 已 emit，Task 20 前端接线）
  await listen("tray:open_live_page", () => {
    trayOpenLivePageHandlers.forEach((h) => h());
  });
}

/** 订阅应用通知事件，返回取消订阅函数 */
export function onNotification(handler: NotificationHandler) {
  notificationHandlers.add(handler);
  return () => notificationHandlers.delete(handler);
}

/** 订阅主播状态推送事件（直播/录制切换），返回取消订阅函数 */
export function onRecordingStatusChanged(handler: RecordingStatusHandler) {
  recordingStatusHandlers.add(handler);
  return () => recordingStatusHandlers.delete(handler);
}

/**
 * 订阅文件缓存变更事件（录制完成/重命名/删除后由后端推送），返回取消订阅函数。
 * fileStore 通过此函数接入，页面不得直接 listen。
 */
export function onRecordingFilesChanged(handler: RecordingFilesHandler) {
  recordingFilesHandlers.add(handler);
  return () => recordingFilesHandlers.delete(handler);
}

/** 订阅调试日志流事件（后端 tracing → `debug:log`，节流 100 条/秒），返回取消订阅函数 */
export function onDebugLog(handler: DebugLogHandler) {
  debugLogHandlers.add(handler);
  return () => debugLogHandlers.delete(handler);
}

/** 订阅 FFmpeg 下载进度事件（`download:progress`），返回取消订阅函数 */
export function onDownloadProgress(handler: DownloadProgressHandler) {
  downloadProgressHandlers.add(handler);
  return () => downloadProgressHandlers.delete(handler);
}

/** 订阅 Mock 状态变更事件（`mock:status_changed`），返回取消订阅函数 */
export function onMockStatusChanged(handler: MockStatusHandler) {
  mockStatusHandlers.add(handler);
  return () => mockStatusHandlers.delete(handler);
}

/** 订阅托盘「录制中：N」点击事件（`tray:open_live_page`，主窗口导航到直播页），返回取消订阅函数 */
export function onTrayOpenLivePage(handler: TrayOpenLivePageHandler) {
  trayOpenLivePageHandlers.add(handler);
  return () => trayOpenLivePageHandlers.delete(handler);
}
