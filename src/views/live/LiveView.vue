<script setup lang="ts">
/**
 * 直播页（规格「直播页面功能规格」）
 *
 * 布局：左 NavRail（56px 竖排操作栏）+ 可选筛选面板 + 右内容区。
 * 卡片视图：grid 自适应列（最小列宽 300px）；列表视图：行式列表。
 * 四态：加载（Skeleton 网格）/ 空 / 错误（重试）/ 无结果。
 * 状态更新：首次全量 fetch，之后只依赖 recording_status_changed 事件更新单条
 * （统一走 events.ts -> anchorStore.updateStatusFromEvent，页面内不直接 listen）。
 * 删除：菜单 -> ConfirmDialog -> remove_anchor。
 * 键盘导航：卡片/列表项 Tab 聚焦，Enter 打开设置，Delete 触发删除确认。
 * 回顶按钮显示由本页内容区滚动监听接管（替代旧 anchorsCount 近似判断）。
 */
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { Filter, LayoutGrid, List, Plus } from "@lucide/vue";
import type { AnchorConfig } from "@/types";

import { useAnchorStore } from "@/stores/anchorStore";
import { ANCHOR_TAGS, ANCHOR_TAG_VALUES } from "@/lib/anchorTags";
import NavRail, { type NavRailItem } from "@/components/common/NavRail.vue";
import EmptyState from "@/components/common/EmptyState.vue";
import ConfirmDialog from "@/components/common/ConfirmDialog.vue";
import AnchorCard from "./AnchorCard.vue";
import AddAnchorDialog from "./AddAnchorDialog.vue";
import AnchorSettingsSheet from "./AnchorSettingsSheet.vue";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Skeleton } from "@/components/ui/skeleton";

const anchorStore = useAnchorStore();
const { t } = useI18n();

// ── 布局状态 ──
const filterOpen = ref(false);
const showScrollTop = ref(false);
const contentRef = ref<HTMLElement | null>(null);

// ── 删除确认 ──
const removeTarget = ref<AnchorConfig | null>(null);

// ── 添加主播对话框 / 主播设置 Sheet（Task 11 接线）──
const showAddDialog = ref(false);
const settingsAnchor = ref<AnchorConfig | null>(null);
const settingsOpen = ref(false);

// ── 筛选面板选项 ──
const recordOptions = [
    { value: "all", label: "live.all" },
    { value: "recording", label: "live.recording" },
    { value: "not-recording", label: "live.notRecording" },
] as const;

const liveOptions = [
    { value: "all", label: "live.all" },
    { value: "live", label: "live.liveNow" },
    { value: "not-live", label: "live.notLive" },
] as const;

// ── 状态查找（模板用）──
function statusOf(anchorId: string) {
    const s = anchorStore.statusMap[anchorId];
    return {
        isLive: s?.is_live ?? false,
        isRecording: s?.is_recording ?? false,
    };
}

// ── 数据加载（首次全量 + 重试）──
async function loadData() {
    try {
        await Promise.all([
            anchorStore.fetchAnchors(),
            anchorStore.fetchRecordingStatuses(),
        ]);
    } catch (e) {
        console.error("Failed to load anchors", e);
    }
}

// ── 回顶：内容区滚动监听 ──
const SCROLL_TOP_THRESHOLD = 400;

function onContentScroll() {
    showScrollTop.value =
        (contentRef.value?.scrollTop ?? 0) > SCROLL_TOP_THRESHOLD;
}

function scrollToTop() {
    contentRef.value?.scrollTo({ top: 0, behavior: "smooth" });
}

// ── 竖向操作栏（NavRail 配置：添加 / 视图切换 / 筛选 / 回顶）──
const railItems = computed<NavRailItem[]>(() => [
    {
        id: "add",
        icon: Plus,
        label: t("live.addAnchor"),
        primary: true,
        onClick: handleAddAnchor,
    },
    {
        id: "view",
        icon: anchorStore.viewMode === "card" ? LayoutGrid : List,
        label:
            anchorStore.viewMode === "card"
                ? t("live.switchToList")
                : t("live.switchToCard"),
        active: true, // 视图切换按钮常亮高亮
        onClick: () => {
            anchorStore.viewMode =
                anchorStore.viewMode === "card" ? "list" : "card";
        },
    },
    {
        id: "filter",
        icon: Filter,
        label: t("live.filterAnchors"),
        active: filterOpen.value,
        expanded: filterOpen.value,
        onClick: () => {
            filterOpen.value = !filterOpen.value;
        },
    },
]);

// ── 菜单/键盘动作 ──
function requestRemove(anchor: AnchorConfig) {
    removeTarget.value = anchor;
}

function confirmRemove() {
    const target = removeTarget.value;
    removeTarget.value = null;
    if (!target) return;
    // 成功/失败通知由后端 dispatcher 推送（app:notification），此处只需防未处理拒绝
    anchorStore.removeAnchor(target.id).catch((e) => {
        console.error("Failed to remove anchor", e);
    });
}

/** 主播设置 Sheet 内删除按钮：关闭 Sheet 并复用删除确认流程 */
function onSheetRemove() {
    settingsOpen.value = false;
    if (settingsAnchor.value) {
        requestRemove(settingsAnchor.value);
    }
}

function handleRefresh(anchor: AnchorConfig) {
    anchorStore.refreshAnchor(anchor.id).catch((e) => {
        console.error("Failed to refresh anchor info", e);
    });
}

/** 打开添加主播对话框（操作栏 + 按钮触发） */
function handleAddAnchor() {
    showAddDialog.value = true;
}

/** 打开主播设置侧栏（卡片/列表项菜单或 Enter） */
function openSettings(anchor: AnchorConfig) {
    settingsAnchor.value = anchor;
    settingsOpen.value = true;
}

// ── 错误文案 ──
const errorText = computed(() => {
    const e = anchorStore.error;
    if (e instanceof Error) return e.message;
    if (typeof e === "string") return e;
    return "";
});

onMounted(() => {
    contentRef.value?.addEventListener("scroll", onContentScroll, {
        passive: true,
    });
    void loadData();
});

onBeforeUnmount(() => {
    contentRef.value?.removeEventListener("scroll", onContentScroll);
});
</script>

<template>
    <div class="relative flex h-full min-w-0 flex-1">
        <!-- 左侧竖排操作栏（NavRail 通用组件，配置式） -->
        <NavRail
            :items="railItems"
            :aria-label="t('nav.liveMonitor')"
            :show-scroll-top="showScrollTop"
            :scroll-top-label="t('live.backToTop')"
            @scroll-top="scrollToTop"
        />

        <!-- 遮罩层：点击外部关闭面板（z-20 低于面板 z-30） -->
        <div
            v-if="filterOpen"
            class="fixed inset-0 z-20 bg-transparent"
            @click="filterOpen = false"
        />
        <!-- 筛选面板（悬浮覆盖在内容区之上，不挤压内容布局；实时生效，条件持久化 localStorage） -->
        <aside
            v-if="filterOpen"
            class="absolute h-[55vh] max-h-[90vh] left-14 top-0 z-30 flex w-64 flex-col gap-5 overflow-y-auto bg-background/95 p-4 backdrop-blur rounded-lg max-[720px]:left-12"
            :aria-label="t('live.filterTitle')"
        >
            <div class="flex items-center justify-between">
                <h2 class="text-sm font-semibold">
                    {{ t("live.filterTitle") }}
                </h2>
                <Button
                    variant="ghost"
                    size="sm"
                    class="h-7 px-2 text-xs"
                    @click="anchorStore.clearFilters()"
                >
                    {{ t("live.clearFilters") }}
                </Button>
            </div>

            <!-- 按主播名模糊搜索 -->
            <div class="flex flex-col gap-2">
                <Label class="text-xs text-muted-foreground">
                    {{ t("live.searchByName") }}
                </Label>
                <Input
                    v-model="anchorStore.searchQuery"
                    :placeholder="t('live.searchPlaceholder')"
                    class="h-8"
                />
            </div>

            <!-- 按标签单选（「全部」= 不过滤；固定 5 标签，tagFilter 为 null 时「全部」选中） -->
            <div class="flex flex-col gap-2">
                <Label class="text-xs text-muted-foreground">
                    {{ t("live.filterByTag") }}
                </Label>
                <RadioGroup
                    v-model="anchorStore.tagFilter"
                    class="flex flex-col gap-1.5"
                >
                    <label
                        class="flex cursor-pointer items-center gap-2 text-sm"
                    >
                        <RadioGroupItem :value="null" class="size-4" />
                        <span>{{ t("live.all") }}</span>
                    </label>
                    <label
                        v-for="(key, i) in ANCHOR_TAGS"
                        :key="key"
                        class="flex cursor-pointer items-center gap-2 text-sm"
                    >
                        <RadioGroupItem
                            :value="ANCHOR_TAG_VALUES[i]"
                            class="size-4"
                        />
                        <span>{{ t(key) }}</span>
                    </label>
                </RadioGroup>
            </div>

            <!-- 按录制状态单选 -->
            <div class="flex flex-col gap-2">
                <Label class="text-xs text-muted-foreground">
                    {{ t("live.filterByRecording") }}
                </Label>
                <RadioGroup
                    v-model="anchorStore.recordFilter"
                    class="flex flex-col gap-1.5"
                >
                    <label
                        v-for="opt in recordOptions"
                        :key="opt.value"
                        class="flex cursor-pointer items-center gap-2 text-sm"
                    >
                        <RadioGroupItem :value="opt.value" class="size-4" />
                        <span>{{ t(opt.label) }}</span>
                    </label>
                </RadioGroup>
            </div>

            <!-- 按直播状态单选 -->
            <div class="flex flex-col gap-2">
                <Label class="text-xs text-muted-foreground">
                    {{ t("live.filterByLive") }}
                </Label>
                <RadioGroup
                    v-model="anchorStore.liveFilter"
                    class="flex flex-col gap-1.5"
                >
                    <label
                        v-for="opt in liveOptions"
                        :key="opt.value"
                        class="flex cursor-pointer items-center gap-2 text-sm"
                    >
                        <RadioGroupItem :value="opt.value" class="size-4" />
                        <span>{{ t(opt.label) }}</span>
                    </label>
                </RadioGroup>
            </div>
        </aside>

        <!-- 内容区（本页滚动容器；居中限宽显示，不贴边） -->
        <section
            ref="contentRef"
            class="page-scroll mx-auto min-w-0 max-w-[1200px] flex-1 overflow-y-auto px-4 py-4"
            :aria-busy="anchorStore.loading"
        >
            <!-- 加载态：Skeleton 网格 -->
            <div
                v-if="anchorStore.loading"
                class="grid grid-cols-[repeat(auto-fill,minmax(300px,1fr))] gap-3"
            >
                <Skeleton
                    v-for="i in 6"
                    :key="i"
                    class="aspect-[4/3] rounded-xl"
                />
            </div>

            <!-- 错误态：提示 + 重试 -->
            <EmptyState
                v-else-if="anchorStore.error"
                icon="⚠️"
                :title="t('live.loadFailed')"
                :description="errorText"
                :action-label="t('common.retry')"
                @action="loadData"
            />

            <!-- 空态：尚未添加主播 -->
            <EmptyState
                v-else-if="anchorStore.anchors.length === 0"
                icon="🎙️"
                :title="t('live.noAnchorsYet')"
                :description="t('live.noAnchorsAddHint')"
            />

            <!-- 无结果态：筛选后无匹配 -->
            <EmptyState
                v-else-if="anchorStore.filteredAnchors.length === 0"
                icon="🔍"
                :title="t('live.noMatchingAnchors')"
            />

            <!-- 卡片视图：min 300px 自适应列 -->
            <div
                v-else-if="anchorStore.viewMode === 'card'"
                class="grid grid-cols-[repeat(auto-fill,minmax(300px,1fr))] gap-3"
            >
                <AnchorCard
                    v-for="anchor in anchorStore.filteredAnchors"
                    :key="anchor.id"
                    :anchor="anchor"
                    :is-live="statusOf(anchor.id).isLive"
                    :is-recording="statusOf(anchor.id).isRecording"
                    view="card"
                    @settings="openSettings"
                    @remove="requestRemove"
                    @refresh="handleRefresh"
                />
            </div>

            <!-- 列表视图：行式，细分割线 -->
            <ul
                v-else
                class="divide-y divide-border rounded-xl border border-border/60 bg-background/60"
            >
                <AnchorCard
                    v-for="anchor in anchorStore.filteredAnchors"
                    :key="anchor.id"
                    :anchor="anchor"
                    :is-live="statusOf(anchor.id).isLive"
                    :is-recording="statusOf(anchor.id).isRecording"
                    view="list"
                    @settings="openSettings"
                    @remove="requestRemove"
                    @refresh="handleRefresh"
                />
            </ul>
        </section>

        <!-- 删除确认 -->
        <ConfirmDialog
            :open="!!removeTarget"
            :title="t('live.deleteAnchor')"
            :message="t('live.deleteConfirmMessage')"
            destructive
            @confirm="confirmRemove"
            @cancel="removeTarget = null"
        />

        <!-- 添加主播对话框 -->
        <AddAnchorDialog v-model:open="showAddDialog" />

        <!-- 主播设置侧栏（保持挂载以保留关闭动画；open 由 settingsOpen 控制） -->
        <AnchorSettingsSheet
            v-if="settingsAnchor"
            v-model:open="settingsOpen"
            :anchor="settingsAnchor"
            @remove="onSheetRemove"
        />
    </div>
</template>
