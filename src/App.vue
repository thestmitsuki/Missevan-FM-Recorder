<script setup lang="ts">
import { ref, watch, onMounted, onUnmounted } from "vue";
import { useRouter } from "vue-router";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import ErrorBoundary from "@/components/common/ErrorBoundary.vue";
import AppLayout from "@/layouts/AppLayout.vue";
import { useNotificationStore } from "@/stores/notificationStore";
import { useAnchorStore } from "@/stores/anchorStore";
import { useConfigStore } from "@/stores/configStore";
import { useThemeStore } from "@/stores/themeStore";
import Toast from "@/components/common/Toast.vue";
import { isWizardWindow } from "@/services/window";
import { onTrayOpenLivePage } from "@/services/events";

const notifStore = useNotificationStore();
const anchorStore = useAnchorStore();
const configStore = useConfigStore();
const toastMessage = ref("");
const toastKey = ref(0);
const router = useRouter();

// ── 双窗口：向导窗口只渲染向导路由，主窗口渲染完整布局。
// 路由跳转已由 router 全局守卫接管（src/router/index.ts），此处仅做渲染分支 ──
const isWizard = isWizardWindow();

// ── 窗口控制栏跟随明暗主题（官方 setTheme API，tao 原生处理标题栏明暗与系统跟随）──
// themeStore.mode 变化时：system → null（跟随系统，tao 监听系统设置变化自动刷新）；
// light / dark → 对应值。浏览器调试环境不可用，catch 忽略。
const themeStore = useThemeStore();
watch(
    () => themeStore.mode,
    (m) => {
        try {
            getCurrentWebviewWindow()
                .setTheme(m === "system" ? null : m)
                .catch(() => {});
        } catch {
            // 非 Tauri 环境（纯浏览器调试）
        }
    },
    { immediate: true },
);

// ── 托盘 → 直播页导航（Task 17 emit `tray:open_live_page`，Task 20 前端接线）──
let stopTrayOpen: (() => void) | null = null;

// ── 生命周期 ──

onMounted(async () => {
    // 1. 启动通知监听（必须）
    notifStore.startListening();

    // 2. 启动主播状态推送监听（recording_status_changed，统一 events.ts 层）
    anchorStore.startListening();

    // 2. 托盘「录制中：N」点击 → 导航到直播页（仅主窗口）
    if (!isWizard) {
        stopTrayOpen = onTrayOpenLivePage(() => {
            router.push("/");
        });
    }

    // 3. 加载配置
    try {
        await configStore.fetchConfig();
    } catch (e) {
        console.error("Failed to load config", e);
    }
});

onUnmounted(() => {
    // 4. 清理监听
    stopTrayOpen?.();
    notifStore.stopListening();
    anchorStore.stopListening();
});


// ── 监听最新通知，显示 Snackbar ──

watch(
    () => notifStore.latestNotification,
    (n) => {
        if (n) {
            toastMessage.value = n.message;
            toastKey.value++; // 通知序号：同文案连续通知也触发 Toast 刷新（不再重建组件，避免 sonner 丢失 toast）
        } else {
            toastMessage.value = ""; // 清空消息
            toastKey.value = 0; // 重置序号
        }
    },
    { immediate: false },
);
</script>

<template>
    <ErrorBoundary>
        <RouterView v-if="isWizard" />
        <AppLayout v-else />
    </ErrorBoundary>
    <!--- ##引发过消息残留## --->
    <Toast :message="toastMessage" :nonce="toastKey" :duration="3000" />
</template>

<style>
/* 注意：不在此做全局 margin/padding 重置——非 @layer 的全局规则会覆盖
   Tailwind v4 @layer 内的组件 utilities（如 p-4/mt-2），导致组件边距为 0。
   preflight 已提供等价重置。 */
* {
    box-sizing: border-box;
}
html,
body,
#app {
    height: 100%;
    width: 100%;
}
body {
    font-family:
        "Noto Sans SC",
        "PingFang SC",
        system-ui,
        -apple-system,
        sans-serif;
    background: var(--background);
    color: var(--foreground);
    -webkit-font-smoothing: antialiased;
}
</style>
