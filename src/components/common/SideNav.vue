<script setup lang="ts">
/**
 * 通用左侧导航（设置页 8 分类 / 调试页 9 模块共用）
 *
 * 布局原则与 NavRail 一致：贴左侧（紧邻应用导航抽屉右缘）、全高
 * （父容器 flex 拉伸，不跟随内容高度）；无边框，与内容区靠留白区分。
 * 选中态：solid primary 高亮（aria-current="page"）。
 * 底部附加项（footerItems）：如设置页「关于」入口。
 * 窄屏（<=720px）：转为横向滚动导航行（与 AppLayout 底部导航断点一致）。
 */
import type { Component } from "vue";

/** 左导航条目配置 */
export interface SideNavItem {
    /** 唯一标识（分类/模块 id） */
    id: string;
    /** 已翻译的条目文案 */
    label: string;
    /** 条目图标（@lucide/vue，可选） */
    icon?: Component;
    /** 条目徽标（选中时显示小圆点，如设置页「未保存」标记） */
    badge?: boolean;
}

const props = withDefaults(
    defineProps<{
        /** 导航标题（如 设置 / 调试） */
        title?: string;
        /** 主条目配置数组 */
        items: SideNavItem[];
        /** 底部附加条目（如「关于」，不参与选中态） */
        footerItems?: SideNavItem[];
        /** 当前选中项 id */
        activeId: string;
    }>(),
    {
        title: "",
        footerItems: () => [],
    },
);

const emit = defineEmits<{
    select: [id: string];
}>();
</script>

<template>
    <aside class="side-nav">
        <h2 v-if="props.title" class="side-nav-title">
            {{ props.title }}
        </h2>
        <nav class="side-nav-list">
            <button
                v-for="item in props.items"
                :key="item.id"
                type="button"
                class="side-nav-item"
                :class="{ active: props.activeId === item.id }"
                :aria-current="props.activeId === item.id ? 'page' : undefined"
                @click="emit('select', item.id)"
            >
                <component :is="item.icon" v-if="item.icon" class="size-4 shrink-0" />
                <span class="side-nav-item-label min-w-0 flex-1 truncate text-left">{{ item.label }}</span>
                <span
                    v-if="item.badge && props.activeId === item.id"
                    class="size-1.5 shrink-0 rounded-full bg-primary-foreground/80"
                    aria-hidden="true"
                />
            </button>
        </nav>
        <!-- 底部附加区（如设置页「关于」入口） -->
        <div v-if="props.footerItems.length" class="side-nav-footer">
            <button
                v-for="item in props.footerItems"
                :key="item.id"
                type="button"
                class="side-nav-item"
                @click="emit('select', item.id)"
            >
                <component :is="item.icon" v-if="item.icon" class="size-4 shrink-0" />
                <span class="side-nav-item-label min-w-0 flex-1 truncate text-left">{{ item.label }}</span>
            </button>
        </div>
    </aside>
</template>

<style scoped>
/* ── 桌面：贴左、全高 ── */
.side-nav {
    display: flex;
    flex-direction: column;
    width: 200px;
    flex-shrink: 0;
    height: 100%;
    min-height: 0;
    background: var(--background);
}
.side-nav-title {
    font-size: 0.6875rem;
    font-weight: 600;
    color: var(--muted-foreground);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 16px 14px 8px;
}
.side-nav-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 0 8px;
}
.side-nav-item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 10px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--muted-foreground);
    font-size: 0.8125rem;
    font-weight: 500;
    cursor: pointer;
    text-align: left;
    outline: none;
    transition:
        background 0.15s,
        color 0.15s;
}
.side-nav-item:hover {
    background: var(--accent);
    color: var(--accent-foreground);
}
.side-nav-item.active {
    background: var(--primary);
    color: var(--primary-foreground);
}
.side-nav-item:focus-visible {
    box-shadow: 0 0 0 2px var(--ring);
}
.side-nav-footer {
    padding: 0px 8px;
}

/* ── 窄屏：保持左侧、压缩为 56px 图标条（与 AppLayout 断点一致；不放底部）── */
@media (max-width: 720px) {
    .side-nav {
        width: 56px;
    }
    .side-nav-title {
        display: none;
    }
    .side-nav-list,
    .side-nav-footer {
        padding: 0 4px;
    }
    .side-nav-item {
        justify-content: center;
        padding: 8px 0;
    }
    .side-nav-item-label {
        display: none;
    }
}
</style>
