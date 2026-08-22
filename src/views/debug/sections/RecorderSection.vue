<script setup lang="ts">
/**
 * 录制引擎（规格 §8.5）：
 * - 活跃任务表：主播 / 房间号 / 状态 / 已录时长 / 输出文件 / PID + 「停止录制」；
 * - 录制历史（可折叠）：最近结束的 20 条摘要。
 *
 * 注：FFmpeg 命令行与实时 stderr 后端暂未提供（get_recorder_state 无该字段），
 * 已在任务报告注明；停止录制复用主播设置同款 stop_recording 命令。
 */
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { ChevronDown, ChevronUp, Square } from "@lucide/vue";
import { api } from "@/services/api";
import type { RecorderStateInfo } from "@/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
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
import { formatDuration, formatTime, usePolling } from "./shared";
import SectionCard from "./SectionCard.vue";

const { t } = useI18n();

const state = ref<RecorderStateInfo | null>(null);
const errorMsg = ref<string | null>(null);
const busyRoom = ref<string | null>(null);
const historyOpen = ref(true);

async function refresh() {
  try {
    state.value = await api.getRecorderState();
    errorMsg.value = null;
  } catch (e) {
    errorMsg.value = String(e);
  }
}

usePolling(refresh, 2000);

async function stopRecording(anchorId: string, roomId: string) {
  busyRoom.value = roomId;
  try {
    await api.stopRecording(anchorId);
    await refresh();
  } catch (e) {
    errorMsg.value = String(e);
  } finally {
    busyRoom.value = null;
  }
}

const statusBadgeClass = (status: string) =>
  status === "recording"
    ? "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400"
    : "bg-muted text-muted-foreground";
</script>

<template>
    <SectionCard
        :title="t('debug.nav.recorder')"
        :subtitle="t('debug.recorder.subtitle')"
        refreshable
        @refresh="refresh"
    >
        <div v-if="errorMsg" class="mb-3 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive">
            {{ errorMsg }}
        </div>

        <h4 class="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            {{ t("debug.recorder.activeTasks") }}
            <span v-if="state" class="ml-1 normal-case">
                ({{ state.active.length }})
            </span>
        </h4>

        <Empty
            v-if="!state || state.active.length === 0"
            class="rounded-md p-6 md:p-6"
        >
            <EmptyContent>
                <EmptyDescription>
                    {{ t("debug.recorder.noActiveTasks") }}
                </EmptyDescription>
            </EmptyContent>
        </Empty>

        <div v-else class="overflow-x-auto rounded-md border">
            <Table>
                <TableHeader>
                    <TableRow class="hover:bg-transparent">
                        <TableHead>{{ t("debug.recorder.anchor") }}</TableHead>
                        <TableHead class="w-20">{{ t("debug.recorder.room") }}</TableHead>
                        <TableHead class="w-20">{{ t("debug.recorder.status") }}</TableHead>
                        <TableHead class="w-24 text-right">{{ t("debug.recorder.duration") }}</TableHead>
                        <TableHead>{{ t("debug.recorder.outputFile") }}</TableHead>
                        <TableHead class="w-16">{{ t("debug.recorder.pid") }}</TableHead>
                        <TableHead class="w-20"></TableHead>
                    </TableRow>
                </TableHeader>
                <TableBody>
                    <TableRow v-for="rec in state.active" :key="rec.anchor_id">
                        <TableCell class="max-w-40 truncate font-medium" :title="rec.anchor_name">
                            {{ rec.anchor_name || rec.anchor_id }}
                        </TableCell>
                        <TableCell class="font-mono text-xs">{{ rec.room_id }}</TableCell>
                        <TableCell>
                            <Badge :class="statusBadgeClass(rec.status)">{{ rec.status }}</Badge>
                        </TableCell>
                        <TableCell class="text-right font-mono text-xs tabular-nums">
                            {{ formatDuration(rec.duration_secs) }}
                        </TableCell>
                        <TableCell class="max-w-56 truncate font-mono text-xs" :title="rec.output_path">
                            {{ rec.output_path }}
                        </TableCell>
                        <TableCell class="font-mono text-xs">{{ rec.pid ?? "—" }}</TableCell>
                        <TableCell class="text-right">
                            <Button
                                variant="destructive"
                                size="xs"
                                :disabled="busyRoom === rec.room_id"
                                @click="stopRecording(rec.anchor_id, rec.room_id)"
                            >
                                <Square class="size-2.5 fill-current" />
                                {{ t("debug.recorder.stop") }}
                            </Button>
                        </TableCell>
                    </TableRow>
                </TableBody>
            </Table>
        </div>

        <!-- 录制历史（可折叠） -->
        <Collapsible v-model:open="historyOpen" class="mt-5">
            <CollapsibleTrigger as-child>
                <Button
                    variant="ghost"
                    class="h-auto w-full justify-between rounded-md px-2 py-1.5 text-xs font-semibold uppercase tracking-wide text-muted-foreground"
                >
                    <span>
                        {{ t("debug.recorder.history") }}
                        <span v-if="state" class="ml-1 normal-case">({{ state.history.length }})</span>
                    </span>
                    <ChevronUp v-if="historyOpen" class="size-3.5" />
                    <ChevronDown v-else class="size-3.5" />
                </Button>
            </CollapsibleTrigger>

            <CollapsibleContent>
                <div class="mt-2 space-y-1.5">
                <Empty
                    v-if="!state || state.history.length === 0"
                    class="rounded-md p-4 md:p-4"
                >
                    <EmptyContent>
                        <EmptyDescription class="text-xs">
                            {{ t("debug.recorder.emptyHistory") }}
                        </EmptyDescription>
                    </EmptyContent>
                </Empty>
                <div
                    v-for="h in state?.history ?? []"
                    :key="h.anchor_id + h.ended_at"
                    class="flex items-center gap-3 rounded-md border bg-muted/20 px-3 py-1.5 text-xs"
                >
                    <span class="w-28 truncate font-medium" :title="h.anchor_name">
                        {{ h.anchor_name || h.anchor_id }}
                    </span>
                    <span class="w-16 shrink-0 font-mono text-muted-foreground">{{ h.room_id }}</span>
                    <span class="shrink-0 text-muted-foreground">
                        {{ formatTime(h.started_at) }} → {{ formatTime(h.ended_at) }}
                    </span>
                    <span class="shrink-0 font-mono tabular-nums">
                        {{ formatDuration(h.duration_secs) }}
                    </span>
                    <span class="min-w-0 flex-1 truncate font-mono text-muted-foreground" :title="h.output_path">
                        {{ h.output_path }}
                    </span>
                </div>
            </div>
            </CollapsibleContent>
        </Collapsible>
    </SectionCard>
</template>
