<script setup lang="ts">
/**
 * 检测循环（规格 §8.4）：
 * - 运行状态（绿「运行中」/ 红「已停止」）+ Mock 模式 "Mock" 标签（规格 Mock 章节）；
 * - 上次检测时间 + 统计（总 / 成功 / 失败 / 启用主播数 / 直播中 / 录制中）；
 * - 操作：手动触发检测 / 重置统计。
 */
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { Radar, RotateCcw, Zap } from "@lucide/vue";
import { api } from "@/services/api";
import { useMockStore } from "@/stores/mockStore";
import type { DetectorStatsSnapshot } from "@/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { formatTime, usePolling } from "./shared";
import SectionCard from "./SectionCard.vue";

const { t } = useI18n();
const mockStore = useMockStore();

const stats = ref<DetectorStatsSnapshot | null>(null);
const errorMsg = ref<string | null>(null);
const actionMsg = ref<string | null>(null);

async function refresh() {
  try {
    stats.value = await api.getDetectorStats();
    errorMsg.value = null;
  } catch (e) {
    errorMsg.value = String(e);
  }
}

usePolling(refresh, 2000);

async function triggerNow() {
  actionMsg.value = null;
  try {
    await api.triggerDetectionNow();
    actionMsg.value = t("debug.detector.triggered");
    await refresh();
  } catch (e) {
    actionMsg.value = t("debug.common.operationFailed", { error: String(e) });
  }
}

async function resetStats() {
  actionMsg.value = null;
  try {
    await api.resetDetectorStats();
    actionMsg.value = t("debug.detector.resetDone");
    await refresh();
  } catch (e) {
    actionMsg.value = t("debug.common.operationFailed", { error: String(e) });
  }
}

const statCards = () => {
  const s = stats.value;
  return [
    { label: t("debug.detector.totalChecks"), value: s?.total_checks ?? 0 },
    { label: t("debug.detector.successChecks"), value: s?.success_checks ?? 0 },
    { label: t("debug.detector.failedChecks"), value: s?.failed_checks ?? 0 },
    { label: t("debug.detector.unknownChecks"), value: s?.unknown_checks ?? 0 },
    { label: t("debug.detector.enabledAnchors"), value: s?.enabled_anchors ?? 0 },
    { label: t("debug.detector.liveAnchors"), value: s?.live_anchors ?? 0 },
    { label: t("debug.detector.recordingAnchors"), value: s?.recording_anchors ?? 0 },
  ];
};
</script>

<template>
    <SectionCard
        :title="t('debug.nav.detector')"
        :subtitle="t('debug.detector.subtitle')"
        refreshable
        @refresh="refresh"
    >
        <!-- 状态行 -->
        <div class="mb-4 flex flex-wrap items-center gap-2">
            <Badge
                :class="
                    stats?.running
                        ? 'bg-emerald-500/15 text-emerald-600 dark:text-emerald-400'
                        : 'bg-red-500/15 text-red-600 dark:text-red-400'
                "
            >
                <span
                    class="mr-1.5 inline-block size-1.5 rounded-full"
                    :class="stats?.running ? 'bg-emerald-500' : 'bg-red-500'"
                />
                {{ stats?.running ? t("debug.detector.running") : t("debug.detector.stopped") }}
            </Badge>
            <!-- Mock 模式标签（规格：Mock 开启时显示明显标签） -->
            <Badge
                v-if="mockStore.enabled"
                class="bg-amber-500/20 text-amber-600 dark:text-amber-400"
            >
                Mock
            </Badge>
            <span class="text-sm text-muted-foreground">
                {{ t("debug.detector.lastCheck", { time: formatTime(stats?.last_check_at) }) }}
            </span>
        </div>

        <div v-if="errorMsg" class="mb-3 rounded-md border border-red-500/40 bg-red-500/10 px-3 py-2 text-xs text-red-600 dark:text-red-400">
            {{ errorMsg }}
        </div>

        <!-- 统计 -->
        <div class="grid grid-cols-2 gap-2 sm:grid-cols-3">
            <div
                v-for="item in statCards()"
                :key="item.label"
                class="rounded-md border bg-muted/30 px-3 py-2"
            >
                <p class="text-xs text-muted-foreground">{{ item.label }}</p>
                <p class="font-mono text-lg font-semibold tabular-nums">{{ item.value }}</p>
            </div>
        </div>

        <!-- 操作 -->
        <div class="mt-4 flex flex-wrap gap-2">
            <Button size="sm" @click="triggerNow">
                <Zap class="size-3.5" />{{ t("debug.detector.triggerNow") }}
            </Button>
            <Button variant="outline" size="sm" @click="resetStats">
                <RotateCcw class="size-3.5" />{{ t("debug.detector.resetStats") }}
            </Button>
            <span class="inline-flex items-center text-xs text-muted-foreground">
                <Radar class="mr-1 size-3" />
                {{ t("debug.detector.pollingNote") }}
            </span>
        </div>
        <p v-if="actionMsg" class="mt-2 text-xs text-muted-foreground" role="status">
            {{ actionMsg }}
        </p>
    </SectionCard>
</template>
