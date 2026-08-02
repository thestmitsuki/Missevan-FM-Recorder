<script setup lang="ts">
/**
 * 文件页（规格「文件页面功能规格」）
 *
 * 布局：左 56px 竖排操作栏（搜索/刷新/筛选/回顶）+ 可选筛选面板 + 右内容区。
 * 数据：fileStore 真实数据（get_recording_files），事件 recording_files_changed
 * 经 events.ts 转发 fileStore 增量更新，保留搜索与筛选状态。
 * 展示：日期分组（今天/昨天/本周/本月/YYYY年M月）+ 分段组（可折叠，整体播放/删除）。
 * 条目：音频图标、文件名加粗（长名省略+悬停 title）、"主播 · 大小 · 时长"、
 * 右侧录制时间 MM-DD HH:mm、三点菜单（播放/重命名/删除）。
 * 四态：加载（Skeleton）/ 空 / 错误+重试 / 搜索无结果。
 * 播放：内置播放器（底部条 + audio 元素，asset 协议加载本地文件）；
 * 分段组"连续播放整组" = 顺序播放各段（ended 自动切下一段）。
 * 键盘：条目 Enter 播放 / Delete 删除（需确认）/ F2 重命名。
 */
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import type { NotificationLevel } from "@/types";
import type { FileGroup, RecordingFile } from "@/types";
import { systemTimeToDate } from "@/types/file";
import {
    AudioLines,
    ChevronRight,
    ListMusic,
    MoreVertical,
    Music,
    Pause,
    Pencil,
    Play,
    RefreshCw,
    Search,
    SlidersHorizontal,
    Trash2,
    Volume2,
    X,
} from "@lucide/vue";

import { useFileStore } from "@/stores/fileStore";
import { usePlayerStore } from "@/stores/playerStore";
import { useNotificationStore } from "@/stores/notificationStore";
import EmptyState from "@/components/common/EmptyState.vue";
import ConfirmDialog from "@/components/common/ConfirmDialog.vue";
import NavRail, { type NavRailItem } from "@/components/common/NavRail.vue";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Skeleton } from "@/components/ui/skeleton";
import {
    Dialog,
    DialogContent,
    DialogFooter,
    DialogHeader,
    DialogTitle,
} from "@/components/ui/dialog";
import {
    DropdownMenu,
    DropdownMenuContent,
    DropdownMenuItem,
    DropdownMenuSeparator,
    DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

const store = useFileStore();
const notifStore = useNotificationStore();
const { t, locale } = useI18n();

// ── 布局状态 ──
const searchOpen = ref(false);
const filterOpen = ref(false);
const showScrollTop = ref(false);
const contentRef = ref<HTMLElement | null>(null);

const SCROLL_TOP_THRESHOLD = 400;

function onContentScroll() {
    showScrollTop.value =
        (contentRef.value?.scrollTop ?? 0) > SCROLL_TOP_THRESHOLD;
}

function scrollToTop() {
    contentRef.value?.scrollTo({ top: 0, behavior: "smooth" });
}

// ── 竖向操作栏（NavRail 配置：搜索 / 刷新 / 筛选 / 回顶）──
const railItems = computed<NavRailItem[]>(() => [
    {
        id: "search",
        icon: Search,
        label: t("files.searchFiles"),
        active: searchOpen.value,
        expanded: searchOpen.value,
        onClick: () => {
            searchOpen.value = !searchOpen.value;
        },
    },
    {
        id: "refresh",
        icon: RefreshCw,
        label: t("files.refreshFiles"),
        onClick: handleRefresh,
    },
    {
        id: "filter",
        icon: SlidersHorizontal,
        label: t("files.filterFiles"),
        active: filterOpen.value,
        expanded: filterOpen.value,
        onClick: () => {
            filterOpen.value = !filterOpen.value;
        },
    },
]);

// ── 日期分组 ──
interface DateGroupInfo {
    key: string;
    label: string;
    rank: number;
    files: RecordingFile[];
}

function groupInfoOf(date: Date): { key: string; label: string; rank: number } {
    if (isNaN(date.getTime())) {
        return { key: "unknown", label: "--", rank: 99 };
    }
    const now = new Date();
    const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
    const target = new Date(
        date.getFullYear(),
        date.getMonth(),
        date.getDate(),
    );
    const diffDays = Math.floor(
        (today.getTime() - target.getTime()) / 86400000,
    );
    if (diffDays === 0)
        return { key: "today", label: t("files.today"), rank: 0 };
    if (diffDays === 1)
        return { key: "yesterday", label: t("files.yesterday"), rank: 1 };
    if (diffDays > 1 && diffDays < 7) {
        return { key: "this-week", label: t("files.thisWeek"), rank: 2 };
    }
    if (diffDays >= 7 && diffDays < 30) {
        return { key: "this-month", label: t("files.thisMonth"), rank: 3 };
    }
    const y = date.getFullYear();
    const m = String(date.getMonth() + 1).padStart(2, "0");
    return {
        key: `${y}-${m}`,
        label: new Intl.DateTimeFormat(locale.value, {
            year: "numeric",
            month: "long",
        }).format(date),
        rank: 4,
    };
}

/** 日期分组（含组内文件，按 rank 升序、年份月份降序） */
const dateGroups = computed<DateGroupInfo[]>(() => {
    const map = new Map<string, DateGroupInfo>();
    for (const f of store.filteredFiles) {
        const info = groupInfoOf(systemTimeToDate(f.created_at));
        let entry = map.get(info.key);
        if (!entry) {
            entry = { ...info, files: [] };
            map.set(info.key, entry);
        }
        entry.files.push(f);
    }
    return [...map.values()].sort(
        (a, b) => a.rank - b.rank || b.key.localeCompare(a.key),
    );
});

// ── 分段组（按组内最早文件时间降序）──
const segmentGroups = computed<FileGroup[]>(() => {
    return [...store.filteredGroups].sort((a, b) => {
        const ta = a.files[0]
            ? systemTimeToDate(a.files[0].created_at).getTime()
            : 0;
        const tb = b.files[0]
            ? systemTimeToDate(b.files[0].created_at).getTime()
            : 0;
        return tb - ta;
    });
});

// ── 折叠状态（localStorage 记忆，默认全部展开）──
const COLLAPSE_KEY = "files:collapsed-groups";
const collapsed = ref<Set<string>>(new Set());

function loadCollapsed() {
    try {
        const raw = localStorage.getItem(COLLAPSE_KEY);
        if (raw) collapsed.value = new Set(JSON.parse(raw) as string[]);
    } catch {
        // 忽略损坏数据
    }
}

function persistCollapsed() {
    try {
        localStorage.setItem(
            COLLAPSE_KEY,
            JSON.stringify([...collapsed.value]),
        );
    } catch {
        // 忽略持久化失败
    }
}

function isCollapsed(key: string) {
    return collapsed.value.has(key);
}

function toggleCollapse(key: string) {
    const next = new Set(collapsed.value);
    if (next.has(key)) {
        next.delete(key);
    } else {
        next.add(key);
    }
    collapsed.value = next;
    persistCollapsed();
}

// ── 格式化 ──
function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    const kb = bytes / 1024;
    if (kb < 1024) return `${kb.toFixed(1)} KB`;
    const mb = kb / 1024;
    if (mb < 1024) return `${mb.toFixed(1)} MB`;
    return `${(mb / 1024).toFixed(1)} GB`;
}

/** 时长：0（获取失败）显示 --:--；否则 H:MM:SS / MM:SS */
function formatDuration(sec: number): string {
    if (!sec || sec <= 0) return "--:--";
    const h = Math.floor(sec / 3600);
    const m = Math.floor((sec % 3600) / 60);
    const s = Math.floor(sec % 60);
    const ss = String(s).padStart(2, "0");
    if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${ss}`;
    return `${m}:${ss}`;
}

/** 录制时间（随 locale 的短日期时间格式，如 07/23 14:05） */
function formatDateShort(date: Date): string {
    if (isNaN(date.getTime())) return "--";
    return new Intl.DateTimeFormat(locale.value, {
        month: "2-digit",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
        hour12: false,
    }).format(date);
}

function formatTime(sec: number): string {
    if (!isFinite(sec) || sec < 0) return "0:00";
    const m = Math.floor(sec / 60);
    const s = Math.floor(sec % 60);
    return `${m}:${String(s).padStart(2, "0")}`;
}

// ── 操作通知（走 notificationStore → App 层 Snackbar）──
function notify(message: string, level: NotificationLevel = "Info") {
    notifStore.addNotification({
        id: `files-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
        code: "files-action",
        level,
        title: message,
        message,
        suggestion: null,
        source: "files",
        timestamp: new Date().toISOString(),
        actionable: false,
    });
}

// ── 删除 ──
type DeleteTarget =
    { kind: "file"; file: RecordingFile } | { kind: "group"; group: FileGroup };

const deleteTarget = ref<DeleteTarget | null>(null);

function requestDeleteFile(file: RecordingFile) {
    // 录制中（FFmpeg 正在写入）：禁止删除，防文件占用错误
    if (file.is_active) return;
    deleteTarget.value = { kind: "file", file };
}

function requestDeleteGroup(group: FileGroup) {
    // 组内任一文件录制中：禁止整组删除
    if (group.files.some((f) => f.is_active)) return;
    deleteTarget.value = { kind: "group", group };
}

const deleteConfirmMessage = computed(() => {
    const target = deleteTarget.value;
    if (!target) return "";
    if (target.kind === "file") return t("files.deleteConfirmMessage");
    return t("files.deleteGroupConfirmMessage", {
        count: target.group.files.length,
    });
});

async function confirmDelete() {
    const target = deleteTarget.value;
    deleteTarget.value = null;
    if (!target) return;
    try {
        if (target.kind === "file") {
            await store.deleteFile(target.file.path);
        } else {
            await Promise.all(
                target.group.files.map((f) => store.deleteFile(f.path)),
            );
        }
        notify(t("files.deleteSuccess"));
    } catch (e) {
        notify(t("files.deleteFailed", { error: String(e) }), "Error");
    }
}

// ── 重命名 ──
const INVALID_NAME_CHARS = /[\\/:*?"<>|]/;

const renameTarget = ref<RecordingFile | null>(null);
const renameValue = ref("");
const renameError = ref("");

function startRename(file: RecordingFile) {
    // 录制中（FFmpeg 正在写入）：禁止重命名，防文件占用错误
    if (file.is_active) return;
    renameTarget.value = file;
    renameValue.value = file.name.replace(/\.[^.]+$/, ""); // 不含扩展名
    renameError.value = "";
}

function extOf(name: string): string {
    const dot = name.lastIndexOf(".");
    return dot >= 0 ? name.slice(dot + 1) : "";
}

/** 客户端预校验：空名 / 非法字符 / 同名冲突（红字错误，不提交） */
function validateRename(): string {
    const name = renameValue.value.trim();
    if (!name) return t("files.renameErrorEmpty");
    if (INVALID_NAME_CHARS.test(name)) return t("files.renameErrorInvalid");
    const target = renameTarget.value;
    if (!target) return "";
    const candidate = `${name}.${extOf(target.name)}`;
    const exists = store.files
        .concat(store.groups.flatMap((g) => g.files))
        .some(
            (f) =>
                f.id !== target.id &&
                f.name.toLowerCase() === candidate.toLowerCase(),
        );
    return exists ? t("files.renameErrorExists") : "";
}

async function confirmRename() {
    const target = renameTarget.value;
    if (!target) return;
    const error = validateRename();
    if (error) {
        renameError.value = error;
        return;
    }
    try {
        await store.renameFile(target.path, renameValue.value.trim());
        renameTarget.value = null;
        notify(t("files.renameSuccess"));
    } catch (e) {
        // 后端错误（如磁盘上的同名冲突/权限不足）红字显示在对话框内
        renameError.value = String(e);
    }
}

// ── 刷新 ──
async function handleRefresh() {
    try {
        await store.refreshFiles();
    } catch (e) {
        notify(t("files.refreshFailed", { error: String(e) }), "Error");
    }
}

// ── 播放（内置播放器；音频生命周期全局：playerStore 单例 audio 挂 document.body）──
// 页面卸载不影响播放（audio 不随组件销毁）；切回本页时 UI 从 store 恢复
// （onMounted 里 syncState 兜底同步）。分段组连续播放 = store 队列顺序播放。
const player = usePlayerStore();

function isCurrent(file: RecordingFile) {
    return player.currentFile?.id === file.id;
}

/** 单文件 / 分段组整组播放（队列迁入 playerStore） */
function playFiles(files: RecordingFile[]) {
    player.playFiles(files);
}

const progressPercent = computed(() => {
    if (!player.duration) return 0;
    return Math.min(100, (player.currentTime / player.duration) * 100);
});

function seekAudio(e: MouseEvent) {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    player.seek((e.clientX - rect.left) / rect.width);
}

function onVolumeInput(e: Event) {
    player.setVolume(Number((e.target as HTMLInputElement).value));
}

// ── 生命周期 ──
onMounted(() => {
    loadCollapsed();
    contentRef.value?.addEventListener("scroll", onContentScroll, {
        passive: true,
    });
    // 页面卸载期间播放继续：挂载时把单例 audio 实时值同步进 store（UI 恢复）
    player.syncState();
    store.startListener();
    void store.fetchFiles().catch((e) => {
        console.error("Failed to load file list", e);
    });
});

onBeforeUnmount(() => {
    contentRef.value?.removeEventListener("scroll", onContentScroll);
    store.stopListener();
});
</script>

<template>
    <div class="relative flex h-full min-w-0 flex-1">
        <!-- 左侧竖排操作栏（NavRail 通用组件，配置式） -->
        <NavRail
            :items="railItems"
            :aria-label="t('nav.fileManager')"
            :show-scroll-top="showScrollTop"
            :scroll-top-label="t('files.backToTop')"
            @scroll-top="scrollToTop"
        />

        <!-- 遮罩层：点击外部关闭面板（z-20 低于面板 z-30） -->
        <div
            v-if="filterOpen"
            class="fixed inset-0 z-20 bg-transparent"
            @click="filterOpen = false"
        />
        <!-- 筛选面板（悬浮覆盖在内容区之上，不挤压内容布局；与搜索框叠加生效） -->
        <aside
            v-if="filterOpen"
            class="absolute h-[45vh] max-h-[90vh] left-14 top-0 z-30 flex w-64 flex-col gap-5 overflow-y-auto bg-background/95 p-4 backdrop-blur rounded-lg max-[720px]:left-12"
            :aria-label="t('files.filterFiles')"
        >
            <div class="flex items-center justify-between">
                <h2 class="text-sm font-semibold">
                    {{ t("files.filterFiles") }}
                </h2>
                <Button
                    v-if="store.hasActiveFilters"
                    variant="ghost"
                    size="sm"
                    class="h-7 px-2 text-xs"
                    @click="store.clearFilters()"
                >
                    {{ t("files.clearFilters") }}
                </Button>
            </div>

            <!-- 按主播名模糊筛选 -->
            <div class="flex flex-col gap-2">
                <Label class="text-xs text-muted-foreground">
                    {{ t("files.filterByAnchor") }}
                </Label>
                <Input
                    v-model="store.anchorQuery"
                    :placeholder="t('files.filterByAnchor')"
                    class="h-8"
                />
            </div>

            <!-- 按文件类型筛选 -->
            <div class="flex flex-col gap-2">
                <Label class="text-xs text-muted-foreground">
                    {{ t("files.filterByType") }}
                </Label>
                <RadioGroup
                    v-model="store.typeFilter"
                    class="flex flex-col gap-1.5"
                >
                    <label
                        v-for="opt in [
                            { value: 'all', label: 'files.allTypes' },
                            { value: 'm4a', label: 'M4A' },
                            { value: 'mp3', label: 'MP3' },
                        ]"
                        :key="opt.value"
                        class="flex cursor-pointer items-center gap-2 text-sm"
                    >
                        <RadioGroupItem :value="opt.value" class="size-4" />
                        <span>{{ t(opt.label) }}</span>
                    </label>
                </RadioGroup>
            </div>

            <!-- 按日期范围筛选（ui 组件缺 date picker，用原生 date 输入） -->
            <div class="flex flex-col gap-2">
                <Label class="text-xs text-muted-foreground">
                    {{ t("files.filterByDate") }}
                </Label>
                <div
                    class="flex items-center gap-2 text-xs text-muted-foreground"
                >
                    <Input
                        v-model="store.dateRange.start"
                        type="date"
                        class="h-8"
                        :aria-label="t('files.startDate')"
                    />
                    <span>–</span>
                    <Input
                        v-model="store.dateRange.end"
                        type="date"
                        class="h-8"
                        :aria-label="t('files.endDate')"
                    />
                </div>
            </div>
        </aside>

        <!-- 内容区（本页滚动容器；居中限宽显示，不贴边） -->
        <section
            ref="contentRef"
            class="page-scroll mx-auto min-w-0 max-w-[1200px] flex-1 overflow-y-auto px-4 py-4"
            :aria-busy="store.loading"
        >
            <!-- 加载态：Skeleton 行 -->
            <div v-if="store.loading" class="flex flex-col gap-3">
                <Skeleton v-for="i in 6" :key="i" class="h-14 rounded-lg" />
            </div>

            <!-- 错误态：提示 + 重试 -->
            <EmptyState
                v-else-if="store.error"
                icon="⚠️"
                :title="t('files.loadFailed')"
                :description="String(store.error)"
                :action-label="t('files.retry')"
                @action="() => store.fetchFiles()"
            />

            <!-- 空态：没有任何录制文件 -->
            <EmptyState
                v-else-if="!store.hasAnyFiles"
                icon="🎧"
                :title="t('files.noAudioFiles')"
                :description="t('files.noAudioFilesDesc')"
            />

            <template v-else>
                <!-- 搜索栏（由左侧搜索按钮控制显示） -->
                <div v-if="searchOpen" class="relative mb-2 flex items-center">
                    <Search
                        class="pointer-events-none absolute left-3 size-4 text-muted-foreground"
                        aria-hidden="true"
                    />
                    <Input
                        v-model="store.searchQuery"
                        :placeholder="t('files.searchPlaceholder')"
                        class="h-9 pl-9 pr-9"
                    />
                    <Button
                        v-if="store.searchQuery"
                        variant="ghost"
                        size="icon-sm"
                        class="absolute right-1.5 rounded-full"
                        :aria-label="t('files.clearSearch')"
                        @click="store.searchQuery = ''"
                    >
                        <X class="size-4" />
                    </Button>
                </div>

                <!-- 搜索/筛选无结果 -->
                <EmptyState
                    v-if="!store.hasFilteredFiles"
                    icon="🔍"
                    :title="t('files.noResults')"
                    :description="
                        store.searchQuery.trim()
                            ? t('files.noMatchDesc', {
                                  query: store.searchQuery.trim(),
                              })
                            : ''
                    "
                />

                <!-- 分组列表：日期分组 + 分段组（折叠状态记忆） -->
                <template v-else>
                    <template
                        v-for="group in dateGroups"
                        :key="`date:${group.key}`"
                    >
                        <!-- 日期分组标题：名称 + 数量 + 折叠 -->
                        <div
                            role="button"
                            tabindex="0"
                            class="flex cursor-pointer select-none items-center gap-2 px-3 pb-1 pt-5 outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
                            :aria-expanded="!isCollapsed(`date:${group.key}`)"
                            @click="toggleCollapse(`date:${group.key}`)"
                            @keydown.enter.prevent="
                                toggleCollapse(`date:${group.key}`)
                            "
                            @keydown.space.prevent="
                                toggleCollapse(`date:${group.key}`)
                            "
                        >
                            <ChevronRight
                                class="size-4 shrink-0 text-muted-foreground transition-transform"
                                :class="
                                    isCollapsed(`date:${group.key}`)
                                        ? ''
                                        : 'rotate-90'
                                "
                                aria-hidden="true"
                            />
                            <span class="text-sm font-semibold">{{
                                group.label
                            }}</span>
                            <span class="text-xs text-muted-foreground">
                                {{
                                    t("files.fileCount", {
                                        count: group.files.length,
                                    })
                                }}
                            </span>
                        </div>

                        <!-- 单文件条目 -->
                        <template v-if="!isCollapsed(`date:${group.key}`)">
                            <div
                                v-for="file in group.files"
                                :key="file.id"
                                role="button"
                                tabindex="0"
                                :title="file.name"
                                class="group flex cursor-pointer select-none items-center gap-3 rounded-lg px-3 py-2.5 outline-none transition hover:bg-accent/60 focus-visible:bg-accent/60 focus-visible:ring-2 focus-visible:ring-ring/50"
                                @dblclick="playFiles([file])"
                                @keydown.enter.prevent="playFiles([file])"
                                @keydown.delete.prevent="
                                    requestDeleteFile(file)
                                "
                                @keydown.f2.prevent="startRename(file)"
                            >
                                <div
                                    class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary"
                                >
                                    <AudioLines
                                        v-if="isCurrent(file)"
                                        class="size-4 animate-pulse"
                                        aria-hidden="true"
                                    />
                                    <Music
                                        v-else
                                        class="size-4"
                                        aria-hidden="true"
                                    />
                                </div>
                                <div class="min-w-0 flex-1">
                                    <p
                                        class="flex min-w-0 items-center gap-1.5"
                                    >
                                        <span
                                            class="truncate text-sm font-semibold"
                                        >
                                            {{ file.name }}
                                        </span>
                                        <!-- 录制中标记：正被 FFmpeg 写入，禁删/禁重命名 -->
                                        <span
                                            v-if="file.is_active"
                                            class="flex shrink-0 items-center gap-1 rounded-full bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-medium leading-tight text-amber-600 dark:text-amber-400"
                                        >
                                            <span
                                                class="size-1.5 animate-pulse rounded-full bg-amber-500"
                                            />
                                            {{ t("files.recordingActive") }}
                                        </span>
                                    </p>
                                </div>
                                <span
                                    class="shrink-0 text-xs tabular-nums text-muted-foreground"
                                >
                                    {{
                                        formatDateShort(
                                            systemTimeToDate(file.created_at),
                                        )
                                    }}
                                </span>
                                <div
                                    class="shrink-0 opacity-0 transition group-hover:opacity-100 focus-within:opacity-100"
                                    @click.stop
                                >
                                    <DropdownMenu>
                                        <DropdownMenuTrigger as-child>
                                            <Button
                                                size="icon-sm"
                                                variant="ghost"
                                                class="rounded-full"
                                                :aria-label="t('files.play')"
                                            >
                                                <MoreVertical class="size-4" />
                                            </Button>
                                        </DropdownMenuTrigger>
                                        <DropdownMenuContent
                                            align="end"
                                            class="w-36"
                                        >
                                            <DropdownMenuItem
                                                @select="playFiles([file])"
                                            >
                                                <Play />
                                                {{ t("files.play") }}
                                            </DropdownMenuItem>
                                            <DropdownMenuItem
                                                :disabled="file.is_active"
                                                @select="startRename(file)"
                                            >
                                                <Pencil />
                                                {{ t("files.rename") }}
                                            </DropdownMenuItem>
                                            <DropdownMenuSeparator />
                                            <DropdownMenuItem
                                                variant="destructive"
                                                :disabled="file.is_active"
                                                @select="
                                                    requestDeleteFile(file)
                                                "
                                            >
                                                <Trash2 />
                                                {{ t("files.delete") }}
                                            </DropdownMenuItem>
                                        </DropdownMenuContent>
                                    </DropdownMenu>
                                </div>
                            </div>
                        </template>
                    </template>

                    <!-- 分段组 -->
                    <template
                        v-for="group in segmentGroups"
                        :key="`seg:${group.prefix}`"
                    >
                        <!-- 分段组标题：基础名（x 个分段）+ 折叠 + 整组菜单 -->
                        <div
                            role="button"
                            tabindex="0"
                            class="flex cursor-pointer select-none items-center gap-2 rounded-lg bg-muted/60 px-3 py-2.5 outline-none transition hover:bg-accent/60 focus-visible:bg-accent/60 focus-visible:ring-2 focus-visible:ring-ring/50"
                            :aria-expanded="!isCollapsed(`seg:${group.prefix}`)"
                            :aria-label="`${group.prefix} (${t('files.segments', { count: group.files.length })})`"
                            @click="toggleCollapse(`seg:${group.prefix}`)"
                            @keydown.enter.prevent="
                                toggleCollapse(`seg:${group.prefix}`)
                            "
                            @keydown.space.prevent="
                                toggleCollapse(`seg:${group.prefix}`)
                            "
                        >
                            <ChevronRight
                                class="size-4 shrink-0 text-muted-foreground transition-transform"
                                :class="
                                    isCollapsed(`seg:${group.prefix}`)
                                        ? ''
                                        : 'rotate-90'
                                "
                                aria-hidden="true"
                            />
                            <ListMusic
                                class="size-4 shrink-0 text-primary"
                                aria-hidden="true"
                            />
                            <span
                                class="min-w-0 truncate text-sm font-semibold"
                            >
                                {{ group.prefix }}
                            </span>
                            <span
                                class="shrink-0 rounded-full bg-primary/10 px-2 py-0.5 text-[11px] font-medium leading-tight text-primary"
                            >
                                {{
                                    t("files.segments", {
                                        count: group.files.length,
                                    })
                                }}
                            </span>
                            <span
                                class="ml-auto hidden shrink-0 truncate text-xs text-muted-foreground md:inline"
                            >
                                {{ group.files[0]?.anchor_name }}
                                <span class="mx-1">·</span>
                                {{ formatSize(group.total_size) }}
                                <span class="mx-1">·</span>
                                {{ formatDuration(group.total_duration) }}
                            </span>
                            <div class="shrink-0" @click.stop>
                                <DropdownMenu>
                                    <DropdownMenuTrigger as-child>
                                        <Button
                                            size="icon-sm"
                                            variant="ghost"
                                            class="rounded-full"
                                            :aria-label="t('files.playGroup')"
                                        >
                                            <MoreVertical class="size-4" />
                                        </Button>
                                    </DropdownMenuTrigger>
                                    <DropdownMenuContent
                                        align="end"
                                        class="w-44"
                                    >
                                        <DropdownMenuItem
                                            @select="playFiles(group.files)"
                                        >
                                            <Play />
                                            {{ t("files.playGroup") }}
                                        </DropdownMenuItem>
                                        <DropdownMenuSeparator />
                                        <DropdownMenuItem
                                            variant="destructive"
                                            :disabled="
                                                group.files.some(
                                                    (f) => f.is_active,
                                                )
                                            "
                                            @select="requestDeleteGroup(group)"
                                        >
                                            <Trash2 />
                                            {{ t("files.deleteGroup") }}
                                        </DropdownMenuItem>
                                    </DropdownMenuContent>
                                </DropdownMenu>
                            </div>
                        </div>

                        <!-- 分段条目（缩进展示，单段独立操作） -->
                        <template v-if="!isCollapsed(`seg:${group.prefix}`)">
                            <div
                                v-for="file in group.files"
                                :key="`seg:${file.id}`"
                                role="button"
                                tabindex="0"
                                :title="file.name"
                                class="group flex cursor-pointer select-none items-center gap-3 rounded-lg py-2.5 pl-9 pr-3 outline-none transition hover:bg-accent/60 focus-visible:bg-accent/60 focus-visible:ring-2 focus-visible:ring-ring/50"
                                @dblclick="playFiles([file])"
                                @keydown.enter.prevent="playFiles([file])"
                                @keydown.delete.prevent="
                                    requestDeleteFile(file)
                                "
                                @keydown.f2.prevent="startRename(file)"
                            >
                                <div
                                    class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary"
                                >
                                    <AudioLines
                                        v-if="isCurrent(file)"
                                        class="size-4 animate-pulse"
                                        aria-hidden="true"
                                    />
                                    <Music
                                        v-else
                                        class="size-4"
                                        aria-hidden="true"
                                    />
                                </div>
                                <div class="min-w-0 flex-1">
                                    <p
                                        class="flex min-w-0 items-center gap-1.5"
                                    >
                                        <span
                                            class="truncate text-sm font-semibold"
                                        >
                                            {{ file.name }}
                                        </span>
                                        <!-- 录制中标记：正被 FFmpeg 写入，禁删/禁重命名 -->
                                        <span
                                            v-if="file.is_active"
                                            class="flex shrink-0 items-center gap-1 rounded-full bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-medium leading-tight text-amber-600 dark:text-amber-400"
                                        >
                                            <span
                                                class="size-1.5 animate-pulse rounded-full bg-amber-500"
                                            />
                                            {{ t("files.recordingActive") }}
                                        </span>
                                    </p>
                                </div>
                                <span
                                    class="shrink-0 text-xs tabular-nums text-muted-foreground"
                                >
                                    {{
                                        formatDateShort(
                                            systemTimeToDate(file.created_at),
                                        )
                                    }}
                                </span>
                                <div
                                    class="shrink-0 opacity-0 transition group-hover:opacity-100 focus-within:opacity-100"
                                    @click.stop
                                >
                                    <DropdownMenu>
                                        <DropdownMenuTrigger as-child>
                                            <Button
                                                size="icon-sm"
                                                variant="ghost"
                                                class="rounded-full"
                                                :aria-label="t('files.play')"
                                            >
                                                <MoreVertical class="size-4" />
                                            </Button>
                                        </DropdownMenuTrigger>
                                        <DropdownMenuContent
                                            align="end"
                                            class="w-36"
                                        >
                                            <DropdownMenuItem
                                                @select="playFiles([file])"
                                            >
                                                <Play />
                                                {{ t("files.play") }}
                                            </DropdownMenuItem>
                                            <DropdownMenuItem
                                                :disabled="file.is_active"
                                                @select="startRename(file)"
                                            >
                                                <Pencil />
                                                {{ t("files.rename") }}
                                            </DropdownMenuItem>
                                            <DropdownMenuSeparator />
                                            <DropdownMenuItem
                                                variant="destructive"
                                                :disabled="file.is_active"
                                                @select="
                                                    requestDeleteFile(file)
                                                "
                                            >
                                                <Trash2 />
                                                {{ t("files.delete") }}
                                            </DropdownMenuItem>
                                        </DropdownMenuContent>
                                    </DropdownMenu>
                                </div>
                            </div>
                        </template>
                    </template>
                </template>
            </template>
        </section>

        <!-- 删除确认（文件 / 分段组整组） -->
        <ConfirmDialog
            :open="!!deleteTarget"
            :title="t('files.deleteConfirmTitle')"
            :message="deleteConfirmMessage"
            destructive
            @confirm="confirmDelete"
            @cancel="deleteTarget = null"
        />

        <!-- 重命名对话框（预填不含扩展名；非法/同名红字错误） -->
        <Dialog
            :open="!!renameTarget"
            @update:open="
                (open) => {
                    if (!open) renameTarget = null;
                }
            "
        >
            <DialogContent class="max-w-sm">
                <DialogHeader>
                    <DialogTitle>{{
                        t("files.renameDialogTitle")
                    }}</DialogTitle>
                </DialogHeader>
                <div class="flex flex-col gap-2 py-2">
                    <Label class="text-xs text-muted-foreground">
                        {{ t("files.renameLabel") }}
                    </Label>
                    <Input
                        v-model="renameValue"
                        :placeholder="t('files.renameLabel')"
                        class="h-9"
                        @keydown.enter.prevent="confirmRename"
                    />
                    <p
                        v-if="renameError"
                        class="text-xs text-destructive"
                        role="alert"
                    >
                        {{ renameError }}
                    </p>
                </div>
                <DialogFooter>
                    <Button variant="ghost" @click="renameTarget = null">
                        {{ t("common.cancel") }}
                    </Button>
                    <Button
                        :disabled="!renameValue.trim()"
                        @click="confirmRename"
                    >
                        {{ t("common.ok") }}
                    </Button>
                </DialogFooter>
            </DialogContent>
        </Dialog>

        <!-- 内置播放器（底部浮条；分段组连续播放时显示进度段号）。
             UI 在文件页，但音频生命周期全局（playerStore 单例 audio 挂
             document.body）：切页不停止播放，切回时 UI 从 store 恢复。 -->
        <div
            v-if="player.currentFile"
            class="fixed bottom-4 left-1/2 z-50 flex w-[min(560px,calc(100vw-2rem))] -translate-x-1/2 items-center gap-3 rounded-xl border border-border/70 bg-background/95 p-3 shadow-lg backdrop-blur"
            :aria-label="t('files.nowPlaying')"
        >
            <Button
                size="icon"
                variant="ghost"
                class="size-9 shrink-0 rounded-full"
                :aria-label="player.playing ? t('files.pause') : t('files.play')"
                @click="player.togglePlay()"
            >
                <Pause v-if="player.playing" class="size-4" />
                <Play v-else class="size-4" />
            </Button>

            <div class="min-w-0 flex-1">
                <div class="flex items-center gap-2">
                    <p class="truncate text-xs font-semibold">
                        {{ player.currentFile.name }}
                    </p>
                    <span
                        v-if="player.isGroupPlay"
                        class="shrink-0 rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-primary"
                    >
                        {{
                            t("files.segmentProgress", {
                                current: player.queueIndex + 1,
                                total: player.queue.length,
                            })
                        }}
                    </span>
                </div>
                <div class="mt-1 flex items-center gap-2">
                    <span
                        class="w-9 shrink-0 text-right text-[10px] tabular-nums text-muted-foreground"
                    >
                        {{ formatTime(player.currentTime) }}
                    </span>
                    <div
                        class="h-1.5 flex-1 cursor-pointer rounded-full bg-muted"
                        role="slider"
                        :aria-valuemin="0"
                        :aria-valuemax="player.duration || 0"
                        :aria-valuenow="player.currentTime"
                        :aria-label="t('files.playerSeek')"
                        @click="seekAudio"
                    >
                        <div
                            class="h-1.5 rounded-full bg-primary"
                            :style="{ width: progressPercent + '%' }"
                        />
                    </div>
                    <span
                        class="w-9 shrink-0 text-[10px] tabular-nums text-muted-foreground"
                    >
                        {{ formatTime(player.duration) }}
                    </span>
                </div>
            </div>

            <div class="flex shrink-0 items-center gap-1">
                <Volume2
                    class="size-3.5 text-muted-foreground"
                    aria-hidden="true"
                />
                <input
                    type="range"
                    min="0"
                    max="1"
                    step="0.01"
                    :value="player.volume"
                    class="w-14 accent-[var(--primary)]"
                    :aria-label="t('files.playerVolume')"
                    @input="onVolumeInput"
                />
                <Button
                    size="icon"
                    variant="ghost"
                    class="size-8 rounded-full"
                    :aria-label="t('common.close')"
                    @click="player.stopPlayback()"
                >
                    <X class="size-4" />
                </Button>
            </div>
        </div>
    </div>
</template>
