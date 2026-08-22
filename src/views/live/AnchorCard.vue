<script setup lang="ts">
/**
 * 主播卡片/列表项（规格「主播卡片/列表项设计」）
 *
 * 卡片视图：4:3 比例，上方 3/4 头像（object-cover + 渐变遮罩保证徽标可读），
 * 下方 1/4 信息区（名称/标签 + 菜单）；右上角状态徽标
 * （直播红呼吸 / 录制蓝叠加在下方）；右下角三点菜单（设置/删除/刷新），
 * 删除复用 ConfirmDialog 流程（主播设置 Sheet 内亦有删除按钮）。
 *
 * 列表视图：40px 圆头像 + 名称（加粗）+ 检测图标 + 标签行 + 右侧小徽标 + 菜单
 * （含删除，与卡片视图区分：列表保持菜单操作）。
 *
 * 标签展示（前端优化任务）：
 * - 卡片视图：信息区两行——行1 名称（truncate）+ 菜单，行2 房间号 + 检测图标 +
 *   标签药丸；标签 ≤4 全显示，>4 时显示前 3 个 + 「+N」溢出计数药丸（第 4 格），
 *   单标签过长 truncate 省略——信息区 25% 高度内两行可完整容纳（药丸 py-0.5/leading-tight）；
 * - 列表视图：保持单行横向排布（flex-wrap，不限制数量）。
 *
 * 键盘导航：卡片/列表项可聚焦，Enter 打开设置，Delete 触发删除确认。
 * 头像加载失败时回退默认主播图标；刷新（anchor 对象被替换）或 avatar_url
 * 变化后重置失败状态——失败回退不永久生效（v-if 重建 AvatarImage 后重新请求）。
 */
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import {
    Mic,
    MoreVertical,
    Radar,
    RefreshCw,
    Settings,
    Trash2,
} from "@lucide/vue";
import type { AnchorConfig } from "@/types";

import { useAppearanceStore } from "@/stores/appearanceStore";
import StatusBadge from "@/components/common/StatusBadge.vue";
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
    DropdownMenu,
    DropdownMenuContent,
    DropdownMenuItem,
    DropdownMenuSeparator,
    DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

const props = withDefaults(
    defineProps<{
        anchor: AnchorConfig;
        /** 直播中 */
        isLive?: boolean;
        /** 录制中 */
        isRecording?: boolean;
        /** 展示模式：卡片 / 列表 */
        view: "card" | "list";
    }>(),
    {
        isLive: false,
        isRecording: false,
    },
);

const emit = defineEmits<{
    settings: [anchor: AnchorConfig];
    remove: [anchor: AnchorConfig];
    refresh: [anchor: AnchorConfig];
}>();

const { t } = useI18n();

// 外观偏好：卡片显示项（设置页「外观」分类，localStorage 即时生效）
const appearance = useAppearanceStore();
const showAvatar = () => appearance.prefs.cardShowAvatar;
const showTags = () => appearance.prefs.cardShowTags;
const showRoomId = () => appearance.prefs.cardShowRoomId;
const showStatusIcon = () => appearance.prefs.cardShowStatusIcon;

// 头像加载失败回退默认图标（reka AvatarImage 内部预加载，失败时 AvatarFallback 显示 Mic）。
// 失败回退不永久生效：
// - anchor 对象被替换（refreshAnchor 回拉后整体替换）→ 重置，v-if 重建 AvatarImage 重新请求
// - avatar_url 值变化 → 重置
const imgFailed = ref(false);
watch(
    () => props.anchor,
    () => {
        imgFailed.value = false;
    },
);
watch(
    () => props.anchor.avatar_url,
    () => {
        imgFailed.value = false;
    },
);

/** reka AvatarImage 预加载状态回调：error 时置位，卸载 img 并显示 AvatarFallback */
function onAvatarError(status: "idle" | "loading" | "loaded" | "error") {
    if (status === "error") imgFailed.value = true;
}

/** 标签展示：≤4 个全显示；>4 个显示前 3 个 + 「+N」溢出计数药丸（规格注释，原实现遗漏） */
const visibleTags = computed(() => {
    const tags = props.anchor.tags;
    if (tags.length <= 4) return { shown: tags, overflow: 0 };
    return { shown: tags.slice(0, 3), overflow: tags.length - 3 };
});

function openSettings() {
    emit("settings", props.anchor);
}

function requestRemove() {
    emit("remove", props.anchor);
}

function requestRefresh() {
    emit("refresh", props.anchor);
}
</script>

<template>
    <!-- ── 卡片视图：4:3，头像 3/4 + 信息 1/4 ── -->
    <article
        v-if="view === 'card'"
        tabindex="0"
        role="button"
        :aria-label="anchor.name"
        class="group relative aspect-[4/3] min-w-0 cursor-pointer select-none overflow-hidden rounded-xl border border-border/60 bg-background shadow-sm outline-none transition-[width,box-shadow,transform] duration-300 ease-out hover:shadow-md focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:ring-offset-2"
        @click="openSettings"
        @keydown.enter.prevent="openSettings"
        @keydown.delete.prevent="requestRemove"
    >
        <!-- 头像区（上 3/4）：封面拉伸裁剪 + 渐变遮罩 -->
        <div class="relative h-[75%] w-full overflow-hidden">
            <Avatar class="absolute inset-0 size-full rounded-none">
                <AvatarImage
                    v-if="showAvatar() && !imgFailed && anchor.avatar_url"
                    :src="anchor.avatar_url"
                    referrer-policy="no-referrer"
                    alt=""
                    class="object-cover transition-transform duration-300 group-hover:scale-105"
                    @loading-status-change="onAvatarError"
                />
                <AvatarFallback class="rounded-none">
                    <Mic class="size-14 text-muted-foreground" aria-hidden="true" />
                </AvatarFallback>
            </Avatar>
            <!-- 半透明渐变遮罩：保证右上角徽标文字可读 -->
            <div
                class="absolute inset-0 bg-gradient-to-b from-black/35 via-transparent to-black/40"
                aria-hidden="true"
            />
            <!-- 右上角状态徽标（录制徽标叠在直播下方） -->
            <div class="absolute right-2 top-2 flex flex-col items-end gap-1">
                <StatusBadge :live="isLive" :recording="isRecording" stacked />
            </div>
        </div>

        <!-- 信息区（下 1/4）：行1 名称+菜单 / 行2 房间号+检测+标签 -->
        <div
            class="relative flex h-[25%] w-full flex-col justify-center px-3 py-1.5"
        >
            <!-- 行 1：名称独占一行（truncate）+ 操作菜单 -->
            <div class="flex min-w-0 items-center justify-between gap-2">
                <p class="min-w-0 truncate text-sm font-semibold leading-tight">
                    {{ anchor.name }}
                </p>

                <!-- 操作菜单（设置/删除/刷新） -->
                <div class="shrink-0" @click.stop>
                    <DropdownMenu>
                        <DropdownMenuTrigger as-child>
                            <Button
                                size="icon-sm"
                                variant="ghost"
                                class="rounded-full"
                                :aria-label="t('live.moreActions')"
                            >
                                <MoreVertical class="size-4" />
                            </Button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end" class="w-36">
                            <DropdownMenuItem @select="openSettings">
                                <Settings />
                                {{ t("common.settings") }}
                            </DropdownMenuItem>
                            <DropdownMenuItem @select="requestRefresh">
                                <RefreshCw />
                                {{ t("common.refresh") }}
                            </DropdownMenuItem>
                            <DropdownMenuItem
                                variant="destructive"
                                @select="requestRemove"
                            >
                                <Trash2 />
                                {{ t("common.delete") }}
                            </DropdownMenuItem>
                        </DropdownMenuContent>
                    </DropdownMenu>
                </div>
            </div>

            <!-- 行 2：房间号 + 检测图标 + 标签（≤4 全显，>4 前 3 个 +「+N」计数药丸） -->
            <div
                v-if="
                    showRoomId() ||
                    (showTags() && anchor.tags.length) ||
                    (showStatusIcon() && anchor.enable_check)
                "
                class="mt-1 flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5"
            >
                <span
                    v-if="showRoomId()"
                    class="shrink-0 text-[11px] leading-tight text-muted-foreground"
                    >#{{ anchor.room_id }}</span
                >
                <Radar
                    v-if="showStatusIcon() && anchor.enable_check"
                    class="size-3.5 shrink-0 text-primary"
                    role="img"
                    :aria-label="t('live.detectionEnabled')"
                />
                <Badge
                    v-for="tag in visibleTags.shown"
                    :key="tag"
                    class="bg-primary/10 text-primary text-[11px] leading-tight"
                    >{{ tag }}</Badge
                >
                <Badge
                    v-if="visibleTags.overflow > 0"
                    class="bg-primary/10 text-primary text-[11px] leading-tight"
                    >+{{ visibleTags.overflow }}</Badge
                >
            </div>
        </div>
    </article>

    <!-- ── 列表视图：40px 圆头像 + 名称/标签 + 徽标 + 菜单 ── -->
    <li
        v-else
        tabindex="0"
        role="button"
        :aria-label="anchor.name"
        class="flex cursor-pointer select-none items-center gap-3 px-3 py-[calc(var(--density-mult)*10px)] outline-none transition hover:bg-accent/60 focus-visible:bg-accent/60 focus-visible:ring-2 focus-visible:ring-ring/50"
        @click="openSettings"
        @keydown.enter.prevent="openSettings"
        @keydown.delete.prevent="requestRemove"
    >
        <Avatar class="size-10 shrink-0">
            <AvatarImage
                v-if="showAvatar() && !imgFailed && anchor.avatar_url"
                :src="anchor.avatar_url"
                referrer-policy="no-referrer"
                alt=""
                class="object-cover"
                @loading-status-change="onAvatarError"
            />
            <AvatarFallback>
                <Mic class="size-5 text-muted-foreground" aria-hidden="true" />
            </AvatarFallback>
        </Avatar>

        <!-- 信息区：第一行名称+检测图标，第二行标签 -->
        <div class="min-w-0 flex-1">
            <div class="flex items-center gap-1.5">
                <span class="truncate text-sm font-semibold">{{
                    anchor.name
                }}</span>
                <span
                    v-if="showRoomId()"
                    class="truncate text-xs text-muted-foreground"
                    >#{{ anchor.room_id }}</span
                >
                <div
                    v-if="
                        (showTags() && anchor.tags.length) ||
                        (showStatusIcon() && anchor.enable_check)
                    "
                    class="flex flex-wrap items-center gap-1.5"
                    :style="{ marginTop: `calc(2px * var(--density-mult))` }"
                >
                    <Radar
                        v-if="showStatusIcon() && anchor.enable_check"
                        class="size-3.5 shrink-0 text-primary"
                        role="img"
                        :aria-label="t('live.detectionEnabled')"
                    />
                </div>
                <div
                    v-if="showTags() && anchor.tags.length"
                    class="mt-0.5 flex flex-wrap items-center gap-1.5"
                >
                    <Badge
                        v-for="tag in anchor.tags"
                        :key="tag"
                        class="bg-primary/10 text-primary text-[11px] leading-tight"
                        >{{ tag }}</Badge
                    >
                </div>
            </div>
        </div>

        <!-- 状态区：小尺寸徽标 -->
        <StatusBadge
            size="sm"
            :live="isLive"
            :recording="isRecording"
            class="shrink-0"
        />

        <!-- 操作菜单（与卡片视图一致） -->
        <div @click.stop>
            <DropdownMenu>
                <DropdownMenuTrigger as-child>
                    <Button
                        size="icon-sm"
                        variant="ghost"
                        class="rounded-full"
                        :aria-label="t('live.moreActions')"
                    >
                        <MoreVertical class="size-4" />
                    </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end" class="w-36">
                    <DropdownMenuItem @select="openSettings">
                        <Settings />
                        {{ t("common.settings") }}
                    </DropdownMenuItem>
                    <DropdownMenuItem @select="requestRefresh">
                        <RefreshCw />
                        {{ t("common.refresh") }}
                    </DropdownMenuItem>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem
                        variant="destructive"
                        @select="requestRemove"
                    >
                        <Trash2 />
                        {{ t("common.delete") }}
                    </DropdownMenuItem>
                </DropdownMenuContent>
            </DropdownMenu>
        </div>
    </li>
</template>
