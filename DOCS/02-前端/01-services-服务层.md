# 01 · services —— 服务层（Tauri 桥接）

> 文件：`src/services/{api,events,liveUrl,platform,window}.ts`

## 1. 职责与纪律

- **唯一允许 `invoke` / `listen` 的层**：页面、组件、store 一律通过本层调用，禁止直接触碰 `@tauri-apps/api`（便于统一错误处理与类型）。
- 类型契约：所有命令返回类型与 `src/types/*` 对齐（后端 DTO 逐字段）。

## 2. api.ts —— 54 个命令封装

按域分组导出单一 `api` 对象：

| 分组 | 方法（节选） | 对应后端命令 |
| --- | --- | --- |
| Anchor | `getAnchors` / `addAnchor` / `removeAnchor` / `refreshAnchor` / `updateAnchor` / `getAnchorProfile` / `getRecordingStatus` / `stopAnchorsRecording` | anchor_cmds |
| Config | `getConfig` / `saveConfig` / `exportConfig` / `importConfig` / `resetConfig` / `setAutostart` / `setShortcut` / `runCleanupNow` | config_cmds |
| Files | `getRecordingFiles` / `refreshRecordingFiles` / `renameRecordingFile` / `deleteRecordingFile` / `playRecordingFile` / `pickOutputDir` | file_cmds |
| Debug | `runHealthCheck` / `getDebugInfo` / `getLogs` / `clearLogs` / `getNetworkLogs` / `clearNetworkLogs` / `getDetectorStats` / `triggerDetectionNow` / `resetDetectorStats` / `getRecorderState` / `getFileCacheState` / `clearFileCache` / `getMockState` / `exportDiagnosticReport` | debug_cmds |
| Mock | `setMockLiveData` / `setMockMode` / `listMockAnchors` / `addMockAnchor` / `updateMockAnchor` / `removeMockAnchor` / `setAllMockLive` / `resetMock` | mock_cmds |
| Update | `checkUpdate` / `getAppInfo` / `openBrowser` | update_cmds |
| Wizard | `downloadFfmpeg` / `runWizardHealthCheck` / `exitApp` / `finishWizard` | wizard_cmds |
| 其他 | `openOutputDir` | fs_utils |

约定：`invoke<T>("command", { args })`，参数名与后端命令参数**逐字对齐**（Tauri 按参数名序列化，camelCase 前端 → snake_case 后端由 Tauri 自动转换）。

## 3. events.ts —— 事件监听中枢

```ts
// 每个事件一个 handler Set + onXxx 注册函数 + setupEventListeners() 统一 listen
const notificationHandlers = new Set<NotificationHandler>();
export function onNotification(h) { ...; return () => delete; }
export async function setupEventListeners() { /* listen 全部 7 事件 → fan-out */ }
```

| 事件 | 订阅函数 | 主要订阅者 |
| --- | --- | --- |
| `app:notification` | `onNotification` | notificationStore |
| `recording_status_changed` | `onRecordingStatusChanged` | anchorStore |
| `recording_files_changed` | `onRecordingFilesChanged` | fileStore |
| `debug:log` | `onDebugLog` | debugStore（调试页日志） |
| `mock:status_changed` | `onMockStatusChanged` | mockStore / debug 面板 |
| `tray:open_live_page` | `onTrayOpenLivePage` | App.vue（跳转直播页） |
| `download:progress` | `onDownloadProgress` | wizard 向导 |

`setupEventListeners()` 在 `main.ts` 挂载后调用（`then` 后才 fetchConfig——保证事件先就绪）。

## 4. liveUrl.ts —— 直播间 URL 校验

```ts
export const LIVE_URL_RE = /^https:\/\/fm\.missevan\.com\/live\/(\d+)\/?([?#].*)?$/;
export function extractRoomId(url: string): string | null;
```

- 与后端 `MissevanClient::extract_room_id` 对齐：仅允许尾部斜杠/查询串/锚点；
- 提取「/live/ 后第一段」为 room_id（路径只允许一段，杜绝 `/live/123/456` 前后端取段不一致的静默错配）。

## 5. platform.ts / window.ts —— 环境判定

- `isWindowsPlatform()` / `isLinuxPlatform()`：UA + navigator.platform 兜底（WebView 内可用）；用于 FFmpeg 选择对话框扩展名规则、Linux 托盘禁用提示。
- `isWizardWindow()`：`getCurrentWebviewWindow().label === "wizard"`（try/catch，浏览器调试环境默认 false=主窗）。

## 6. 已知陷阱

- **参数名对齐**：Tauri invoke 的参数名必须与后端 `#[tauri::command]` 函数参数一致（如 `{ anchor }` / `{ roomId }` → 后端 `room_id` 由 Tauri 转换）。新增命令时先写后端签名，再补 api.ts。
- 浏览器调试（非 Tauri）下 `invoke` 会失败：`services/window.ts` 已兜底，但业务功能必须在 `npm run tauri dev` 里验证。
- 事件订阅返回的 `unlisten` 函数要正确调用（store 的 `startListening`/`stopListening` 成对），避免重复监听导致事件处理翻倍。
- 新增事件 = 4 处：后端 emit 点、`events.ts` 类型 + listen + onXxx、订阅 store、i18n（如需文案）。
