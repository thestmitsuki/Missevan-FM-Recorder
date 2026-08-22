<script setup lang="ts">
/**
 * 主播状态徽标（规格「直播页面」状态徽标）
 *
 * live=直播中：强调色（--primary）淡底徽标 + 呼吸灯圆点动画；
 * recording=录制中：种子混合色（--seed-recording = destructive 70% + primary 30%）
 * 淡底徽标 + 呼吸灯圆点动画（与直播徽标同脉冲节奏）。
 * 两态色相不同（直播=纯强调色，录制=种子混合色，种子色权重 70%）可快速区分，
 * 且录制含 30% 强调色成分跟随主题；图标 Radio/CircleDot 双通道辅助区分。
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
        <!-- 直播中（强调色淡底 + 呼吸灯圆点）；录制中时隐藏（录制优先，只显示一个）
        颜色：--primary 强调色淡底（bg-primary/10）
        与录制中差异：直播=纯强调色（primary），录制=种子混合色
        （destructive 70% + primary 30%），色相不同一眼可辨 -->
        <Badge
            v-if="live && !recording"
            variant="outline"
            class="border-primary/40 bg-primary/10 text-primary"
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

        <!-- 录制中（种子混合色淡底 + 呼吸圆点，同直播徽标动画）
        颜色：--seed-recording = destructive 70% + primary 30%（种子色权重 70%），
        与直播中（纯 primary）色相不同可快速区分；含 30% 强调色成分跟随主题 -->
        <Badge
            v-if="recording"
            variant="outline"
            class="border-seed-recording/40 bg-seed-recording/10 text-seed-recording"
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
