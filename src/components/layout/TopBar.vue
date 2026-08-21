<script setup lang="ts">
/**
 * 应用顶栏：页面标题（可选面包屑槽）+ 右侧操作区槽。
 * 简约风格——扁平、无边框、半透明背景 + 轻量 backdrop-blur。
 */
defineProps<{
    title: string;
}>();
</script>

<template>
    <header
        class="flex h-14 shrink-0 items-center justify-between gap-4 bg-background/80 px-6 backdrop-blur"
    >
        <div class="flex min-w-0 items-center gap-3">
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
        </div>
    </header>
</template>
