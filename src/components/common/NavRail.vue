<script setup lang="ts">
/**
 * 通用竖向操作栏（56px 竖排操作栏，规格「直播页面」操作栏一节）
 *
 * 配置式纯展示组件：每页通过 `items` 数组声明自己的按钮集
 * （直播页：添加主播 / 视图切换 / 筛选 / 回顶；文件页：搜索 / 刷新 / 筛选 / 回顶）。
 * 不持有业务状态——激活态、面板展开态、点击处理全部由调用方在配置中声明。
 *
 * 无障碍（Task 9/10 已验收能力，全部保留）：
 * - 原生 button（Tab 聚焦，Enter/Space 激活）；
 * - 每个按钮动态 aria-label（配置 label）与 aria-expanded（expanded）；
 * - 图标按钮右侧 Tooltip。
 */
import type { Component } from "vue";
import { ArrowUp } from "@lucide/vue";

import { Button } from "@/components/ui/button";
import {
    Tooltip,
    TooltipContent,
    TooltipProvider,
    TooltipTrigger,
} from "@/components/ui/tooltip";

/** 操作栏按钮配置 */
export interface NavRailItem {
    /** 唯一标识 */
    id: string;
    /** 图标组件（@lucide/vue）；使用 slotName 自定义触发器时可不提供 */
    icon?: Component;
    /** 已翻译的按钮文案（Tooltip + aria-label） */
    label: string;
    /** 激活态高亮（视图模式 / 筛选面板展开等） */
    active?: boolean;
    /** 主按钮：顶部强调色圆形（如「添加主播」） */
    primary?: boolean;
    /** aria-expanded（控制展开/收起面板的按钮） */
    expanded?: boolean;
    /** 禁用态 */
    disabled?: boolean;
    /** 自定义触发器插槽名：提供时渲染调用方插槽（用于 Popover 等组合触发），不渲染默认按钮 */
    slotName?: string;
    /** 点击处理（由调用方声明） */
    onClick?: () => void;
}

const props = withDefaults(
    defineProps<{
        /** 按钮配置数组 */
        items: NavRailItem[];
        /** 操作栏 nav 的 aria-label */
        ariaLabel?: string;
        /** 是否显示回顶按钮（内容区滚动超过阈值时由父组件控制显示） */
        showScrollTop?: boolean;
        /** 回顶按钮文案（Tooltip + aria-label） */
        scrollTopLabel?: string;
    }>(),
    {
        items: () => [],
        ariaLabel: "",
        showScrollTop: false,
        scrollTopLabel: "",
    },
);

const emit = defineEmits<{
    /** 回到顶部 */
    scrollTop: [];
}>();
</script>

<template>
    <TooltipProvider :delay-duration="200">
        <nav
            class="flex w-14 shrink-0 flex-col items-center gap-2 bg-background py-3 max-[720px]:w-12"
            :aria-label="props.ariaLabel"
        >
            <!-- 配置按钮区 -->
            <template v-for="item in props.items" :key="item.id">
                <!-- 自定义触发器（Popover 组合等）：由调用方插槽提供，仍置于 TooltipProvider 内 -->
                <slot v-if="item.slotName" :name="item.slotName" :item="item" />
                <Tooltip v-else>
                    <TooltipTrigger as-child>
                        <Button
                            size="icon"
                            :variant="item.primary ? 'default' : 'ghost'"
                            :class="[
                                item.primary
                                    ? 'size-11 rounded-full shadow-sm max-[720px]:size-10'
                                    : 'size-10 max-[720px]:size-9',
                                item.active ? 'bg-accent text-accent-foreground' : '',
                            ]"
                            :aria-label="item.label"
                            :aria-expanded="item.expanded ?? undefined"
                            :disabled="item.disabled"
                            @click="item.onClick"
                        >
                            <component
                                v-if="item.icon"
                                :is="item.icon"
                                class="size-5 max-[720px]:size-4"
                            />
                        </Button>
                    </TooltipTrigger>
                    <TooltipContent side="right">
                        {{ item.label }}
                    </TooltipContent>
                </Tooltip>
            </template>

            <!-- 弹性占位：回顶按钮贴底 -->
            <div class="flex-1" />

            <!-- 回到顶部（底部；内容区滚动超过阈值时由父组件控制显示） -->
            <Tooltip v-if="props.showScrollTop">
                <TooltipTrigger as-child>
                    <Button
                        size="icon"
                        variant="ghost"
                        class="size-10 max-[720px]:size-9"
                        :aria-label="props.scrollTopLabel"
                        @click="emit('scrollTop')"
                    >
                        <ArrowUp class="size-5 max-[720px]:size-4" />
                    </Button>
                </TooltipTrigger>
                <TooltipContent side="right">
                    {{ props.scrollTopLabel }}
                </TooltipContent>
            </Tooltip>
        </nav>
    </TooltipProvider>
</template>
