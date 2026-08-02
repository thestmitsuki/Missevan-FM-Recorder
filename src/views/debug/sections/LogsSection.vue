<script setup lang="ts">
/**
 * 实时日志（规格 §8.2）：
 * - 初始加载 get_logs + `debug:log` 事件实时追加（环形缓冲 1000 条）；
 * - 级别多选过滤（Error/Warn/Info/Debug/Trace）+ 来源子串过滤（下拉快选 + 手动输入）
 *   + 搜索关键字高亮（<mark>，不引入 v-html）；
 * - 自动滚动（滚动到顶自动暂停；按钮恢复）；
 * - 清空（后端 clear_logs + 本地）、复制当前筛选结果到剪贴板。
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { Copy, Eraser, Pause, Play } from "@lucide/vue";
import { api } from "@/services/api";
import { onDebugLog } from "@/services/events";
import type { LogEntry } from "@/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { levelActiveClass, levelClass, formatTime } from "./shared";
import SectionCard from "./SectionCard.vue";

const { t } = useI18n();

const LOG_LEVELS = ["trace", "debug", "info", "warn", "error"] as const;
const MAX_LOGS = 1000;

// 显示顺序：新日志在末尾（自动滚动到底部阅读）
const logs = ref<LogEntry[]>([]);
const levelFilter = ref<Set<string>>(new Set());
const sourceFilter = ref("");
const search = ref("");
const autoScroll = ref(true);
const copied = ref(false);
const logArea = ref<HTMLElement | null>(null);
const errorMsg = ref<string | null>(null);

let unlisten: (() => void) | null = null;
let copiedTimer: ReturnType<typeof setTimeout> | null = null;

// ── 过滤 ──
function toggleLevel(level: string) {
  const next = new Set(levelFilter.value);
  if (next.has(level)) next.delete(level);
  else next.add(level);
  levelFilter.value = next;
}

const filteredLogs = computed(() => {
  let list = logs.value;
  if (levelFilter.value.size > 0) {
    list = list.filter((e) => levelFilter.value.has(e.level));
  }
  const src = sourceFilter.value.trim().toLowerCase();
  if (src) {
    list = list.filter((e) => e.module.toLowerCase().includes(src));
  }
  const q = search.value.trim().toLowerCase();
  if (q) {
    list = list.filter((e) => e.message.toLowerCase().includes(q));
  }
  return list;
});

/** 手动刷新：重新拉取后端日志缓冲（事件流可能被节流丢弃，刷新可补齐） */
async function manualRefresh() {
  errorMsg.value = null;
  try {
    const initial = await api.getLogs();
    logs.value = [...initial].reverse();
  } catch (e) {
    errorMsg.value = String(e);
  }
}

/** 搜索高亮切段：命中部分标记 hit */
function highlightSegments(text: string): { text: string; hit: boolean }[] {
  const q = search.value.trim();
  if (!q) return [{ text, hit: false }];
  const lower = text.toLowerCase();
  const ql = q.toLowerCase();
  const out: { text: string; hit: boolean }[] = [];
  let i = 0;
  for (;;) {
    const idx = lower.indexOf(ql, i);
    if (idx < 0) {
      if (i < text.length) out.push({ text: text.slice(i), hit: false });
      break;
    }
    if (idx > i) out.push({ text: text.slice(i, idx), hit: false });
    out.push({ text: text.slice(idx, idx + q.length), hit: true });
    i = idx + q.length;
  }
  return out;
}

// ── 自动滚动 ──
function scrollToBottom() {
  const el = logArea.value;
  if (el) el.scrollTop = el.scrollHeight;
}

watch(filteredLogs, async () => {
  if (autoScroll.value) {
    await nextTick();
    scrollToBottom();
  }
});

function onScroll() {
  const el = logArea.value;
  if (!el || !autoScroll.value) return;
  const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 32;
  if (!nearBottom) autoScroll.value = false; // 用户向上滚动：自动暂停
}

function toggleAutoScroll() {
  autoScroll.value = !autoScroll.value;
  if (autoScroll.value) scrollToBottom();
}

// ── 操作 ──
async function clearLogs() {
  errorMsg.value = null;
  try {
    await api.clearLogs();
    logs.value = [];
  } catch (e) {
    errorMsg.value = String(e);
  }
}

async function copyAll() {
  const text = filteredLogs.value
    .map((e) => `[${e.timestamp}] [${e.level}] [${e.module}] ${e.message}`)
    .join("\n");
  if (!text) return;
  try {
    await navigator.clipboard.writeText(text);
    copied.value = true;
    if (copiedTimer) clearTimeout(copiedTimer);
    copiedTimer = setTimeout(() => (copied.value = false), 2000);
  } catch {
    errorMsg.value = String(t("debug.logs.copyFailed"));
  }
}

// ── 初始化：get_logs 初始快照 + `debug:log` 事件流 ──
onMounted(async () => {
  try {
    const initial = await api.getLogs();
    // 后端最新在前 → 反转后追加（新在末尾）
    logs.value = [...initial].reverse();
  } catch (e) {
    errorMsg.value = String(e);
  }
  unlisten = onDebugLog((entry) => {
    logs.value.push(entry);
    if (logs.value.length > MAX_LOGS) {
      logs.value.splice(0, logs.value.length - MAX_LOGS);
    }
  });
});

onBeforeUnmount(() => {
  unlisten?.();
  if (copiedTimer) clearTimeout(copiedTimer);
});
</script>

<template>
    <SectionCard
        :title="t('debug.nav.logs')"
        :subtitle="t('debug.logs.subtitle')"
        refreshable
        @refresh="manualRefresh"
    >
        <!-- 工具栏：级别 / 来源 / 搜索 -->
        <div class="mb-3 flex flex-wrap items-center gap-2">
            <div class="flex flex-wrap items-center gap-1.5">
                <button
                    v-for="lvl in LOG_LEVELS"
                    :key="lvl"
                    type="button"
                    class="rounded-full border px-2.5 py-0.5 text-xs font-medium transition-colors"
                    :class="
                        levelFilter.has(lvl)
                            ? levelActiveClass(lvl)
                            : 'border-input text-muted-foreground hover:bg-accent'
                    "
                    @click="toggleLevel(lvl)"
                >
                    {{ lvl }}
                </button>
                <button
                    v-if="levelFilter.size > 0"
                    type="button"
                    class="text-xs text-muted-foreground underline underline-offset-2 hover:text-foreground"
                    @click="levelFilter = new Set()"
                >
                    {{ t("debug.logs.clearLevels") }}
                </button>
            </div>

            <Input
                v-model="sourceFilter"
                :placeholder="t('debug.logs.sourceFilterPlaceholder')"
                class="h-8 w-48"
            />

            <Input
                v-model="search"
                :placeholder="t('debug.logs.searchPlaceholder')"
                class="h-8 w-44"
            />
        </div>

        <!-- 操作行 -->
        <div class="mb-2 flex flex-wrap items-center gap-2">
            <Button variant="outline" size="xs" @click="toggleAutoScroll">
                <Pause v-if="autoScroll" class="size-3" />
                <Play v-else class="size-3" />
                {{
                    autoScroll
                        ? t("debug.logs.pauseScroll")
                        : t("debug.logs.resumeScroll")
                }}
            </Button>
            <Button variant="outline" size="xs" @click="clearLogs">
                <Eraser class="size-3" />{{ t("debug.logs.clear") }}
            </Button>
            <Button variant="outline" size="xs" @click="copyAll">
                <Copy class="size-3" />
                {{ copied ? t("debug.logs.copied") : t("debug.logs.copyAll") }}
            </Button>
            <span class="ml-auto text-xs text-muted-foreground">
                {{ t("debug.logs.count", { shown: filteredLogs.length, total: logs.length }) }}
            </span>
        </div>

        <div v-if="errorMsg" class="mb-2 rounded-md border border-red-500/40 bg-red-500/10 px-3 py-2 text-xs text-red-600 dark:text-red-400">
            {{ errorMsg }}
        </div>

        <!-- 日志区域 -->
        <div
            ref="logArea"
            class="h-96 overflow-y-auto rounded-md border bg-black/90 p-2 font-mono text-xs leading-5 dark:bg-black"
            @scroll="onScroll"
        >
            <div v-if="logs.length === 0" class="p-4 text-center text-muted-foreground">
                {{ t("debug.logs.empty") }}
            </div>
            <div
                v-for="(entry, idx) in filteredLogs"
                :key="entry.timestamp + '-' + idx"
                class="flex gap-2 whitespace-pre-wrap break-all px-1 hover:bg-white/5"
            >
                <span class="shrink-0 text-zinc-500">{{ formatTime(entry.timestamp) }}</span>
                <span class="w-11 shrink-0 font-semibold uppercase" :class="levelClass(entry.level)">
                    {{ entry.level }}
                </span>
                <span class="max-w-40 shrink-0 truncate text-zinc-500" :title="entry.module">
                    {{ entry.module }}
                </span>
                <span class="min-w-0 text-zinc-100">
                    <template v-for="(seg, si) in highlightSegments(entry.message)" :key="si">
                        <mark
                            v-if="seg.hit"
                            class="rounded-sm bg-yellow-300/80 px-0.5 text-black"
                        >{{ seg.text }}</mark>
                        <template v-else>{{ seg.text }}</template>
                    </template>
                </span>
            </div>
        </div>
    </SectionCard>
</template>
