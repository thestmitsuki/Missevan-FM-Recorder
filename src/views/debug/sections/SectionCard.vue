<script setup lang="ts">
/**
 * 调试模块卡片：标题栏（标题 + 副标题 + 右侧操作槽 + 刷新 + 折叠）+ 可折叠内容区。
 * 规格 §8 交互细节：「每个面板右上角有折叠/展开按钮、刷新按钮」。
 */
import { ref } from "vue";
import { ChevronDown, ChevronUp, RefreshCw } from "@lucide/vue";
import { Button } from "@/components/ui/button";

const props = withDefaults(
  defineProps<{
    title: string;
    subtitle?: string;
    /** 是否显示折叠按钮（默认显示） */
    collapsible?: boolean;
    /** 是否显示刷新按钮并上抛 refresh 事件（默认不显示） */
    refreshable?: boolean;
  }>(),
  {
    subtitle: "",
    collapsible: true,
    refreshable: false,
  },
);

const emit = defineEmits<{ refresh: [] }>();

const collapsed = ref(false);
const busy = ref(false);

async function doRefresh() {
  if (busy.value) return;
  busy.value = true;
  try {
    emit("refresh");
  } finally {
    // 短暂展示旋转动画，避免高频点击
    setTimeout(() => (busy.value = false), 400);
  }
}
</script>

<template>
    <section class="rounded-xl border bg-card">
        <header class="flex items-center gap-2 border-b px-4 py-3">
            <div class="min-w-0 flex-1">
                <h3 class="truncate text-sm font-semibold">{{ title }}</h3>
                <p v-if="subtitle" class="truncate text-xs text-muted-foreground">
                    {{ subtitle }}
                </p>
            </div>
            <slot name="header-right" />
            <Button
                v-if="props.refreshable"
                variant="ghost"
                size="xs"
                aria-label="refresh"
                @click="doRefresh"
            >
                <RefreshCw :class="busy ? 'animate-spin' : ''" class="size-3.5" />
            </Button>
            <Button
                v-if="props.collapsible"
                variant="ghost"
                size="xs"
                :aria-label="collapsed ? 'expand' : 'collapse'"
                @click="collapsed = !collapsed"
            >
                <ChevronUp v-if="!collapsed" class="size-3.5" />
                <ChevronDown v-else class="size-3.5" />
            </Button>
        </header>
        <div v-show="!collapsed" class="p-4">
            <slot />
        </div>
    </section>
</template>
