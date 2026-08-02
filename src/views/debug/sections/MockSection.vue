<script setup lang="ts">
/**
 * Mock 控制面板（规格 Mock 章节）：
 * - 顶部开关（set_mock_mode）：开启弹确认「真实录制将停止」（规格安全与可见性）；
 * - 模拟主播表格：名称 / 房间号 / 直播状态(Switch) / 流地址 可编辑 + 保存 / 删除 / 新增行；
 * - 批量控制：全部开播 / 全部下播 / 重置（二次确认）；
 * - 状态指示：Mock 模式开关态 + 模拟主播数 + 直播中数；
 * - `mock:status_changed` 事件 → mockStore 自动刷新。
 *
 * 数据不持久化（内存存储，重启清空，规格「数据持久化」章节）。
 */
import { onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { Plus, RotateCcw, Save, Trash2 } from "@lucide/vue";
import { useMockStore } from "@/stores/mockStore";
import type { MockLiveData } from "@/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import ConfirmDialog from "@/components/common/ConfirmDialog.vue";
import SectionCard from "./SectionCard.vue";

const { t } = useI18n();
const mockStore = useMockStore();

/** 可编辑行（深拷贝自 store；保存时与后端对齐） */
const drafts = ref<MockLiveData[]>([]);
/** 行 → 原始 room_id（room_id 是后端键；改名 = 删除旧键 + 新增新键） */
const originalRoomIds = new Map<MockLiveData, string>();

const enableConfirmOpen = ref(false);
const resetConfirmOpen = ref(false);
const busyRoom = ref<string | null>(null);
const errorMsg = ref<string | null>(null);
const actionMsg = ref<string | null>(null);

function rebuildDrafts() {
  drafts.value = mockStore.anchors.map((a) => ({ ...a }));
  for (const d of drafts.value) originalRoomIds.set(d, d.room_id);
}

function newRow() {
  drafts.value.push({
    room_id: "",
    name: "",
    is_live: false,
    stream_url: "mock://stream/",
    local_file: null,
  });
  actionMsg.value = null;
}

// ── 模式开关（开启需确认：真实录制将停止）──
function onToggleMode(next: boolean) {
  if (next) {
    enableConfirmOpen.value = true;
  } else {
    void doSetMode(false);
  }
}

async function doSetMode(enable: boolean) {
  enableConfirmOpen.value = false;
  errorMsg.value = null;
  try {
    await mockStore.setMode(enable);
  } catch (e) {
    errorMsg.value = t("debug.common.operationFailed", { error: String(e) });
  }
}

// ── 行操作 ──
async function saveRow(draft: MockLiveData) {
  const room = draft.room_id.trim();
  if (!room) {
    errorMsg.value = t("debug.mock.roomIdRequired");
    return;
  }
  busyRoom.value = room;
  errorMsg.value = null;
  actionMsg.value = null;
  try {
    const oldRoom = originalRoomIds.get(draft);
    const payload: MockLiveData = {
      ...draft,
      room_id: room,
      stream_url: draft.stream_url.trim(),
      local_file: draft.local_file?.trim() ? draft.local_file.trim() : null,
    };
    if (oldRoom && oldRoom !== room) {
      // 房间号改名：旧键删除 + 新键新增，避免残留孤儿条目
      await mockStore.removeAnchor(oldRoom);
      await mockStore.addAnchor(payload);
    } else if (oldRoom) {
      await mockStore.updateAnchor(payload);
    } else {
      await mockStore.addAnchor(payload);
    }
    actionMsg.value = t("debug.mock.saved");
    await mockStore.refresh();
    rebuildDrafts();
  } catch (e) {
    errorMsg.value = t("debug.mock.operationFailed", { error: String(e) });
  } finally {
    busyRoom.value = null;
  }
}

async function deleteRow(draft: MockLiveData) {
  const room = originalRoomIds.get(draft) ?? draft.room_id;
  errorMsg.value = null;
  try {
    if (room) await mockStore.removeAnchor(room);
    await mockStore.refresh();
    rebuildDrafts();
  } catch (e) {
    errorMsg.value = t("debug.mock.operationFailed", { error: String(e) });
  }
}

// ── 批量控制 ──
async function setAllLive(live: boolean) {
  errorMsg.value = null;
  try {
    await mockStore.setAllLive(live);
    await mockStore.refresh();
    rebuildDrafts();
  } catch (e) {
    errorMsg.value = t("debug.mock.operationFailed", { error: String(e) });
  }
}

async function resetAll() {
  resetConfirmOpen.value = false;
  errorMsg.value = null;
  try {
    await mockStore.reset();
    await mockStore.refresh();
    rebuildDrafts();
  } catch (e) {
    errorMsg.value = t("debug.mock.operationFailed", { error: String(e) });
  }
}

// ── 初始化 ──
onMounted(async () => {
  mockStore.startListening();
  await mockStore.refresh();
  rebuildDrafts();
});

onBeforeUnmount(() => {
  mockStore.stopListening();
});
</script>

<template>
    <SectionCard
        :title="t('debug.nav.mock')"
        :subtitle="t('debug.mock.desc')"
        :collapsible="false"
    >
        <!-- 开关 + 状态指示 -->
        <div
            class="mb-4 flex flex-wrap items-center gap-3 rounded-md border px-4 py-3"
            :class="
                mockStore.enabled
                    ? 'border-amber-500/40 bg-amber-500/5'
                    : 'border-input bg-muted/20'
            "
        >
            <!-- Task 20 a11y：Mock 模式开关态变化经 aria-live 播报（规格 §7.1） -->
            <div class="min-w-0 flex-1" role="status" aria-live="polite">
                <p class="text-sm font-medium">
                    {{
                        mockStore.enabled
                            ? t("debug.mock.statusOn")
                            : t("debug.mock.statusOff")
                    }}
                </p>
                <p class="text-xs text-muted-foreground">
                    {{
                        t("debug.mock.statusDetail", {
                            count: mockStore.count,
                            live: mockStore.liveCount,
                        })
                    }}
                </p>
            </div>
            <Switch
                :checked="mockStore.enabled"
                @update:checked="onToggleMode"
            />
        </div>

        <div v-if="errorMsg" class="mb-3 rounded-md border border-red-500/40 bg-red-500/10 px-3 py-2 text-xs text-red-600 dark:text-red-400" role="alert">
            {{ errorMsg }}
        </div>
        <p v-if="actionMsg" class="mb-3 text-xs text-muted-foreground" role="status">
            {{ actionMsg }}
        </p>

        <!-- 批量控制 -->
        <div class="mb-3 flex flex-wrap gap-2">
            <Button size="sm" @click="setAllLive(true)">
                {{ t("debug.mock.allLive") }}
            </Button>
            <Button variant="outline" size="sm" @click="setAllLive(false)">
                {{ t("debug.mock.allOffline") }}
            </Button>
            <Button variant="outline" size="sm" class="text-destructive hover:text-destructive" @click="resetConfirmOpen = true">
                <RotateCcw class="size-3.5" />{{ t("debug.mock.reset") }}
            </Button>
        </div>

        <!-- 模拟主播表格 -->
        <div class="overflow-x-auto rounded-md border">
            <Table>
                <TableHeader>
                    <TableRow class="hover:bg-transparent">
                        <TableHead class="min-w-28">{{ t("debug.mock.name") }}</TableHead>
                        <TableHead class="min-w-24">{{ t("debug.mock.roomId") }}</TableHead>
                        <TableHead class="w-20">{{ t("debug.mock.liveState") }}</TableHead>
                        <TableHead class="min-w-40">{{ t("debug.mock.streamUrl") }}</TableHead>
                        <TableHead class="min-w-40">{{ t("debug.mock.localFile") }}</TableHead>
                        <TableHead class="w-28 text-right">{{ t("debug.mock.actions") }}</TableHead>
                    </TableRow>
                </TableHeader>
                <TableBody>
                    <TableRow v-for="(draft, idx) in drafts" :key="draft.room_id + '-' + idx">
                        <TableCell>
                            <Input v-model="draft.name" :placeholder="t('debug.mock.namePlaceholder')" class="h-8" />
                        </TableCell>
                        <TableCell>
                            <Input v-model="draft.room_id" :placeholder="t('debug.mock.roomIdPlaceholder')" class="h-8 font-mono" />
                        </TableCell>
                        <TableCell>
                            <Switch
                                :checked="draft.is_live"
                                @update:checked="(v: boolean) => (draft.is_live = v)"
                            />
                        </TableCell>
                        <TableCell>
                            <Input v-model="draft.stream_url" placeholder="mock://stream/" class="h-8 font-mono" />
                        </TableCell>
                        <TableCell>
                            <Input
                                :model-value="draft.local_file ?? ''"
                                :placeholder="t('debug.mock.localFilePlaceholder')"
                                class="h-8 font-mono"
                                @update:model-value="(v) => (draft.local_file = String(v) || null)"
                            />
                        </TableCell>
                        <TableCell class="text-right">
                            <div class="flex justify-end gap-1">
                                <Button
                                    size="xs"
                                    :disabled="busyRoom !== null"
                                    @click="saveRow(draft)"
                                >
                                    <Save class="size-3" />{{ t("debug.mock.save") }}
                                </Button>
                                <Button
                                    variant="ghost"
                                    size="xs"
                                    class="text-destructive hover:text-destructive"
                                    :disabled="busyRoom !== null"
                                    @click="deleteRow(draft)"
                                >
                                    <Trash2 class="size-3" />
                                </Button>
                            </div>
                        </TableCell>
                    </TableRow>
                </TableBody>
            </Table>
        </div>

        <div class="mt-3">
            <Button variant="outline" size="sm" @click="newRow">
                <Plus class="size-3.5" />{{ t("debug.mock.addAnchor") }}
            </Button>
        </div>

        <!-- 开启 Mock 确认：真实录制将停止（规格安全与可见性） -->
        <ConfirmDialog
            :open="enableConfirmOpen"
            :title="t('debug.mock.enableConfirmTitle')"
            :message="t('debug.mock.enableConfirmMsg')"
            :confirm-text="t('debug.mock.enable')"
            destructive
            @confirm="doSetMode(true)"
            @cancel="enableConfirmOpen = false"
        />

        <!-- 重置确认 -->
        <ConfirmDialog
            :open="resetConfirmOpen"
            :title="t('debug.mock.resetConfirmTitle')"
            :message="t('debug.mock.resetConfirmMsg')"
            :confirm-text="t('debug.mock.reset')"
            destructive
            @confirm="resetAll"
            @cancel="resetConfirmOpen = false"
        />
    </SectionCard>
</template>
