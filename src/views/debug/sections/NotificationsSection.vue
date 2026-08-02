<script setup lang="ts">
/**
 * 通知历史（规格 §8.6）：
 * - 展示通知分发器的环形缓冲（前端 notificationStore 事件驱动，容量 50；
 *   后端历史缓冲命令未提供，任务报告注明）；
 * - 级别过滤（Info / Warning / Error / Critical 多选）+ 搜索文本；
 * - 「清空历史」。
 */
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { Eraser } from "@lucide/vue";
import { useNotificationStore } from "@/stores/notificationStore";
import type { NotificationLevel } from "@/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { formatTime } from "./shared";
import SectionCard from "./SectionCard.vue";

const { t } = useI18n();
const notificationStore = useNotificationStore();

const LEVELS: NotificationLevel[] = ["Info", "Warning", "Error", "Critical"];
const levelFilter = ref<Set<NotificationLevel>>(new Set());
const search = ref("");

function toggleLevel(level: NotificationLevel) {
  const next = new Set(levelFilter.value);
  if (next.has(level)) next.delete(level);
  else next.add(level);
  levelFilter.value = next;
}

const levelChipClass = (level: NotificationLevel, active: boolean) => {
  if (!active) return "border-input text-muted-foreground hover:bg-accent";
  switch (level) {
    case "Info":
      return "border-sky-500 bg-sky-500/15 text-sky-600 dark:text-sky-400";
    case "Warning":
      return "border-amber-500 bg-amber-500/15 text-amber-600 dark:text-amber-400";
    case "Error":
      return "border-red-500 bg-red-500/15 text-red-600 dark:text-red-400";
    case "Critical":
      return "border-purple-500 bg-purple-500/15 text-purple-600 dark:text-purple-400";
  }
};

const levelTextClass = (level: NotificationLevel) => {
  switch (level) {
    case "Info":
      return "text-sky-500";
    case "Warning":
      return "text-amber-500";
    case "Error":
      return "text-red-500";
    case "Critical":
      return "text-purple-500";
  }
};

// 最新在前
const all = computed(() => [...notificationStore.notifications].reverse());

const filtered = computed(() => {
  let list = all.value;
  if (levelFilter.value.size > 0) {
    list = list.filter((n) => levelFilter.value.has(n.level));
  }
  const q = search.value.trim().toLowerCase();
  if (q) {
    list = list.filter(
      (n) =>
        n.title.toLowerCase().includes(q) ||
        n.message.toLowerCase().includes(q) ||
        n.code.toLowerCase().includes(q),
    );
  }
  return list;
});

function clearAll() {
  notificationStore.clearAll();
}
</script>

<template>
    <SectionCard
        :title="t('debug.nav.notifications')"
        :subtitle="t('debug.notifications.subtitle')"
        :collapsible="false"
    >
        <div class="mb-3 flex flex-wrap items-center gap-2">
            <div class="flex flex-wrap items-center gap-1.5">
                <button
                    v-for="lvl in LEVELS"
                    :key="lvl"
                    type="button"
                    class="rounded-full border px-2.5 py-0.5 text-xs font-medium transition-colors"
                    :class="levelChipClass(lvl, levelFilter.has(lvl))"
                    @click="toggleLevel(lvl)"
                >
                    {{ lvl }}
                </button>
            </div>
            <Input
                v-model="search"
                :placeholder="t('debug.notifications.searchPlaceholder')"
                class="h-8 w-48"
            />
            <Button variant="outline" size="xs" @click="clearAll">
                <Eraser class="size-3" />{{ t("debug.notifications.clearAll") }}
            </Button>
            <span class="ml-auto text-xs text-muted-foreground">
                {{ t("debug.notifications.count", { shown: filtered.length, total: all.length }) }}
            </span>
        </div>

        <div
            v-if="filtered.length === 0"
            class="rounded-md border border-dashed p-8 text-center text-sm text-muted-foreground"
        >
            {{ t("debug.notifications.empty") }}
        </div>

        <div class="space-y-2">
            <div
                v-for="n in filtered"
                :key="n.id"
                class="rounded-md border bg-muted/20 px-3 py-2"
            >
                <div class="flex items-center gap-2">
                    <span class="text-xs font-semibold uppercase" :class="levelTextClass(n.level)">
                        {{ n.level }}
                    </span>
                    <span class="truncate text-sm font-medium">{{ n.title }}</span>
                    <span class="ml-auto shrink-0 font-mono text-xs text-muted-foreground">
                        {{ n.code }}
                    </span>
                </div>
                <p v-if="n.message" class="mt-0.5 text-xs text-muted-foreground">{{ n.message }}</p>
                <p class="mt-1 text-xs text-muted-foreground/70">
                    {{ n.source }} · {{ formatTime(n.timestamp) }}
                </p>
            </div>
        </div>
    </SectionCard>
</template>
