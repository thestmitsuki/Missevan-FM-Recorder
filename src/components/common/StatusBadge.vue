<script setup lang="ts">
/**
 * 主播状态徽标（规格「直播页面」状态徽标）
 *
 * live=直播中：红色徽标 + 呼吸灯圆点动画；
 * recording=录制中：蓝色徽标 + 呼吸灯圆点动画（与直播红徽标同脉冲节奏，
 * 前端优化任务：录制徽标由常亮改为动态呼吸，组件级生效——卡片/列表/Sheet 统一）。
 * 统一规则（规格「只显示一个」）：直播 + 录制同时存在时**只显示录制**
 * （录制中更重要，recording ? recording-only : live）。
 * 规则在组件内一次性实施，所有消费方（卡片/列表/设置面板）自动生效。
 * 圆点 + 图标 + 文字双通道呈现，不只靠颜色区分，色觉友好。
 * 纯展示组件，无业务逻辑。
 */
import { useI18n } from "vue-i18n";
import { CircleDot, Radio } from "@lucide/vue";

import { Badge } from "@/components/ui/badge";

const props = withDefaults(
    defineProps<{
        live?: boolean;
        recording?: boolean;
        size?: "sm" | "md";
        /** 是否竖直堆叠（录制在直播下方） */
        stacked?: boolean;
    }>(),
    {
        live: false,
        recording: false,
        size: "md",
        stacked: false,
    },
);

const { t } = useI18n();
</script>

<template>
    <!-- Task 20 a11y：状态徽标动态出现/变化经 aria-live 播报（规格 §7.1） -->
    <span
        class="inline-flex gap-1.5"
        :class="stacked ? 'flex-col items-end' : 'items-center'"
        role="status"
        aria-live="polite"
    >
        <!-- 直播中（红色 + 呼吸灯圆点）；录制中时隐藏（录制优先，只显示一个） -->
        <Badge
            v-if="live && !recording"
            variant="outline"
            class="border-red-500/40 bg-red-500/10 text-red-600 dark:text-red-400"
            :class="
                size === 'sm'
                    ? 'gap-1 px-2 py-0.5 text-xs'
                    : 'gap-1.5 px-2.5 py-1 text-xs'
            "
        >
            <span
                class="status-dot"
                :class="size === 'sm' ? 'size-1.5' : 'size-2'"
                aria-hidden="true"
            />
            <Radio
                class="shrink-0"
                :class="size === 'sm' ? 'size-3' : 'size-3.5'"
                aria-hidden="true"
            />
            {{ t("live.liveNow") }}
        </Badge>

        <!-- 录制中（蓝色 + 呼吸圆点，同直播红徽标动画） -->
        <Badge
            v-if="recording"
            variant="outline"
            class="border-blue-500/40 bg-blue-500/10 text-blue-600 dark:text-blue-400"
            :class="
                size === 'sm'
                    ? 'gap-1 px-2 py-0.5 text-xs'
                    : 'gap-1.5 px-2.5 py-1 text-xs'
            "
        >
            <span
                class="status-dot"
                :class="size === 'sm' ? 'size-1.5' : 'size-2'"
                aria-hidden="true"
            />
            <CircleDot
                class="shrink-0"
                :class="size === 'sm' ? 'size-3' : 'size-3.5'"
                aria-hidden="true"
            />
            {{ t("live.recording") }}
        </Badge>
    </span>
</template>

<style scoped>
/* 呼吸灯圆点：颜色继承文字色（红/蓝），由徽标类控制。
   录制徽标与直播徽标共用同一脉冲动画（scale + opacity），
   prefers-reduced-motion 下关闭（见下方媒体查询）。 */
.status-dot {
    border-radius: 9999px;
    background: currentColor;
    animation: status-pulse 1.2s ease-in-out infinite;
}

@keyframes status-pulse {
    0%,
    100% {
        opacity: 1;
        transform: scale(1);
    }
    50% {
        opacity: 0.45;
        transform: scale(0.75);
    }
}

@media (prefers-reduced-motion: reduce) {
    .status-dot {
        animation: none;
    }
}
</style>
