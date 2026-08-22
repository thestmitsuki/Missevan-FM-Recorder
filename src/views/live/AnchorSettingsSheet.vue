<script setup lang="ts">
/**
 * 主播设置弹窗（Sheet side="none" 居中模态，规格「主播设置」）
 *
 * 动效：默认 fade-in + zoom-in-95（打开 500ms / 关闭 300ms，tw-animate-css），
 * 不使用 side="right" 的右侧滑入动画（side="none" 不产生 slide 类）。
 * 比例：横向 4:3（max-w-2xl 672px × aspect-[4/3] 高 504px，小屏 max-h-[90vh] 兜底），
 * 表单字段两列、操作按钮竖排（刷新信息在删除主播上方）；内容区独立滚动，header/footer 固定。
 *
 * - 状态区：录制中显示录制时长（隐藏直播时长）；否则直播中显示直播时长；
 *   时长取 store 中的状态起始时间戳（后端不提供起始时间，以首次获知状态为起点），
 *   组件内 setInterval 每秒递增，卸载/关闭时清理。
 * - 简介：经 get_anchor_profile 从后端获取（info.creator.introduction），
 *   获取失败或无简介显示占位文案。
 * - 房间号只读；URL 可改（保存后后端 update_anchor 强制重取信息）；
 *   检测开关 / 别名 / Cookie / 代理可编辑；「刷新信息」立即重取名称头像；
 *   「停止录制」仅录制中显示。保存调 update_anchor，成功后关闭。
 * - 打开时通过 open-auto-focus preventDefault 不聚焦任何字段（默认会聚焦
 *   第一个可聚焦元素——只读房间号输入框）。
 * 成功/失败通知由后端 dispatcher 推送（app:notification），本组件不重复造通知。
 */
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { Mic, RefreshCw, Square, Trash2 } from "@lucide/vue";

import type { AnchorConfig, AnchorProfile } from "@/types";
import { useAnchorStore } from "@/stores/anchorStore";
import { ANCHOR_TAGS, ANCHOR_TAG_VALUES, isPresetTag } from "@/lib/anchorTags";
import { api } from "@/services/api";
import { extractRoomId } from "@/services/liveUrl";
import StatusBadge from "@/components/common/StatusBadge.vue";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
    Sheet,
    SheetContent,
    SheetFooter,
    SheetHeader,
    SheetTitle,
} from "@/components/ui/sheet";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";

const open = defineModel<boolean>("open", { default: false });

const props = defineProps<{
    anchor: AnchorConfig;
}>();

const emit = defineEmits<{
    /** 请求删除该主播（父级弹确认对话框，复用删除流程） */
    remove: [];
}>();

const anchorStore = useAnchorStore();
const { t } = useI18n();

// ── 状态与时长 ──
const now = ref(Date.now());
let tickTimer: number | undefined;

const status = computed(() => anchorStore.statusMap[props.anchor.id]);
const isLive = computed(() => status.value?.is_live ?? false);
const isRecording = computed(() => status.value?.is_recording ?? false);

const liveElapsed = computed(() => {
    const since = anchorStore.liveSinceOf(props.anchor.id);
    return since === undefined ? null : formatDuration(now.value - since);
});

const recordingElapsed = computed(() => {
    const since = anchorStore.recordingSinceOf(props.anchor.id);
    return since === undefined ? null : formatDuration(now.value - since);
});

/** HH:MM:SS（不足 1 小时显示 MM:SS） */
function formatDuration(ms: number): string {
    const totalSec = Math.max(0, Math.floor(ms / 1000));
    const h = Math.floor(totalSec / 3600);
    const m = Math.floor((totalSec % 3600) / 60);
    const s = totalSec % 60;
    const pad = (n: number) => String(n).padStart(2, "0");
    return h > 0 ? `${pad(h)}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
}

// ── 主播简介（后端获取）──
const profile = ref<AnchorProfile | null>(null);
const profileLoading = ref(false);

async function loadProfile(roomId: string) {
    profileLoading.value = true;
    try {
        profile.value = await api.getAnchorProfile(roomId);
    } catch (e) {
        console.error("Failed to fetch anchor profile:", e);
        profile.value = null;
    } finally {
        profileLoading.value = false;
    }
}

// ── 表单字段 ──
const editName = ref("");
const editUrl = ref("");
const editEnableCheck = ref(true);
const editCookie = ref("");
const editProxy = ref("");
/** 已勾选的固定标签（规范值；预选 anchor.tags 中属于固定 5 标签的部分） */
const selectedTags = ref<string[]>([]);
const urlError = ref("");
const saveError = ref("");
const saving = ref(false);
const refreshing = ref(false);
const stopping = ref(false);

function resetForm(anchor: AnchorConfig) {
    editName.value = anchor.name;
    editUrl.value = anchor.url;
    editEnableCheck.value = anchor.enable_check;
    editCookie.value = anchor.cookie ?? "";
    editProxy.value = anchor.proxy ?? "";
    selectedTags.value = (anchor.tags ?? []).filter(isPresetTag);
    urlError.value = "";
    saveError.value = "";
}

/** 展示用名称：优先取 store 最新数据（刷新/保存后可能变化） */
const displayName = computed(
    () =>
        anchorStore.anchors.find((a) => a.id === props.anchor.id)?.name ??
        props.anchor.name,
);

/** 展示用头像：优先取 store 最新数据（「刷新信息」后 avatar_url 更新，无需重开面板） */
const avatarUrlDisplay = computed(
    () =>
        anchorStore.anchors.find((a) => a.id === props.anchor.id)?.avatar_url ??
        props.anchor.avatar_url,
);

const roomIdDisplay = computed(
    () => props.anchor.room_id || extractRoomId(editUrl.value) || "",
);

// 打开或切换主播：重置表单 + 加载简介 + 启动计时
watch(
    [open, () => props.anchor],
    ([isOpen]) => {
        if (isOpen) {
            resetForm(props.anchor);
            now.value = Date.now();
            if (tickTimer === undefined) {
                tickTimer = window.setInterval(() => {
                    now.value = Date.now();
                }, 1000);
            }
            void loadProfile(props.anchor.room_id || roomIdDisplay.value);
        } else {
            if (tickTimer !== undefined) {
                window.clearInterval(tickTimer);
                tickTimer = undefined;
            }
        }
    },
    { immediate: true },
);

onBeforeUnmount(() => {
    if (tickTimer !== undefined) {
        window.clearInterval(tickTimer);
        tickTimer = undefined;
    }
});

function validateUrl(): boolean {
    const value = editUrl.value.trim();
    if (!value) {
        urlError.value = t("live.urlRequired");
        return false;
    }
    if (!extractRoomId(value)) {
        urlError.value = t("live.invalidUrl");
        return false;
    }
    urlError.value = "";
    return true;
}

function handleOpenChange(isOpen: boolean) {
    open.value = isOpen;
}

/**
 * Sheet 打开时 reka-ui FocusScope 默认自动聚焦内容区第一个可聚焦元素
 * （只读「房间号」输入框——只读字段不应被聚焦）。preventDefault 阻止自动聚焦，
 * 打开时不聚焦任何字段，焦点保留在触发元素（卡片/菜单）上。
 */
function onOpenAutoFocus(event: Event) {
    event.preventDefault();
}

async function handleSave() {
    if (!validateUrl()) return;
    saveError.value = "";
    saving.value = true;
    try {
        const urlChanged = editUrl.value.trim() !== props.anchor.url;
        await anchorStore.updateAnchor(props.anchor.id, {
            ...props.anchor,
            name: editName.value.trim(),
            url: editUrl.value.trim(),
            room_id: extractRoomId(editUrl.value) ?? props.anchor.room_id,
            enable_check: editEnableCheck.value,
            cookie: editCookie.value.trim() || null,
            proxy: editProxy.value.trim() || null,
            tags: [...selectedTags.value],
        });
        // URL 变更 → 后端已强制重取信息（update_anchor 内部调 API），同步简介
        if (urlChanged) {
            const rid = extractRoomId(editUrl.value);
            if (rid) void loadProfile(rid);
        }
        open.value = false;
    } catch (e) {
        console.error("Failed to save anchor settings:", e);
        saveError.value = t("live.updateFailed");
    } finally {
        saving.value = false;
    }
}

/** 刷新信息：立即从 API 拉取最新名称/头像（后端 refresh_anchor），并同步简介 */
async function handleRefresh() {
    refreshing.value = true;
    try {
        await anchorStore.refreshAnchor(props.anchor.id);
        const fresh = anchorStore.anchors.find((a) => a.id === props.anchor.id);
        if (fresh) editName.value = fresh.name;
        const rid = extractRoomId(editUrl.value) ?? props.anchor.room_id;
        if (rid) await loadProfile(rid);
    } catch (e) {
        console.error("Failed to refresh anchor info:", e);
    } finally {
        refreshing.value = false;
    }
}

async function handleStopRecording() {
    stopping.value = true;
    try {
        await anchorStore.stopRecording(props.anchor.id);
    } catch (e) {
        console.error("Failed to stop recording:", e);
    } finally {
        stopping.value = false;
    }
}

/** Checkbox 勾选切换：从固定 5 标签中多选（规范值） */
function toggleTag(value: string, checked: boolean) {
    selectedTags.value = checked
        ? [...selectedTags.value, value]
        : selectedTags.value.filter((v) => v !== value);
}
</script>

<template>
    <Sheet :open="open" @update:open="handleOpenChange">
        <SheetContent
            side="none"
            class="fixed left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 w-full max-w-2xl aspect-[4/3] max-h-[90vh] overflow-hidden sm:rounded-xl gap-0 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95"
            @open-auto-focus="onOpenAutoFocus"
        >
            <SheetHeader class="border-b border-border px-5 py-4">
                <SheetTitle>{{ t("live.anchorSettings") }}</SheetTitle>
            </SheetHeader>

            <div class="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-5 py-4">
                <!-- 头像 + 名称 + 状态徽标（头像取 store 最新值，刷新后即时更新） -->
                <div class="flex items-center gap-3">
                    <img
                        v-if="avatarUrlDisplay"
                        :src="avatarUrlDisplay"
                        alt=""
                        class="size-14 shrink-0 rounded-full border border-border/60 object-cover"
                    />
                    <div
                        v-else
                        class="flex size-14 shrink-0 items-center justify-center rounded-full bg-muted"
                    >
                        <Mic
                            class="size-6 text-muted-foreground"
                            aria-hidden="true"
                        />
                    </div>
                    <div class="min-w-0 flex-1">
                        <p class="truncate text-base font-semibold">
                            {{ displayName }}
                        </p>
                        <StatusBadge
                            class="mt-1"
                            :live="isLive"
                            :recording="isRecording"
                        />
                    </div>
                </div>

                <!-- 状态区：录制中显示录制时长并隐藏直播时长；否则直播中显示直播时长 -->
                <div class="rounded-lg border border-border/60 px-3 py-2.5">
                    <template v-if="isRecording">
                        <div class="flex items-center justify-between text-sm">
                            <span class="text-muted-foreground">
                                {{ t("live.recordingDuration") }}
                            </span>
                            <span
                                class="font-mono text-base font-semibold text-primary"
                            >
                                {{ recordingElapsed ?? "--:--" }}
                            </span>
                        </div>
                    </template>
                    <template v-else-if="isLive">
                        <div class="flex items-center justify-between text-sm">
                            <span class="text-muted-foreground">
                                {{ t("live.liveDuration") }}
                            </span>
                            <span
                                class="font-mono text-base font-semibold text-destructive"
                            >
                                {{ liveElapsed ?? "--:--" }}
                            </span>
                        </div>
                    </template>
                    <p v-else class="text-sm text-muted-foreground">
                        {{ t("live.statusNotLive") }}
                    </p>
                </div>

                <!-- 主播简介（后端获取，失败/缺失显示占位） -->
                <div class="flex flex-col gap-1.5">
                    <Label class="text-sm font-medium">
                        {{ t("live.introduction") }}
                    </Label>
                    <p
                        class="rounded-lg border border-border/60 bg-background/60 px-3 py-2 text-sm leading-relaxed"
                    >
                        <span
                            v-if="profileLoading"
                            class="inline-flex items-center gap-2 text-muted-foreground"
                        >
                            <Spinner class="size-3.5" />
                            {{ t("common.loading") }}
                        </span>
                        <span v-else-if="profile?.introduction">
                            {{ profile.introduction }}
                        </span>
                        <span v-else class="italic text-muted-foreground">
                            {{ t("live.noIntroduction") }}
                        </span>
                    </p>
                </div>

                <div class="grid grid-cols-2 gap-x-4 gap-y-4">
                <!-- 房间号（只读） -->
                <div class="flex flex-col gap-1.5">
                    <Label class="text-sm font-medium" for="set-room">
                        {{ t("live.roomId") }}
                    </Label>
                    <Input
                        id="set-room"
                        :model-value="roomIdDisplay"
                        readonly
                    />
                </div>

                <!-- 主播别名（留空使用官方名称） -->
                <div class="flex flex-col gap-1.5">
                    <Label class="text-sm font-medium" for="set-name">
                        {{ t("live.alias") }}
                    </Label>
                    <Input
                        id="set-name"
                        v-model="editName"
                        :placeholder="t('live.aliasPlaceholder')"
                    />
                </div>

                <!-- 直播间 URL（可修改，保存后强制重取信息） -->
                <div class="col-span-2 flex flex-col gap-1.5">
                    <Label class="text-sm font-medium" for="set-url">
                        {{ t("live.homepageUrl") }}
                    </Label>
                    <Input
                        id="set-url"
                        v-model="editUrl"
                        type="url"
                        :placeholder="t('live.homepageUrlPlaceholder')"
                        :aria-invalid="!!urlError"
                        @blur="validateUrl"
                    />
                    <p
                        v-if="urlError"
                        class="text-xs text-destructive"
                        role="alert"
                    >
                        {{ urlError }}
                    </p>
                </div>

                <!-- Cookie（可选） -->
                <div class="flex flex-col gap-1.5">
                    <Label class="text-sm font-medium" for="set-cookie">
                        {{ t("live.cookie") }}
                    </Label>
                    <Input
                        id="set-cookie"
                        v-model="editCookie"
                        :placeholder="t('live.cookiePlaceholder')"
                        maxlength="4096"
                    />
                </div>

                <!-- 代理（可选） -->
                <div class="flex flex-col gap-1.5">
                    <Label class="text-sm font-medium" for="set-proxy">
                        {{ t("live.proxy") }}
                    </Label>
                    <Input
                        id="set-proxy"
                        v-model="editProxy"
                        :placeholder="t('live.proxyPlaceholder')"
                    />
                </div>
                </div>

                <!-- 标签（固定 5 个多选，禁止自由输入；后端持久化） -->
                <div class="flex flex-col gap-1.5">
                    <Label class="text-sm font-medium">
                        {{ t("live.tags") }}
                    </Label>
                    <div class="flex flex-wrap gap-x-4 gap-y-1.5">
                        <label
                            v-for="(key, i) in ANCHOR_TAGS"
                            :key="key"
                            class="flex cursor-pointer items-center gap-2 text-sm"
                        >
                            <Checkbox
                                :checked="
                                    selectedTags.includes(
                                        ANCHOR_TAG_VALUES[i],
                                    )
                                "
                                @update:checked="(v) =>
                                    toggleTag(ANCHOR_TAG_VALUES[i], v === true)"
                            />
                            <span>{{ t(key) }}</span>
                        </label>
                    </div>
                    <p class="text-xs text-muted-foreground">
                        {{ t("live.tagSelectHint") }}
                    </p>
                </div>

                <!-- 检测开关 -->
                <div
                    class="flex items-center justify-between rounded-lg border border-border/60 px-3 py-2.5"
                >
                    <Label class="text-sm font-medium" for="set-check">
                        {{ t("live.enableLiveCheck") }}
                    </Label>
                    <Switch id="set-check" v-model:checked="editEnableCheck" />
                </div>

                <p
                    v-if="saveError"
                    class="text-xs text-destructive"
                    role="alert"
                >
                    {{ saveError }}
                </p>

                <!-- 操作按钮：刷新信息 / 停止录制（仅录制中）/ 删除主播（竖排，刷新在删除上方） -->
                <div class="flex flex-col gap-2">
                    <Button
                        variant="outline"
                        :disabled="refreshing"
                        @click="handleRefresh"
                    >
                        <RefreshCw
                            v-if="!refreshing"
                            class="size-4"
                            aria-hidden="true"
                        />
                        <Spinner v-else class="size-4" />
                        {{ t("live.refreshInfo") }}
                    </Button>
                    <Button
                        v-if="isRecording"
                        variant="destructive"
                        :disabled="stopping"
                        @click="handleStopRecording"
                    >
                        <Square
                            v-if="!stopping"
                            class="size-4"
                            aria-hidden="true"
                        />
                        <Spinner v-else class="size-4" />
                        {{ t("live.stopRecording") }}
                    </Button>
                    <Button
                        variant="ghost"
                        class="w-full justify-center border border-destructive/40 text-destructive hover:bg-destructive/10 hover:text-destructive"
                        :disabled="saving"
                        @click="emit('remove')"
                    >
                        <Trash2 class="size-4" aria-hidden="true" />
                        {{ t("live.deleteAnchor") }}
                    </Button>
                </div>
            </div>

            <SheetFooter class="border-t border-border/60 px-5 py-4">
                <div class="flex-1" />
                <Button
                    variant="ghost"
                    :disabled="saving"
                    @click="handleOpenChange(false)"
                >
                    {{ t("live.cancel") }}
                </Button>
                <Button :disabled="saving" @click="handleSave">
                    {{ saving ? t("live.saving") : t("live.save") }}
                </Button>
            </SheetFooter>
        </SheetContent>
    </Sheet>
</template>
