import { defineStore } from "pinia";
import { ref } from "vue";
import { api } from "@/services/api";
import { useConfigStore } from "@/stores/configStore";
import { compareVersions } from "@/lib/version";
import type { UpdateInfo } from "@/types";

/** GitHub Releases 发布页兜底（download_url 为 null 时的防御跳转） */
const RELEASES_FALLBACK_URL =
    "https://github.com/thestmitsuki/Missevan-FM-Recorder/releases";

/**
 * 请求级并发去重：自动检查与手动检查并发时复用同一请求。
 *
 * `manual` 语义：
 * - `false`（默认，启动检查）：后端受 `check_updates` 开关约束；
 * - `true`（AboutDialog 手动检查）：后端永远放行。
 *
 * 复用规则：仅当已有进行中的请求时复用；不存在 inFlight 时按本次 `manual`
 * 新建。由于失败路径 `finally` 会立即清空 `inFlight`，被开关拒绝的自动请求
 * 不会污染紧随其后的手动请求。
 */
let inFlight: Promise<UpdateInfo> | null = null;
export function fetchUpdateInfo(manual = false): Promise<UpdateInfo> {
    if (!inFlight) {
        inFlight = api.checkUpdate(manual).finally(() => {
            inFlight = null;
        });
    }
    return inFlight;
}

/**
 * 启动自动检查更新（App.vue onMounted 主窗口触发一次）。
 *
 * 开关语义：
 * - `check_updates` 关闭 → 不发起检查（后端 check_update 也会拒绝，双保险）；
 * - `notify_update` 只控制弹窗——开启则检查，关闭则检查结果静默丢弃；
 * - 失败静默全集：网络错误 / 429 / 404 / 解析失败 / 开关关闭 → 一律静默，
 *   仅「确认有新版本 且 notify_update 开启」才弹出更新提示。
 */
export const useUpdateStore = defineStore("update", () => {
    const checking = ref(false);
    /** 本次会话启动检查已执行过（只弹一次，不重复打扰） */
    const prompted = ref(false);
    /** AlertDialog 绑定（全局挂载于 App.vue，跨路由显示） */
    const promptOpen = ref(false);
    const latest = ref("");
    const current = ref("");
    const downloadUrl = ref<string | null>(null);

    async function checkOnStartup(): Promise<void> {
        const config = useConfigStore().config;
        if (!config.check_updates) return; // 未启用 → 不执行
        if (checking.value || prompted.value) return;
        checking.value = true;
        try {
            const result = await fetchUpdateInfo();
            if (compareVersions(result.latest, result.current) > 0) {
                if (!config.notify_update) return; // 检查到更新但不提醒（静默）
                latest.value = result.latest;
                current.value = result.current;
                downloadUrl.value = result.download_url;
                promptOpen.value = true;
            }
            // 无更新 → 静默
        } catch {
            // 失败静默全集（网络/HTTP/解析/开关拒绝）
        } finally {
            checking.value = false;
            prompted.value = true;
        }
    }

    /** 前往下载（download_url 为 null 时回退发布页；打开失败静默） */
    async function openDownload(): Promise<void> {
        const url = downloadUrl.value ?? RELEASES_FALLBACK_URL;
        promptOpen.value = false;
        try {
            await api.openBrowser(url);
        } catch {
            // 打开浏览器失败静默（启动弹窗场景不打扰）
        }
    }

    /** 稍后再说 */
    function dismiss(): void {
        promptOpen.value = false;
    }

    return {
        checking,
        prompted,
        promptOpen,
        latest,
        current,
        downloadUrl,
        checkOnStartup,
        openDownload,
        dismiss,
    };
});
