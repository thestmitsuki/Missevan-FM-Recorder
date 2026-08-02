<script setup lang="ts">
/**
 * 调试页（规格「调试页面功能规格」§8 + Mock 章节）——左导航 9 模块 + 右内容面板。
 *
 * 模块：
 *  1. 运行概览  2. 实时日志  3. 网络请求  4. 检测循环  5. 录制引擎
 *  6. 通知历史  7. 文件缓存  8. 性能监控（占位，标注实验性）  9. Mock 控制面板
 *
 * 交互模型：
 * - 每个模块一个 section 组件（sections/），模块挂载时加载一次数据 + 2s 轮询
 *   （日志/网络模块提供暂停按钮停轮询）；组件卸载自动清理定时器与事件监听；
 * - 左导航高亮当前模块；右面板顶栏含折叠/展开 + 刷新（规格交互细节）；
 * - 数据不持久化，刷新页面即清空（规格）。
 */
import { computed, ref, type Component } from "vue";
import { useI18n } from "vue-i18n";
import {
    Activity,
    Bell,
    Database,
    FlaskConical,
    Gauge,
    Globe,
    Radar,
    Terminal,
    Video,
} from "@lucide/vue";
import SideNav, { type SideNavItem } from "@/components/common/SideNav.vue";
import OverviewSection from "./sections/OverviewSection.vue";
import LogsSection from "./sections/LogsSection.vue";
import NetworkSection from "./sections/NetworkSection.vue";
import DetectorSection from "./sections/DetectorSection.vue";
import RecorderSection from "./sections/RecorderSection.vue";
import NotificationsSection from "./sections/NotificationsSection.vue";
import FileCacheSection from "./sections/FileCacheSection.vue";
import PerformanceSection from "./sections/PerformanceSection.vue";
import MockSection from "./sections/MockSection.vue";

const { t } = useI18n();

interface DebugModule {
    id: string;
    labelKey: string;
    icon: Component;
    component: Component;
}

const MODULES: DebugModule[] = [
    {
        id: "overview",
        labelKey: "debug.nav.overview",
        icon: Gauge,
        component: OverviewSection,
    },
    {
        id: "logs",
        labelKey: "debug.nav.logs",
        icon: Terminal,
        component: LogsSection,
    },
    {
        id: "network",
        labelKey: "debug.nav.network",
        icon: Globe,
        component: NetworkSection,
    },
    {
        id: "detector",
        labelKey: "debug.nav.detector",
        icon: Radar,
        component: DetectorSection,
    },
    {
        id: "recorder",
        labelKey: "debug.nav.recorder",
        icon: Video,
        component: RecorderSection,
    },
    {
        id: "notifications",
        labelKey: "debug.nav.notifications",
        icon: Bell,
        component: NotificationsSection,
    },
    {
        id: "filecache",
        labelKey: "debug.nav.filecache",
        icon: Database,
        component: FileCacheSection,
    },
    {
        id: "performance",
        labelKey: "debug.nav.performance",
        icon: Activity,
        component: PerformanceSection,
    },
    {
        id: "mock",
        labelKey: "debug.nav.mock",
        icon: FlaskConical,
        component: MockSection,
    },
];

const activeId = ref("overview");
const activeModule = computed(
    () => MODULES.find((m) => m.id === activeId.value) ?? MODULES[0],
);

function selectModule(id: string) {
    if (id !== activeId.value) activeId.value = id;
}

// ── 左导航（SideNav 通用组件，配置式）──
const navItems = computed<SideNavItem[]>(() =>
    MODULES.map((mod) => ({
        id: mod.id,
        label: t(mod.labelKey),
        icon: mod.icon,
    })),
);
</script>

<template>
    <div class="debug-layout">
        <!-- ── 左导航：9 模块（SideNav 通用组件，贴左全高） ── -->
        <SideNav
            :title="t('debug.title')"
            :items="navItems"
            :active-id="activeId"
            @select="selectModule"
        />

        <!-- ── 右内容面板（v-if 切换，仅挂载当前模块 → 定时器/监听随卸载清理）── -->
        <section class="debug-panel page-scroll">
            <component :is="activeModule.component" :key="activeModule.id" />
        </section>
    </div>
</template>

<style scoped>
/* 布局：左 SideNav（贴左全高，样式在 SideNav.vue）+ 右滚动面板 */
.debug-layout {
    display: flex;
    height: 100%;
    min-height: 0;
    flex: 1;
}

/* ── 右面板（居中限宽，不贴边） ── */
.debug-panel {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 24px;
    max-width: 1200px;
    margin: 0 auto;
}

/* 隐藏 SectionCard 内部的折叠按钮 */
:deep([aria-label="collapse"]) {
    display: none !important;
}
</style>
