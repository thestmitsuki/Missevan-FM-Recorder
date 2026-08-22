<script setup lang="ts">
import { computed } from "vue";
import { useRouter, useRoute } from "vue-router";
import { useI18n } from "vue-i18n";
import { Bug, Folder, Radio, Settings } from "@lucide/vue";
import { useAnchorStore } from "@/stores/anchorStore"; // 导入锚点 store
import { useDebugStore } from "@/stores/debugStore";
import { Button } from "@/components/ui/button";
import TopBar from "@/components/layout/TopBar.vue";
import PageContainer from "@/components/layout/PageContainer.vue";

const router = useRouter();
const route = useRoute();
const { t } = useI18n();
useAnchorStore();
const debugStore = useDebugStore();

// 主导航：直播/文件/设置 常显；调试面板仅在设置-关于开启后显示
const navItems = computed(() => {
    const items = [
        { path: "/", labelKey: "nav.liveMonitor", icon: Radio },
        { path: "/files", labelKey: "nav.fileManager", icon: Folder },
        { path: "/settings", labelKey: "nav.settings", icon: Settings },
    ];
    if (debugStore.enabled) {
        items.push({ path: "/debug", labelKey: "nav.debugPanel", icon: Bug });
    }
    return items;
});
</script>

<template>
    <div class="app-layout">
        <!-- 应用导航抽屉（桌面 240px；窄屏压缩为 56px 图标条，始终在左侧） -->
        <aside class="nav-drawer">
            <div class="nav-drawer-header">
                <span class="nav-brand">{{ t("brand") }}</span>
            </div>
            <nav class="nav-items">
                <Button
                    v-for="item in navItems"
                    :key="item.path"
                    variant="ghost"
                    class="nav-item h-auto w-full justify-start gap-3 rounded-lg px-3 py-2 text-sm font-medium"
                    :class="{
                        'bg-primary text-primary-foreground hover:bg-primary/85 hover:text-primary-foreground dark:hover:bg-primary/85 dark:hover:text-primary-foreground':
                            route.path === item.path,
                    }"
                    @click="router.push(item.path)"
                >
                    <component :is="item.icon" class="size-5 shrink-0" />
                    <span class="nav-item-label">{{ t(item.labelKey) }}</span>
                </Button>
            </nav>
        </aside>

        <!-- 主内容区：TopBar + 全高内容容器（页面自带竖向操作栏 + 滚动区） -->
        <main class="main-content">
            <TopBar :title="t(String(route.meta.title || ''))" />
            <PageContainer>
                <RouterView />
            </PageContainer>
        </main>
    </div>
</template>

<style scoped>
.app-layout {
    display: flex;
    height: 100vh;
    overflow: hidden;
    background: var(--background);
    color: var(--foreground);
}

/* 桌面导航 */
.nav-drawer {
    width: 240px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    background: var(--background);
}
.nav-drawer-header {
    padding: 20px 20px 12px;
}
.nav-brand {
    font-size: 1.0625rem;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--foreground);
}
.nav-items {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px 12px;
}

/* 主内容 */
.main-content {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    overflow: hidden;
    background: var(--background);
}

/* 窄屏：应用导航保持左侧，压缩为 56px 图标条（不放底部）——
   品牌字与条目文字隐藏，图标居中；子页面 NavRail/SideNav 同规则（见各自组件） */
@media (max-width: 720px) {
    .nav-drawer {
        width: 56px;
    }
    .nav-drawer-header {
        padding: 16px 0 8px;
    }
    .nav-brand,
    .nav-item-label {
        display: none;
    }
    .nav-items {
        padding: 8px 4px;
    }
    .nav-item {
        justify-content: center;
        padding: 9px 0;
    }
}
</style>
