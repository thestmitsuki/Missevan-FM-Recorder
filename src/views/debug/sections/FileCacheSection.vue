<script setup lang="ts">
/**
 * 文件缓存（规格 §8.7）：
 * - 当前缓存状态：上次扫描时间 / 文件总数 / 分段组总数 / 总大小；
 * - 扫描日志（最近 20 次：耗时 / 新增 / 移除 / 分段组数）；
 * - 操作：「立即扫描」（同文件页刷新）与「清除缓存」（仅清内存索引，不动磁盘）。
 */
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { RefreshCw, Trash2 } from "@lucide/vue";
import { api } from "@/services/api";
import type { FileCacheState } from "@/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyContent,
  EmptyDescription,
} from "@/components/ui/empty";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { formatBytes, formatTime, usePolling } from "./shared";
import SectionCard from "./SectionCard.vue";

const { t } = useI18n();

const state = ref<FileCacheState | null>(null);
const errorMsg = ref<string | null>(null);
const actionMsg = ref<string | null>(null);
const busy = ref<"scan" | "clear" | null>(null);

async function refresh() {
  try {
    state.value = await api.getFileCacheState();
    errorMsg.value = null;
  } catch (e) {
    errorMsg.value = String(e);
  }
}

usePolling(refresh, 2000);

async function scanNow() {
  busy.value = "scan";
  actionMsg.value = null;
  try {
    await api.refreshRecordingFiles();
    actionMsg.value = t("debug.filecache.scanDone");
    await refresh();
  } catch (e) {
    actionMsg.value = t("debug.common.operationFailed", { error: String(e) });
  } finally {
    busy.value = null;
  }
}

async function clearCache() {
  busy.value = "clear";
  actionMsg.value = null;
  try {
    await api.clearFileCache();
    actionMsg.value = t("debug.filecache.clearDone");
    await refresh();
  } catch (e) {
    actionMsg.value = t("debug.common.operationFailed", { error: String(e) });
  } finally {
    busy.value = null;
  }
}

const infoCards = () => [
  {
    label: t("debug.filecache.lastScan"),
    value: formatTime(state.value?.last_scan_at),
  },
  { label: t("debug.filecache.fileCount"), value: String(state.value?.file_count ?? 0) },
  { label: t("debug.filecache.groupCount"), value: String(state.value?.group_count ?? 0) },
  {
    label: t("debug.filecache.totalSize"),
    value: formatBytes(state.value?.total_size_bytes ?? 0),
  },
];

const kindBadge = (kind: string) =>
  kind === "clear"
    ? "bg-amber-500/15 text-amber-600 dark:text-amber-400"
    : "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400";
</script>

<template>
    <SectionCard
        :title="t('debug.nav.filecache')"
        :subtitle="t('debug.filecache.subtitle')"
        refreshable
        @refresh="refresh"
    >
        <div v-if="errorMsg" class="mb-3 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive">
            {{ errorMsg }}
        </div>

        <div class="grid grid-cols-2 gap-2 sm:grid-cols-4">
            <div
                v-for="item in infoCards()"
                :key="item.label"
                class="rounded-md border bg-muted/30 px-3 py-2"
            >
                <p class="text-xs text-muted-foreground">{{ item.label }}</p>
                <p class="font-mono text-sm font-semibold tabular-nums">{{ item.value }}</p>
            </div>
        </div>

        <div class="mt-4 flex flex-wrap gap-2">
            <Button size="sm" :disabled="busy !== null" @click="scanNow">
                <RefreshCw :class="busy === 'scan' ? 'animate-spin' : ''" class="size-3.5" />
                {{ t("debug.filecache.scanNow") }}
            </Button>
            <Button variant="outline" size="sm" :disabled="busy !== null" @click="clearCache">
                <Trash2 class="size-3.5" />{{ t("debug.filecache.clearCache") }}
            </Button>
        </div>
        <p v-if="actionMsg" class="mt-2 text-xs text-muted-foreground" role="status">
            {{ actionMsg }}
        </p>

        <!-- 扫描日志 -->
        <h4 class="mb-2 mt-5 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            {{ t("debug.filecache.scanLog") }}
        </h4>
        <Empty
            v-if="!state || state.scan_log.length === 0"
            class="rounded-md p-6 md:p-6"
        >
            <EmptyContent>
                <EmptyDescription>
                    {{ t("debug.filecache.emptyLog") }}
                </EmptyDescription>
            </EmptyContent>
        </Empty>
        <div v-else class="overflow-x-auto rounded-md border">
            <Table>
                <TableHeader>
                    <TableRow class="hover:bg-transparent">
                        <TableHead class="w-36">{{ t("debug.filecache.time") }}</TableHead>
                        <TableHead class="w-16">{{ t("debug.filecache.kind") }}</TableHead>
                        <TableHead class="w-20 text-right">{{ t("debug.filecache.durationMs") }}</TableHead>
                        <TableHead class="text-right">{{ t("debug.filecache.files") }}</TableHead>
                        <TableHead class="w-20 text-right">{{ t("debug.filecache.groups") }}</TableHead>
                    </TableRow>
                </TableHeader>
                <TableBody>
                    <TableRow v-for="(log, idx) in state.scan_log" :key="log.timestamp + '-' + idx">
                        <TableCell class="whitespace-nowrap font-mono text-xs text-muted-foreground">
                            {{ formatTime(log.timestamp) }}
                        </TableCell>
                        <TableCell>
                            <Badge :class="kindBadge(log.kind)">{{ log.kind }}</Badge>
                        </TableCell>
                        <TableCell class="text-right font-mono text-xs tabular-nums">
                            {{ log.duration_ms }}
                        </TableCell>
                        <TableCell class="text-right font-mono text-xs tabular-nums">
                            {{ log.files_before }} → {{ log.files_after }}
                        </TableCell>
                        <TableCell class="text-right font-mono text-xs tabular-nums">
                            {{ log.groups }}
                        </TableCell>
                    </TableRow>
                </TableBody>
            </Table>
        </div>
    </SectionCard>
</template>
