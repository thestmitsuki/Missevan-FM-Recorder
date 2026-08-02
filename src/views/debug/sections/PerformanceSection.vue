<script setup lang="ts">
/**
 * 性能监控（规格 §8.8）：占位入口，标注「实验性」。
 * 规格注明可选、依赖系统 API（CPU / 内存 / 磁盘 IO），设计文档决策跳过实现。
 */
import { useI18n } from "vue-i18n";
import { Activity } from "@lucide/vue";
import { Badge } from "@/components/ui/badge";
import SectionCard from "./SectionCard.vue";

const { t } = useI18n();
</script>

<template>
    <SectionCard
        :title="t('debug.nav.performance')"
        :subtitle="t('debug.performance.subtitle')"
        :collapsible="false"
    >
        <div class="flex flex-col items-center gap-3 rounded-md border border-dashed p-10 text-center">
            <Activity class="size-10 text-muted-foreground/50" />
            <Badge class="bg-amber-500/15 text-amber-600 dark:text-amber-400">
                {{ t("debug.performance.experimental") }}
            </Badge>
            <p class="max-w-md text-sm text-muted-foreground">
                {{ t("debug.performance.desc") }}
            </p>
        </div>
    </SectionCard>
</template>
