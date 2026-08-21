<script setup lang="ts">
/**
 * 应用顶栏：页面标题（可选面包屑槽）+ 右侧操作区槽。
 * 简约风格——扁平、无边框、半透明背景 + 轻量 backdrop-blur。
 *
 * Linux 无边框（Arch 包无顶部操作栏，见 lib.rs setup）：左侧标题区作为
 * data-tauri-drag-region 拖拽窗口；右侧追加最小化/关闭控制按钮（仅 Linux
 * 渲染；按钮 @mousedown.stop 阻断拖拽区域冒泡，避免按下即拖动）。
 */
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, X } from "@lucide/vue";

defineProps<{
    title: string;
}>();

/** Linux（WebKitGTK UA 含 "Linux"；jsdom 亦含，按钮仅渲染、点击时才调用窗口 API） */
const isLinux = navigator.userAgent.includes("Linux");

function minimizeWindow() {
    try {
        void getCurrentWindow().minimize();
    } catch {
        /* 非 Tauri 环境（浏览器调试）忽略 */
    }
}

function closeWindow() {
    try {
        void getCurrentWindow().close();
    } catch {
        /* 非 Tauri 环境（浏览器调试）忽略 */
    }
}
</script>

<template>
    <header
        class="flex h-14 shrink-0 items-center justify-between gap-4 bg-background/80 px-6 backdrop-blur"
    >
        <div class="flex min-w-0 items-center gap-3" data-tauri-drag-region>
            <!-- 面包屑 / 上级导航（可选） -->
            <slot name="breadcrumb" />
            <h1
                class="truncate text-[15px] font-semibold tracking-tight text-foreground"
            >
                {{ title }}
            </h1>
        </div>
        <div class="flex shrink-0 items-center gap-2">
            <slot name="actions" />
            <!-- Linux 无边框：窗口控制按钮（Windows/macOS 用系统标题栏按钮） -->
            <template v-if="isLinux">
                <button
                    type="button"
                    class="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                    aria-label="最小化"
                    @mousedown.stop
                    @click="minimizeWindow"
                >
                    <Minus class="size-4" />
                </button>
                <button
                    type="button"
                    class="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                    aria-label="关闭"
                    @mousedown.stop
                    @click="closeWindow"
                >
                    <X class="size-4" />
                </button>
            </template>
        </div>
    </header>
</template>
