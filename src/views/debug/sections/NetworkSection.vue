<script setup lang="ts">
/**
 * 网络请求（规格 §8.3）：
 * - 2s 轮询 get_network_logs（暂停按钮停轮询），展示最近 500 条；
 * - 表格：时间 / 方法 / URL（截断，悬停 title 完整）/ 状态码 / 耗时 / 主播 ID / 错误；
 * - 行背景按状态码着色（200 绿 / 429 黄 / 5xx 与失败 红）；
 * - 「清空记录」（clear_network_logs）。
 */
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { Eraser, Pause, Play } from "@lucide/vue";
import { api } from "@/services/api";
import type { NetworkLog } from "@/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { formatTime, statusRowClass, usePolling } from "./shared";
import SectionCard from "./SectionCard.vue";

const { t } = useI18n();

// 显示顺序：最新在最上方（后端 get_network_logs 已按倒叙返回，前端不再反转）
const logs = ref<NetworkLog[]>([]);
const paused = ref(false);
const errorMsg = ref<string | null>(null);

async function refresh() {
  try {
    const list = await api.getNetworkLogs();
    logs.value = list;
    errorMsg.value = null;
  } catch (e) {
    errorMsg.value = String(e);
  }
}

// 暂停时跳过轮询（手动刷新按钮仍可用）
usePolling(refresh, 2000, () => !paused.value);

async function clearLogs() {
  try {
    await api.clearNetworkLogs();
    logs.value = [];
  } catch (e) {
    errorMsg.value = String(e);
  }
}

/** 状态码徽章着色：2xx 绿 / 429 黄 / 5xx 与 0 红 / 其余中性 */
function statusBadgeClass(status: number): string {
  if (status >= 200 && status < 300)
    return "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400";
  if (status === 429)
    return "bg-amber-500/15 text-amber-600 dark:text-amber-400";
  if (status >= 500 || status === 0)
    return "bg-red-500/15 text-red-600 dark:text-red-400";
  return "bg-muted text-muted-foreground";
}

const statusText = (status: number) => (status === 0 ? "ERR" : String(status));

const totalCount = computed(() => logs.value.length);
</script>

<template>
    <SectionCard
        :title="t('debug.nav.network')"
        :subtitle="t('debug.network.subtitle')"
        refreshable
        @refresh="refresh"
    >
        <div class="mb-2 flex flex-wrap items-center gap-2">
            <Button variant="outline" size="xs" @click="paused = !paused">
                <Pause v-if="!paused" class="size-3" />
                <Play v-else class="size-3" />
                {{
                    paused
                        ? t("debug.network.resumePolling")
                        : t("debug.network.pausePolling")
                }}
            </Button>
            <Button variant="outline" size="xs" @click="clearLogs">
                <Eraser class="size-3" />{{ t("debug.network.clear") }}
            </Button>
            <span class="ml-auto text-xs text-muted-foreground">
                {{ t("debug.network.count", { total: totalCount }) }}
            </span>
        </div>

        <div v-if="errorMsg" class="mb-2 rounded-md border border-red-500/40 bg-red-500/10 px-3 py-2 text-xs text-red-600 dark:text-red-400">
            {{ errorMsg }}
        </div>

        <div v-if="logs.length === 0" class="rounded-md border border-dashed p-8 text-center text-sm text-muted-foreground">
            {{ t("debug.network.empty") }}
        </div>

        <div v-else class="overflow-x-auto rounded-md border max-w-full">
            <Table class="table-fixed w-full">
                <TableHeader>
                    <TableRow class="hover:bg-transparent">
                        <TableHead class="w-20 whitespace-nowrap">{{ t("debug.network.time") }}</TableHead>
                        <TableHead class="w-16">{{ t("debug.network.method") }}</TableHead>
                        <TableHead>{{ t("debug.network.url") }}</TableHead>
                        <TableHead class="w-16 text-right">{{ t("debug.network.status") }}</TableHead>
                        <TableHead class="w-20 text-right">{{ t("debug.network.duration") }}</TableHead>
                        <TableHead class="w-24">{{ t("debug.network.anchorId") }}</TableHead>
                    </TableRow>
                </TableHeader>
                <TableBody>
                    <TableRow
                        v-for="(log, idx) in logs"
                        :key="log.timestamp + '-' + idx"
                        :class="statusRowClass(log.status)"
                    >
                        <TableCell class="whitespace-nowrap font-mono text-xs text-muted-foreground">
                            {{ formatTime(log.timestamp) }}
                        </TableCell>
                        <TableCell class="font-mono text-xs">{{ log.method }}</TableCell>
                        <TableCell class="max-w-80 font-mono text-xs">
                            <span class="block truncate" :title="log.url">{{ log.url }}</span>
                            <span v-if="log.error" class="block text-xs text-red-500" :title="log.error">
                                {{ log.error }}
                            </span>
                        </TableCell>
                        <TableCell class="text-right">
                            <Badge :class="statusBadgeClass(log.status)">{{ statusText(log.status) }}</Badge>
                        </TableCell>
                        <TableCell class="text-right font-mono text-xs tabular-nums">
                            {{ log.duration_ms }}ms
                        </TableCell>
                        <TableCell class="font-mono text-xs">
                            {{ log.anchor_id ?? "—" }}
                        </TableCell>
                    </TableRow>
                </TableBody>
            </Table>
        </div>

        <p v-if="logs.some((l) => l.error)" class="mt-2 text-xs text-muted-foreground">
            {{ t("debug.network.errorHint") }}
        </p>
    </SectionCard>
</template>
