<script setup lang="ts">
/**
 * 运行概览（规格 §8.1）：
 * - 系统信息（应用版本 / Rust / Tauri / 操作系统）+ 配置摘要（只读）；
 * - 心跳指示器：轮询 get_debug_info 成功=绿色，失败=红色警告；
 * - 快速操作：强制刷新文件缓存 / 立即检测 / 清空日志 / 导出诊断报告。
 */
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { Download, Eraser, HeartPulse, Radar, RefreshCw } from "@lucide/vue";
import { api } from "@/services/api";
import { useConfigStore } from "@/stores/configStore";
import type { DebugInfo } from "@/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { formatTime, usePolling } from "./shared";
import SectionCard from "./SectionCard.vue";

const { t } = useI18n();
const configStore = useConfigStore();

const info = ref<DebugInfo | null>(null);
const heartbeatOk = ref(false);
const lastHeartbeat = ref("");
const errorMsg = ref<string | null>(null);
const actionMsg = ref<string | null>(null);

async function refresh() {
  try {
    info.value = await api.getDebugInfo();
    heartbeatOk.value = true;
    errorMsg.value = null;
  } catch (e) {
    heartbeatOk.value = false;
    errorMsg.value = String(e);
  } finally {
    lastHeartbeat.value = formatTime(new Date().toISOString());
  }
}

usePolling(refresh, 2000);

// ── 配置摘要（进页面加载一次；失败静默）──
const cfg = computed(() => configStore.config);
async function loadConfig() {
  try {
    await configStore.fetchConfig();
  } catch {
    // 后端不可用时保留默认值
  }
}
void loadConfig();

// ── 快速操作 ──
async function runAction(fn: () => Promise<void>, okKey: string) {
  actionMsg.value = null;
  try {
    await fn();
    actionMsg.value = t(okKey);
  } catch (e) {
    actionMsg.value = t("debug.common.operationFailed", { error: String(e) });
  }
}

function refreshFileCache() {
  return runAction(() => api.refreshRecordingFiles(), "debug.overview.fileCacheRefreshed");
}

function triggerDetection() {
  return runAction(() => api.triggerDetectionNow(), "debug.overview.detectionTriggered");
}

function clearLogs() {
  return runAction(() => api.clearLogs(), "debug.overview.logsCleared");
}

async function exportReport() {
  actionMsg.value = null;
  try {
    const json = await api.exportDiagnosticReport();
    const blob = new Blob([json], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `diagnostic-report-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-")}.json`;
    a.click();
    URL.revokeObjectURL(url);
    actionMsg.value = t("debug.overview.reportExported");
  } catch (e) {
    actionMsg.value = t("debug.common.operationFailed", { error: String(e) });
  }
}

const statItems = computed(() => {
  const i = info.value;
  if (!i) return [];
  return [
    { label: t("debug.overview.activeRecordings"), value: String(i.active_recordings) },
    { label: t("debug.overview.enabledAnchors"), value: String(i.enabled_anchors) },
    { label: t("debug.overview.liveAnchors"), value: String(i.live_anchors) },
    { label: t("debug.overview.fileCount"), value: String(i.file_count) },
    { label: t("debug.overview.totalChecks"), value: String(i.total_checks) },
    { label: t("debug.overview.failedChecks"), value: String(i.failed_checks) },
  ];
});

const configRows = computed(() => {
  const c = cfg.value;
  return [
    { label: t("debug.overview.outputDir"), value: c.output_dir || "—" },
    { label: t("debug.overview.recordFormat"), value: c.record_format.toUpperCase() },
    { label: t("debug.overview.checkInterval"), value: `${c.check_interval_secs}s` },
    {
      label: t("debug.overview.maxConcurrent"),
      value: String(c.max_concurrent_recordings),
    },
    { label: t("debug.overview.logLevel"), value: c.log_level },
  ];
});

// ── FFmpeg / ffprobe 状态（系统信息区）──
/** 工具状态展示文案：未找到 → 本地化「未找到」；找到但无版本 → 路径；否则版本号 */
function toolLabel(s?: { found: boolean; path: string; version: string | null }): string {
  if (!s) return "—";
  if (!s.found) return t("debug.overview.toolNotFound");
  return s.version ?? s.path;
}

/** 工具状态颜色：找到 → 正常色；未找到 → 警示色 */
function toolClass(s?: { found: boolean }): string {
  return s?.found
    ? "text-emerald-700 dark:text-emerald-400"
    : "text-destructive";
}
</script>

<template>
    <SectionCard
        :title="t('debug.nav.overview')"
        :subtitle="t('debug.overview.subtitle')"
        refreshable
        :collapsible="false"
        @refresh="refresh"
    >
        <!-- 心跳指示器 -->
        <div
            class="mb-4 flex items-center gap-2 rounded-md border px-3 py-2 text-sm"
            :class="
                heartbeatOk
                    ? 'border-emerald-500/40 bg-emerald-500/5'
                    : 'border-destructive/50 bg-destructive/10'
            "
            role="status"
        >
            <HeartPulse
                :class="heartbeatOk ? 'text-emerald-500' : 'text-destructive'"
                class="size-4 shrink-0"
            />
            <span :class="heartbeatOk ? 'text-emerald-700 dark:text-emerald-400' : 'text-destructive'">
                {{ heartbeatOk ? t("debug.overview.heartbeatOk") : t("debug.overview.heartbeatFail") }}
            </span>
            <span v-if="lastHeartbeat" class="ml-auto text-xs text-muted-foreground">
                {{ t("debug.overview.lastHeartbeat", { time: lastHeartbeat }) }}
            </span>
            <Badge v-if="info?.mock_mode" class="bg-amber-500/15 text-amber-600 dark:text-amber-400">
                Mock
            </Badge>
        </div>

        <div v-if="errorMsg" class="mb-4 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive">
            {{ errorMsg }}
        </div>

        <!-- 统计卡片 -->
        <div class="grid grid-cols-2 gap-2 sm:grid-cols-3">
            <div
                v-for="item in statItems"
                :key="item.label"
                class="rounded-md border bg-muted/30 px-3 py-2"
            >
                <p class="text-xs text-muted-foreground">{{ item.label }}</p>
                <p class="font-mono text-lg font-semibold tabular-nums">{{ item.value }}</p>
            </div>
        </div>

        <!-- 系统信息 -->
        <h4 class="mb-2 mt-5 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            {{ t("debug.overview.systemInfo") }}
        </h4>
        <dl class="grid grid-cols-1 gap-x-6 gap-y-1.5 text-sm sm:grid-cols-2">
            <div class="flex justify-between gap-4">
                <dt class="shrink-0 text-muted-foreground">{{ t("debug.overview.appVersion") }}</dt>
                <dd class="truncate font-mono text-xs leading-6">{{ info?.app_version ?? "—" }}</dd>
            </div>
            <div class="flex justify-between gap-4">
                <dt class="shrink-0 text-muted-foreground">{{ t("debug.overview.rustVersion") }}</dt>
                <dd class="truncate font-mono text-xs leading-6">{{ info?.rust_version ?? "—" }}</dd>
            </div>
            <div class="flex justify-between gap-4">
                <dt class="shrink-0 text-muted-foreground">{{ t("debug.overview.tauriVersion") }}</dt>
                <dd class="truncate font-mono text-xs leading-6">{{ info?.tauri_version ?? "—" }}</dd>
            </div>
            <div class="flex justify-between gap-4">
                <dt class="shrink-0 text-muted-foreground">{{ t("debug.overview.os") }}</dt>
                <dd class="truncate font-mono text-xs leading-6">{{ info?.os ?? "—" }}</dd>
            </div>
            <div class="flex justify-between gap-4">
                <dt class="shrink-0 text-muted-foreground">{{ t("debug.overview.ffmpeg") }}</dt>
                <dd class="truncate font-mono text-xs leading-6" :class="toolClass(info?.ffmpeg_status)">
                    {{ toolLabel(info?.ffmpeg_status) }}
                </dd>
            </div>
            <div class="flex justify-between gap-4">
                <dt class="shrink-0 text-muted-foreground">{{ t("debug.overview.ffprobe") }}</dt>
                <dd class="truncate font-mono text-xs leading-6" :class="toolClass(info?.ffprobe_status)">
                    {{ toolLabel(info?.ffprobe_status) }}
                </dd>
            </div>
        </dl>

        <!-- 配置摘要 -->
        <h4 class="mb-2 mt-5 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            {{ t("debug.overview.configSummary") }}
        </h4>
        <dl class="grid grid-cols-1 gap-x-6 gap-y-1.5 text-sm sm:grid-cols-2">
            <div
                v-for="row in configRows"
                :key="row.label"
                class="flex justify-between gap-4"
            >
                <dt class="shrink-0 text-muted-foreground">{{ row.label }}</dt>
                <dd class="truncate font-mono text-xs leading-6">{{ row.value }}</dd>
            </div>
        </dl>

        <!-- 快速操作 -->
        <h4 class="mb-2 mt-5 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            {{ t("debug.overview.quickActions") }}
        </h4>
        <div class="flex flex-wrap gap-2">
            <Button variant="outline" size="sm" @click="refreshFileCache">
                <RefreshCw class="size-3.5" />{{ t("debug.overview.refreshFileCache") }}
            </Button>
            <Button variant="outline" size="sm" @click="triggerDetection">
                <Radar class="size-3.5" />{{ t("debug.overview.runDetectionNow") }}
            </Button>
            <Button variant="outline" size="sm" @click="clearLogs">
                <Eraser class="size-3.5" />{{ t("debug.overview.clearLogs") }}
            </Button>
            <Button variant="outline" size="sm" @click="exportReport">
                <Download class="size-3.5" />{{ t("debug.overview.exportReport") }}
            </Button>
        </div>
        <p v-if="actionMsg" class="mt-2 text-xs text-muted-foreground" role="status">
            {{ actionMsg }}
        </p>
    </SectionCard>
</template>
