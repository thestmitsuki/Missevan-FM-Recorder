<script setup lang="ts">
/**
 * 文件页（规格「文件页面功能规格」）
 *
 * 布局：左 56px 竖排操作栏（搜索/刷新/筛选/回顶）+ 可选筛选面板 + 右内容区。
 * 数据：fileStore 真实数据（get_recording_files），事件 recording_files_changed
 * 经 events.ts 转发 fileStore 增量更新，保留搜索与筛选状态。
 * 展示：月份 → 主播文件夹 → 全部音频文件（不再有分段概念；主播文件夹内
 * 全部音频文件平铺展示，可折叠）。
 * 条目：音频图标、文件名加粗（长名省略+悬停 title）、"主播 · 大小 · 时长"、
 * 右侧录制时间 MM-DD HH:mm、三点菜单（播放/重命名/删除）。
 * 四态：加载（Skeleton）/ 空 / 错误+重试 / 搜索无结果。
 * 播放：内置播放器（底部条 + audio 元素，asset 协议加载本地文件）；
 * 文件夹头「更多」菜单：播放全部（连续播放该主播本月全部音频）/
 * 删除全部（仅删除该文件夹头即本月内容，跨月不受影响）。
 * 键盘：条目 Enter 播放 / Delete 删除（需确认）/ F2 重命名。
 *
 * 性能优化（审计 11.3）：
 * - 搜索/主播筛选输入去抖（本地输入 ref → 280ms 去抖 → 写 store；筛选语义不变）。
 * - 分组列表虚拟滚动：月份头 + 文件夹头 + 文件行扁平化为单层
 *   固定行高列表，只渲染视口内（含 overscan）条目，DOM 数量与文件总量解耦。
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import type { NotificationLevel } from "@/types";
import type { FileFolder, RecordingFile } from "@/types";
import { systemTimeToDate } from "@/types/file";
import { debounce } from "@/lib/debounce";
import { computeVisibleRange } from "@/lib/virtualList";
import {
    ArrowUpDown,
    AudioLines,
    ChevronRight,
    Eraser,
    Folder,
    FolderOpen,
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
import { api } from "@/services/api";
import EmptyState from "@/components/common/EmptyState.vue";
import ConfirmDialog from "@/components/common/ConfirmDialog.vue";
import NavRail, { type NavRailItem } from "@/components/common/NavRail.vue";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Popover, PopoverAnchor, PopoverContent } from "@/components/ui/popover";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Skeleton } from "@/components/ui/skeleton";
import { Slider } from "@/components/ui/slider";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
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

// ── 筛选面板 Popover 无障碍：ESC/关闭后焦点归还触发器按钮 ──
// 触发器是 PopoverAnchor + 手动 Button（非 PopoverTrigger），reka 的
// closeAutoFocus 拿不到 triggerElement 不会自动归还焦点（焦点落 body），
// 因此监听 escape-key-down 在关闭瞬间手动把焦点还给筛选按钮。
const filterBtnEl = ref<HTMLElement | null>(null);

/** Button 组件 ref 回调：取组件根元素（原生 button） */
function onFilterBtnRef(el: unknown) {
    if (el && typeof el === "object" && "$el" in el) {
        filterBtnEl.value = (el as { $el?: unknown }).$el as HTMLElement | null;
    } else {
        filterBtnEl.value = el as HTMLElement | null;
    }
}

/** ESC 关闭筛选面板后把焦点还给触发器按钮 */
function onFilterEscapeKeyDown() {
    filterBtnEl.value?.focus();
}

// ── 虚拟滚动运行态：列表相对滚动位置 + 视口高度 ──
const listRef = ref<HTMLElement | null>(null);
const virtualScrollTop = ref(0);
const virtualViewportH = ref(0);
let listResizeObserver: ResizeObserver | null = null;
/** 折叠/展开切换前的视口首条条目 id：flatItems 重建后恢复其位置（防跳动） */
let pendingAnchorId: string | null = null;
/** 锚点条目在视口内的 Y 坐标（折叠/展开前捕获，重建后按此恢复） */
let pendingAnchorViewportY = 0;

/** 立即清理确认对话框开关（railItems 引用，需先声明） */
const cleanupOpen = ref(false);

const SCROLL_TOP_THRESHOLD = 400;

function onContentScroll() {
    const c = contentRef.value;
    showScrollTop.value = (c?.scrollTop ?? 0) > SCROLL_TOP_THRESHOLD;
    // 虚拟滚动：刷新列表相对滚动位置与视口高度（列表偏移随布局变化，实时读取）
    if (c) {
        virtualViewportH.value = c.clientHeight;
        virtualScrollTop.value = readVirtualScrollTop();
    }
}

function scrollToTop() {
    contentRef.value?.scrollTo({ top: 0, behavior: "smooth" });
}

// ── 搜索/主播筛选去抖（输入层本地 ref → 去抖后写 store）──
// 仅推迟计算，不改变筛选匹配逻辑；store 侧程序化改动（如清除筛选）回写输入框
const SEARCH_DEBOUNCE_MS = 280;

/** 搜索框本地输入（:model-value 单向 + update 事件，去抖后写 store.searchQuery） */
const searchInput = ref("");
/** 筛选面板主播名本地输入（同理，去抖后写 store.anchorQuery） */
const anchorInput = ref("");

const commitSearchQuery = debounce((v: string) => {
    store.searchQuery = v;
}, SEARCH_DEBOUNCE_MS);
const commitAnchorQuery = debounce((v: string) => {
    store.anchorQuery = v;
}, SEARCH_DEBOUNCE_MS);

function onSearchInput(v: string | number) {
    searchInput.value = String(v);
    commitSearchQuery(searchInput.value);
}

function onAnchorInput(v: string | number) {
    anchorInput.value = String(v);
    commitAnchorQuery(anchorInput.value);
}

/** 清空搜索：取消挂起中的去抖提交，输入框与 store 同步清空 */
function clearSearch() {
    commitSearchQuery.cancel();
    searchInput.value = "";
    store.searchQuery = "";
}

// store 侧被程序修改（如 clearFilters）时：取消挂起提交并回写本地输入框
watch(
    () => store.searchQuery,
    (v) => {
        if (searchInput.value !== v) {
            commitSearchQuery.cancel();
            searchInput.value = v;
        }
    },
);
watch(
    () => store.anchorQuery,
    (v) => {
        if (anchorInput.value !== v) {
            commitAnchorQuery.cancel();
            anchorInput.value = v;
        }
    },
);

// ── 竖向操作栏（NavRail 配置：搜索 / 刷新 / 筛选 / 打开输出目录 / 立即清理 / 回顶）──
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
        label: t("files.filterFiles"),
        slotName: "filter", // 自定义触发器：PopoverTrigger 接管展开/收起（见模板 #filter 插槽）
    },
    {
        id: "open-dir",
        icon: FolderOpen,
        label: t("files.openOutputDir"),
        onClick: handleOpenOutputDir,
    },
    {
        id: "cleanup",
        icon: Eraser,
        label: t("files.cleanupNow"),
        onClick: () => {
            cleanupOpen.value = true;
        },
    },
]);

// ── 月份 × 文件夹树（文件管理结构：月份 → 主播文件夹 → 全部音频文件）──
// 文件夹直接来自后端文件夹树：name=主播名（已剥离 `-房间号`）、path=磁盘
// 文件夹身份键；name 为空（输出目录根下文件）归入「未分类」伪文件夹。
// 后端不再按文件名推测分段，文件夹内全部音频文件平铺展示。
const UNCATEGORIZED = "__uncategorized__";

/** 日历月键（m:YYYY-MM，补零保证字典序=时间序）；无效日期 → m:unknown */
function monthKeyOf(date: Date): string {
    if (isNaN(date.getTime())) return "m:unknown";
    return `m:${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}`;
}

function monthLabelOf(date: Date): string {
    if (isNaN(date.getTime())) return "--";
    return new Intl.DateTimeFormat(locale.value, {
        year: "numeric",
        month: "long",
    }).format(date);
}

function isCurrentMonth(date: Date): boolean {
    const now = new Date();
    return (
        !isNaN(date.getTime()) &&
        date.getFullYear() === now.getFullYear() &&
        date.getMonth() === now.getMonth()
    );
}

/** 文件夹内排序方式（月份条右侧控件，按月记忆） */
type FolderSortMode = "latest" | "name-asc" | "name-desc";
const MONTH_SORTS_KEY = "files:month-sorts";
const monthSorts = ref<Record<string, FolderSortMode>>({});

function loadMonthSorts() {
    try {
        const raw = localStorage.getItem(MONTH_SORTS_KEY);
        if (raw) {
            monthSorts.value = JSON.parse(raw) as Record<string, FolderSortMode>;
        }
    } catch {
        // 忽略损坏数据
    }
}

function monthSortOf(monthKey: string): FolderSortMode {
    return monthSorts.value[monthKey] ?? "latest";
}

function setMonthSort(monthKey: string, mode: FolderSortMode) {
    monthSorts.value = { ...monthSorts.value, [monthKey]: mode };
    try {
        localStorage.setItem(
            MONTH_SORTS_KEY,
            JSON.stringify(monthSorts.value),
        );
    } catch {
        // 忽略持久化失败
    }
}

interface FolderNode {
    /** 主播名（后端已剥离 `-房间号`；""=未分类，显示「未分类」） */
    name: string;
    /** 磁盘文件夹路径（月份内唯一身份键；后端 folder.path） */
    path: string;
    /** 该主播该文件夹在本月内的音频文件（时间降序） */
    files: RecordingFile[];
    /** 该主播该文件夹全部音频文件（跨月，播放「播放全部」用，时间降序） */
    playlist: RecordingFile[];
    /** 文件夹内最新文件时间戳（排序用） */
    latestAt: number;
    totalSize: number;
}

interface MonthNode {
    key: string; // m:YYYY-MM
    label: string;
    isCurrent: boolean;
    folders: FolderNode[];
    /** 音频文件总数 */
    fileCount: number;
}

const monthTree = computed<MonthNode[]>(() => {
    const months = new Map<
        string,
        { label: string; isCurrent: boolean; folders: Map<string, FolderNode> }
    >();
    const monthOf = (date: Date) => {
        const key = monthKeyOf(date);
        let m = months.get(key);
        if (!m) {
            m = {
                label: monthLabelOf(date),
                isCurrent: isCurrentMonth(date),
                folders: new Map(),
            };
            months.set(key, m);
        }
        return m;
    };
    const folderOf = (
        m: { folders: Map<string, FolderNode> },
        folder: FileFolder,
    ) => {
        let f = m.folders.get(folder.path);
        if (!f) {
            f = {
                name: folder.name || UNCATEGORIZED,
                path: folder.path,
                files: [],
                playlist: [],
                latestAt: 0,
                totalSize: 0,
            };
            m.folders.set(folder.path, f);
        }
        return f;
    };
    const fileTime = (f: RecordingFile) =>
        systemTimeToDate(f.created_at).getTime();

    // 文件夹树 → 月份 × 文件夹（文件夹内全部音频文件按 created_at 归月；
    // playlist 与 files 同引用 = 本月该文件夹的文件，播放/删除全部仅限本月，
    // 不跨月）
    for (const folder of store.filteredFolders) {
        for (const file of folder.files) {
            const m = monthOf(systemTimeToDate(file.created_at));
            const f = folderOf(m, folder);
            f.files.push(file);
            f.playlist = f.files;
            f.latestAt = Math.max(f.latestAt, fileTime(file));
            f.totalSize += file.size;
        }
    }

    const out: MonthNode[] = [];
    for (const [key, m] of months) {
        const folders = [...m.folders.values()].map((f) => ({
            ...f,
            files: f.files.sort((a, b) => fileTime(b) - fileTime(a)),
        }));
        const fileCount = folders.reduce((s, f) => s + f.files.length, 0);
        out.push({
            key,
            label: m.label,
            isCurrent: m.isCurrent,
            folders,
            fileCount,
        });
    }
    // 月份：时间降序（未知日期最后）
    out.sort((a, b) => {
        if (a.key === "m:unknown") return 1;
        if (b.key === "m:unknown") return -1;
        return b.key.localeCompare(a.key);
    });
    return out;
});

/** 文件夹排序（未分类恒排最后；latest 按最新文件时间降序） */
function sortFolders(
    folders: FolderNode[],
    mode: FolderSortMode,
): FolderNode[] {
    const arr = [...folders];
    const cmp = (a: FolderNode, b: FolderNode) => {
        if (a.name === UNCATEGORIZED) return 1;
        if (b.name === UNCATEGORIZED) return -1;
        if (mode === "name-asc")
            return a.name.localeCompare(b.name, undefined, { numeric: true });
        if (mode === "name-desc")
            return b.name.localeCompare(a.name, undefined, { numeric: true });
        return b.latestAt - a.latestAt;
    };
    return arr.sort(cmp);
}

// ── 虚拟滚动（手写固定行高；扁平化「月份头 + 文件夹头 + 文件行」单层列表）──
// 行高常量与模板类严格一致：文件行 h-14(56px)、月份头/文件夹头 h-11(44px)。
// 密度缩放（--density-mult）只作用于直播页，不影响本页。
const FILE_ROW_HEIGHT = 56;
const MONTH_HEADER_HEIGHT = 44;
const FOLDER_HEADER_HEIGHT = 44;
/** 视口外预渲染缓冲行数（快速滚动时减少白屏） */
const OVERSCAN = 6;

/** 扁平化条目：月份头 / 文件夹头 / 文件行 */
interface FlatItem {
    kind: "month-header" | "folder-header" | "file";
    /** 全局唯一 key（分组/文件 id 前缀区分，避免跨分组重复） */
    id: string;
    /** 在列表内容中的偏移（px，扁平化时累加行高得出） */
    top: number;
    height: number;
    /** month-header：月份折叠键（折叠状态用）与标题/文件数 */
    monthKey?: string;
    label?: string;
    isCurrentMonth?: boolean;
    count?: number;
    /** folder-header：主播名与汇总 + 播放全部（该主播全部音频） */
    folderName?: string;
    folderSize?: number;
    playlist?: RecordingFile[];
    /** file：文件 */
    file?: RecordingFile;
}

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

/**
 * 扁平化：月份头 → 文件夹头 → 文件行，单层列表。
 * 折叠规则：月份折叠 → 整月隐藏；文件夹折叠 → 其下全部隐藏。
 * 文件夹顺序按月排序控件（monthSorts）决定。
 */
const flatItems = computed<FlatItem[]>(() => {
    const items: FlatItem[] = [];
    let top = 0;
    const push = (item: Omit<FlatItem, "top">) => {
        items.push({ ...item, top });
        top += item.height;
    };
    for (const m of monthTree.value) {
        push({
            kind: "month-header",
            id: m.key,
            height: MONTH_HEADER_HEIGHT,
            monthKey: m.key,
            label: m.label,
            isCurrentMonth: m.isCurrent,
            count: m.fileCount,
        });
        if (isCollapsed(m.key)) continue;
        const folders = sortFolders(m.folders, monthSortOf(m.key));
        for (const f of folders) {
            const fkey = `a:${m.key}:${f.path}`;
            push({
                kind: "folder-header",
                id: fkey,
                height: FOLDER_HEADER_HEIGHT,
                folderName: f.name,
                count: f.files.length,
                folderSize: f.totalSize,
                playlist: f.playlist,
            });
            if (isCollapsed(fkey)) continue;
            for (const file of f.files) {
                push({
                    kind: "file",
                    id: `f:${file.id}`,
                    height: FILE_ROW_HEIGHT,
                    file,
                });
            }
        }
    }
    return items;
});

/** 列表总高度（虚拟列表占位层高度） */
const totalHeight = computed(() => {
    const items = flatItems.value;
    return items.length
        ? items[items.length - 1].top + items[items.length - 1].height
        : 0;
});

// ── 视口内应渲染的条目（含 overscan；纯范围计算见 lib/virtualList.ts）──
const visibleItems = computed<{ item: FlatItem }[]>(() => {
    const { start, end } = computeVisibleRange(
        flatItems.value,
        virtualScrollTop.value,
        virtualViewportH.value,
        OVERSCAN,
    );
    const out: { item: FlatItem }[] = [];
    for (let i = start; i <= end; i++) {
        out.push({ item: flatItems.value[i] });
    }
    return out;
});

/**
 * 条目折叠态：month-header / folder-header 看自身 key
 * （月份折叠时整月不渲染；文件夹折叠时其下文件不渲染，因此
 *   子条目无需再查父级 key。）
 */
function isItemCollapsed(item: FlatItem): boolean {
    return isCollapsed(item.id);
}

/** 条目折叠切换（记录视口锚点，重建后恢复滚动位置，避免内容跳动） */
function toggleItemCollapse(item: FlatItem) {
    const anchor = visibleItems.value[0]?.item;
    pendingAnchorId = anchor?.id ?? null;
    // 锚点在视口内的 Y = 列表内偏移 − 列表相对滚动（折叠前捕获，
    // 重建后按「新偏移 + 偏移量 − 视口Y」反推容器 scrollTop）
    pendingAnchorViewportY = anchor ? anchor.top - virtualScrollTop.value : 0;
    toggleCollapse(item.id);
}

/**
 * 列表相对滚动位置 = 容器 scrollTop − 列表在容器内容中的偏移。
 *
 * 几何推导：设列表在容器内容中（scrollTop=0 时）的偏移为 `off`（搜索栏/
 * 内边距），容器滚动 s 后列表 top 相对容器 top = off − s，因此：
 *     off = (list.top − container.top) + s
 *     列表相对滚动 = s − off = container.top − list.top
 * 旧实现直接取 `(list.top − container.top) + scrollTop`，两项相加恒等于常量
 * `off`——滚动时虚拟化位置不更新，只渲染首屏附近条目，展开内容越多、
 * 下方越空白（「显示上限」）。这里改为 `container.top − list.top` 并钳制 ≥0。
 */
function readVirtualScrollTop(): number {
    const c = contentRef.value;
    const w = listRef.value;
    if (!c || !w) return 0;
    return Math.max(
        0,
        c.getBoundingClientRect().top - w.getBoundingClientRect().top,
    );
}

/** 滚动到列表顶部（筛选变化时保持可见性，避免停留在缩水列表末尾的空视口） */
function scrollListToTop() {
    contentRef.value?.scrollTo({ top: 0 });
}

// 筛选条件变化（去抖落定后）→ 回到顶部
watch(
    [
        () => store.searchQuery,
        () => store.anchorQuery,
        () => store.typeFilter,
        () => store.dateRange.start,
        () => store.dateRange.end,
    ],
    () => {
        scrollListToTop();
    },
);

// 列表变化（数据刷新/折叠切换/筛选落定）→ 锚定恢复 + 布局后越界钳制 +
// 刷新虚拟化范围。钳制用容器 scrollHeight - clientHeight（含上下内边距与
// 搜索栏占位，旧公式 totalHeight - viewportH 漏算这些偏移，内容多时反复
// 展开/折叠会把 scrollTop 越钳越上，表现为「展开后显示不完全/跳动」）。
// 钳制必须在布局落定后执行（折叠后 scrollHeight 才更新为收缩值），
// 故放在 requestAnimationFrame 内；锚定恢复（scrollTop 即时生效）立即执行。
watch(flatItems, () => {
    const c = contentRef.value;
    if (!c) return;
    if (pendingAnchorId) {
        const idx = flatItems.value.findIndex((i) => i.id === pendingAnchorId);
        if (idx >= 0) {
            const w = listRef.value;
            // 列表在容器内容中的偏移 off（此刻布局未变，仍是折叠前的值）
            const offset0 = w
                ? w.getBoundingClientRect().top -
                  c.getBoundingClientRect().top +
                  c.scrollTop
                : 0;
            // 容器 scrollTop = 条目新偏移 + off − 锚点原视口 Y
            c.scrollTop = Math.max(
                0,
                flatItems.value[idx].top + offset0 - pendingAnchorViewportY,
            );
        }
        pendingAnchorId = null;
    }
    requestAnimationFrame(() => {
        const el = contentRef.value;
        if (!el) return;
        const maxListScroll = Math.max(0, el.scrollHeight - el.clientHeight);
        if (el.scrollTop > maxListScroll) {
            el.scrollTop = maxListScroll;
        }
        virtualViewportH.value = el.clientHeight;
        virtualScrollTop.value = readVirtualScrollTop();
    });
});

// 搜索栏显隐会改变列表在容器内的偏移（布局位移，不触发 scroll/Resize），
// 显式刷新虚拟化范围，避免可见区间偏移一个搜索栏高度
watch(searchOpen, () => {
    virtualViewportH.value = contentRef.value?.clientHeight ?? 0;
    virtualScrollTop.value = readVirtualScrollTop();
});

// ── 格式化 ──
function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    const kb = bytes / 1024;
    if (kb < 1024) return `${kb.toFixed(1)} KB`;
    const mb = kb / 1024;
    if (mb < 1024) return `${mb.toFixed(1)} MB`;
    return `${(mb / 1024).toFixed(1)} GB`;
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
    | { kind: "file"; file: RecordingFile }
    | { kind: "folder"; files: RecordingFile[] };

const deleteTarget = ref<DeleteTarget | null>(null);

function requestDeleteFile(file: RecordingFile) {
    // 录制中（FFmpeg 正在写入）：禁止删除，防文件占用错误
    if (file.is_active) return;
    deleteTarget.value = { kind: "file", file };
}

/** 删除文件夹头全部内容：仅本月该文件夹内的文件（录制中的文件跳过不删） */
function requestDeleteFolder(files: RecordingFile[]) {
    const deletable = files.filter((f) => !f.is_active);
    if (deletable.length === 0) return;
    deleteTarget.value = { kind: "folder", files: deletable };
}

const deleteConfirmTitle = computed(() =>
    deleteTarget.value?.kind === "folder"
        ? t("files.deleteFolderConfirmTitle")
        : t("files.deleteConfirmTitle"),
);

const deleteConfirmMessage = computed(() => {
    const target = deleteTarget.value;
    if (!target) return "";
    if (target.kind === "file") return t("files.deleteConfirmMessage");
    return t("files.deleteFolderConfirmMessage", {
        count: target.files.length,
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
                target.files.map((f) => store.deleteFile(f.path)),
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
    const exists = store.allFiles.some(
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

// ── 打开输出目录（M4 修复：接线 open_output_dir 命令；目录不存在时后端自动创建）──
const openingDir = ref(false);

async function handleOpenOutputDir() {
    if (openingDir.value) return;
    openingDir.value = true;
    try {
        await api.openOutputDir();
        notify(t("files.outputDirOpened"));
    } catch (e) {
        notify(t("files.outputDirOpenFailed", { error: String(e) }), "Error");
    } finally {
        openingDir.value = false;
    }
}

// ── 立即清理（M4 修复：接线 run_cleanup_now 命令；删除文件需二次确认）──
const cleaning = ref(false);

async function confirmCleanup() {
    cleanupOpen.value = false;
    if (cleaning.value) return;
    cleaning.value = true;
    try {
        const summary = await api.runCleanupNow();
        if (summary.files_deleted > 0) {
            notify(
                t("files.cleanupSuccess", {
                    count: summary.files_deleted,
                    size: formatSize(summary.bytes_freed),
                }),
            );
        } else {
            notify(t("files.cleanupNothing"));
        }
        // run_cleanup_now 内部已刷新缓存并 emit recording_files_changed；
        // 此处再拉一次缓存（无重扫）保证 UI 与磁盘一致（事件竞态兜底）
        await store.fetchFiles();
    } catch (e) {
        notify(t("files.cleanupFailed", { error: String(e) }), "Error");
    } finally {
        cleaning.value = false;
    }
}

// ── 播放（内置播放器；音频生命周期全局：playerStore 单例 audio 挂 document.body）──
// 页面卸载不影响播放（audio 不随组件销毁）；切回本页时 UI 从 store 恢复
// （onMounted 里 syncState 兜底同步）。「播放全部」= store 队列顺序播放
// 该主播全部音频文件（ended 自动切下一个）。
const player = usePlayerStore();

function isCurrent(file: RecordingFile) {
    return player.currentFile?.id === file.id;
}

/** 单文件 / 分段组整组播放（队列迁入 playerStore） */
function playFiles(files: RecordingFile[]) {
    player.playFiles(files);
}

function seekAudio(value: number[]) {
    const next = value[0] ?? 0;
    if (player.duration) player.seek(next / player.duration);
}

/** 音量滑块双向绑定（Slider 组件，跟随主题；setVolume 同步到 audio） */
const playerVolume = computed({
    get: () => [player.volume],
    set: (v: number[]) => {
        player.setVolume(v[0] ?? 0);
    },
});

// ── 生命周期 ──
onMounted(() => {
    loadCollapsed();
    loadMonthSorts();
    contentRef.value?.addEventListener("scroll", onContentScroll, {
        passive: true,
    });
    // 虚拟滚动初始化：视口高度 + 容器尺寸变化监听
    const c = contentRef.value;
    if (c) {
        virtualViewportH.value = c.clientHeight;
        virtualScrollTop.value = readVirtualScrollTop();
        listResizeObserver = new ResizeObserver(() => {
            if (contentRef.value) {
                virtualViewportH.value = contentRef.value.clientHeight;
            }
        });
        listResizeObserver.observe(c);
    }
    // 页面卸载期间播放继续：挂载时把单例 audio 实时值同步进 store（UI 恢复）
    player.syncState();
    store.startListener();
    void store.fetchFiles().catch((e) => {
        console.error("Failed to load file list", e);
    });
});

onBeforeUnmount(() => {
    contentRef.value?.removeEventListener("scroll", onContentScroll);
    listResizeObserver?.disconnect();
    listResizeObserver = null;
    commitSearchQuery.cancel();
    commitAnchorQuery.cancel();
    store.stopListener();
});
</script>

<template>
    <div class="relative flex h-full min-w-0 flex-1">
        <!-- 左侧竖排操作栏 + 筛选面板（Popover 平替手写浮层：锚定筛选按钮，点击外部/ESC 自动关闭） -->
        <Popover v-model:open="filterOpen">
            <NavRail
                :items="railItems"
                :aria-label="t('nav.fileManager')"
                :show-scroll-top="showScrollTop"
                :scroll-top-label="t('files.backToTop')"
                @scroll-top="scrollToTop"
            >
                <!-- 筛选触发器：PopoverAnchor 提供锚点；展开/收起由按钮手动切换。
                    注意 1：PopoverAnchor 必须放在 <Tooltip> 外层——Tooltip 内部自建
                    PopperRoot，锚点若在 Tooltip 内会注册到 Tooltip 的 PopperRoot，
                    PopoverContent 读到的仍是空锚点，浮层永不定位（错位/不可见）。
                    注意 2：PopoverAnchor 不是 PopoverTrigger，DismissableLayer 不会把
                    它当作 trigger 排除——面板打开时点击按钮，pointerdown 冒泡到
                    document 触发「外部点击关闭」、mousedown 聚焦按钮触发 focusin
                    的「外部焦点关闭」，与按钮 toggle 竞争（闪关闪开，点按钮关不掉）。
                    因此在按钮上阻止 pointerdown / focusin 冒泡，让 toggle 独占。 -->
                <template #filter="{ item }">
                    <PopoverAnchor as-child>
                        <Tooltip>
                            <TooltipTrigger as-child>
                                <Button
                                    :ref="onFilterBtnRef"
                                    size="icon"
                                    variant="ghost"
                                    class="size-10 max-[720px]:size-9"
                                    :class="
                                        filterOpen
                                            ? 'bg-accent text-accent-foreground'
                                            : ''
                                    "
                                    :aria-label="item.label"
                                    :aria-expanded="filterOpen"
                                    aria-haspopup="dialog"
                                    :data-state="filterOpen ? 'open' : 'closed'"
                                    @click="filterOpen = !filterOpen"
                                    @pointerdown.stop
                                    @focusin.stop
                                >
                                    <SlidersHorizontal
                                        class="size-5 max-[720px]:size-4"
                                    />
                                </Button>
                            </TooltipTrigger>
                            <TooltipContent side="right">
                                {{ item.label }}
                            </TooltipContent>
                        </Tooltip>
                    </PopoverAnchor>
                </template>
            </NavRail>

            <PopoverContent
                side="right"
                align="start"
                :side-offset="0"
                class="flex max-h-[45vh] w-64 flex-col gap-5 overflow-y-auto"
                :aria-label="t('files.filterFiles')"
                @escape-key-down="onFilterEscapeKeyDown"
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

            <!-- 按主播名模糊筛选（输入层去抖，280ms 后写入 store） -->
            <div class="flex flex-col gap-2">
                <Label class="text-xs text-muted-foreground">
                    {{ t("files.filterByAnchor") }}
                </Label>
                <Input
                    :model-value="anchorInput"
                    :placeholder="t('files.filterByAnchor')"
                    class="h-8"
                    @update:model-value="onAnchorInput"
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
                            { value: 'all', labelKey: 'files.allTypes' },
                            { value: 'm4a', labelKey: 'files.formatM4A' },
                            { value: 'mp3', labelKey: 'files.formatMP3' },
                        ]"
                        :key="opt.value"
                        class="flex cursor-pointer items-center gap-2 text-sm"
                    >
                        <RadioGroupItem :value="opt.value" class="size-4" />
                        <span>{{ t(opt.labelKey) }}</span>
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
            </PopoverContent>
        </Popover>

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
                <!-- 搜索栏（由左侧搜索按钮控制显示；输入去抖 280ms 后写 store） -->
                <div v-if="searchOpen" class="relative mb-2 flex items-center">
                    <Search
                        class="pointer-events-none absolute left-3 size-4 text-muted-foreground"
                        aria-hidden="true"
                    />
                    <Input
                        :model-value="searchInput"
                        :placeholder="t('files.searchPlaceholder')"
                        class="h-9 pl-9 pr-9"
                        @update:model-value="onSearchInput"
                    />
                    <Button
                        v-if="store.searchQuery"
                        variant="ghost"
                        size="icon-sm"
                        class="absolute right-1.5 rounded-full"
                        :aria-label="t('files.clearSearch')"
                        @click="clearSearch"
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

                <!-- 分组列表：扁平化虚拟滚动（月份头 + 文件夹头 + 文件行；
                     只渲染视口内条目，其余以占位高度撑开滚动） -->
                <div
                    v-else
                    ref="listRef"
                    class="relative"
                    :style="{ height: totalHeight + 'px' }"
                >
                    <template v-for="entry in visibleItems" :key="entry.item.id">
                        <!-- 月份标题：年月 + 文件数 + 本月徽标 + 右侧排序控件 + 折叠 -->
                        <div
                            v-if="entry.item.kind === 'month-header'"
                            role="button"
                            tabindex="0"
                            class="absolute left-0 right-0 flex h-11 cursor-pointer select-none items-center gap-2 px-3 pb-1 pt-5 outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
                            :style="{ top: entry.item.top + 'px' }"
                            :aria-expanded="!isItemCollapsed(entry.item)"
                            @click="toggleItemCollapse(entry.item)"
                            @keydown.enter.prevent="
                                toggleItemCollapse(entry.item)
                            "
                            @keydown.space.prevent="
                                toggleItemCollapse(entry.item)
                            "
                        >
                            <ChevronRight
                                class="size-4 shrink-0 text-muted-foreground transition-transform"
                                :class="
                                    isItemCollapsed(entry.item)
                                        ? ''
                                        : 'rotate-90'
                                "
                                aria-hidden="true"
                            />
                            <span class="text-sm font-semibold">{{
                                entry.item.label
                            }}</span>
                            <Badge
                                as="span"
                                v-if="entry.item.isCurrentMonth"
                                class="bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium leading-tight text-primary"
                            >
                                {{ t("files.thisMonth") }}
                            </Badge>
                            <span class="text-xs text-muted-foreground">
                                {{
                                    t("files.fileCount", {
                                        count: entry.item.count ?? 0,
                                    })
                                }}
                            </span>
                            <!-- 月份内文件夹排序控件（点击不触发折叠） -->
                            <div
                                class="ml-auto flex shrink-0 items-center"
                                @click.stop
                            >
                                <DropdownMenu>
                                    <DropdownMenuTrigger as-child>
                                        <Button
                                            size="icon-sm"
                                            variant="ghost"
                                            class="rounded-full"
                                            :aria-label="t('files.sortBy')"
                                        >
                                            <ArrowUpDown class="size-4" />
                                        </Button>
                                    </DropdownMenuTrigger>
                                    <DropdownMenuContent
                                        align="end"
                                        class="w-40"
                                    >
                                        <DropdownMenuItem
                                            :class="{
                                                'bg-accent text-accent-foreground':
                                                    monthSortOf(
                                                        entry.item.monthKey!,
                                                    ) === 'latest',
                                            }"
                                            @select="
                                                setMonthSort(
                                                    entry.item.monthKey!,
                                                    'latest',
                                                )
                                            "
                                        >
                                            <ListMusic class="size-4" />
                                            {{ t("files.sortLatest") }}
                                        </DropdownMenuItem>
                                        <DropdownMenuItem
                                            :class="{
                                                'bg-accent text-accent-foreground':
                                                    monthSortOf(
                                                        entry.item.monthKey!,
                                                    ) === 'name-asc',
                                            }"
                                            @select="
                                                setMonthSort(
                                                    entry.item.monthKey!,
                                                    'name-asc',
                                                )
                                            "
                                        >
                                            <ArrowUpDown class="size-4" />
                                            {{ t("files.sortNameAsc") }}
                                        </DropdownMenuItem>
                                        <DropdownMenuItem
                                            :class="{
                                                'bg-accent text-accent-foreground':
                                                    monthSortOf(
                                                        entry.item.monthKey!,
                                                    ) === 'name-desc',
                                            }"
                                            @select="
                                                setMonthSort(
                                                    entry.item.monthKey!,
                                                    'name-desc',
                                                )
                                            "
                                        >
                                            <ArrowUpDown class="size-4 rotate-180" />
                                            {{ t("files.sortNameDesc") }}
                                        </DropdownMenuItem>
                                    </DropdownMenuContent>
                                </DropdownMenu>
                            </div>
                        </div>

                        <!-- 文件夹标题（主播）：主播名 + 文件数 + 大小 + 播放全部 + 折叠 -->
                        <div
                            v-else-if="entry.item.kind === 'folder-header'"
                            role="button"
                            tabindex="0"
                            class="absolute left-0 right-0 flex h-11 cursor-pointer select-none items-center gap-2 rounded-lg bg-muted/40 px-3 py-2.5 outline-none transition hover:bg-accent/60 focus-visible:bg-accent/60 focus-visible:ring-2 focus-visible:ring-ring/50"
                            :style="{ top: entry.item.top + 'px' }"
                            :aria-expanded="!isItemCollapsed(entry.item)"
                            @click="toggleItemCollapse(entry.item)"
                            @keydown.enter.prevent="
                                toggleItemCollapse(entry.item)
                            "
                            @keydown.space.prevent="
                                toggleItemCollapse(entry.item)
                            "
                        >
                            <ChevronRight
                                class="size-4 shrink-0 text-muted-foreground transition-transform"
                                :class="
                                    isItemCollapsed(entry.item)
                                        ? ''
                                        : 'rotate-90'
                                "
                                aria-hidden="true"
                            />
                            <Folder
                                class="size-4 shrink-0 text-muted-foreground"
                                aria-hidden="true"
                            />
                            <span
                                class="min-w-0 truncate text-sm font-semibold"
                                :title="
                                    entry.item.folderName === UNCATEGORIZED
                                        ? t('files.uncategorized')
                                        : entry.item.folderName
                                "
                            >
                                {{
                                    entry.item.folderName === UNCATEGORIZED
                                        ? t("files.uncategorized")
                                        : entry.item.folderName
                                }}
                            </span>
                            <span class="shrink-0 text-xs text-muted-foreground">
                                {{
                                    t("files.fileCount", {
                                        count: entry.item.count ?? 0,
                                    })
                                }}
                            </span>
                            <span
                                class="ml-auto shrink-0 text-xs tabular-nums text-muted-foreground"
                            >
                                {{ formatSize(entry.item.folderSize ?? 0) }}
                            </span>
                            <!-- 更多菜单（播放全部 = 本月该主播全部音频；
                                 删除全部 = 仅删除本月该文件夹内容；点击不触发折叠） -->
                            <div class="shrink-0" @click.stop>
                                <DropdownMenu>
                                    <DropdownMenuTrigger as-child>
                                        <Button
                                            size="icon-sm"
                                            variant="ghost"
                                            class="rounded-full"
                                            :aria-label="
                                                t('files.moreActions')
                                            "
                                            :disabled="
                                                !entry.item.playlist ||
                                                entry.item.playlist.length === 0
                                            "
                                        >
                                            <MoreVertical class="size-4" />
                                        </Button>
                                    </DropdownMenuTrigger>
                                    <DropdownMenuContent
                                        align="end"
                                        class="w-40"
                                    >
                                        <DropdownMenuItem
                                            @select="
                                                playFiles(
                                                    entry.item.playlist ?? [],
                                                )
                                            "
                                        >
                                            <Play />
                                            {{ t("files.playAll") }}
                                        </DropdownMenuItem>
                                        <DropdownMenuSeparator />
                                        <DropdownMenuItem
                                            variant="destructive"
                                            :disabled="
                                                !entry.item.playlist ||
                                                entry.item.playlist.length === 0
                                            "
                                            @select="
                                                requestDeleteFolder(
                                                    entry.item.playlist ?? [],
                                                )
                                            "
                                        >
                                            <Trash2 />
                                            {{ t("files.deleteAll") }}
                                        </DropdownMenuItem>
                                    </DropdownMenuContent>
                                </DropdownMenu>
                            </div>
                        </div>

                        <!-- 音频文件条目（主播文件夹下） -->
                        <div
                            v-else-if="
                                entry.item.kind === 'file' &&
                                entry.item.file
                            "
                            role="button"
                            tabindex="0"
                            :title="entry.item.file.name"
                            class="group absolute left-0 right-0 flex h-14 cursor-pointer select-none items-center gap-3 rounded-lg px-3 py-2.5 outline-none transition hover:bg-accent/60 focus-visible:bg-accent/60 focus-visible:ring-2 focus-visible:ring-ring/50"
                            :style="{ top: entry.item.top + 'px' }"
                            @dblclick="playFiles([entry.item.file])"
                            @keydown.enter.prevent="playFiles([entry.item.file])"
                            @keydown.delete.prevent="
                                requestDeleteFile(entry.item.file)
                            "
                            @keydown.f2.prevent="startRename(entry.item.file)"
                        >
                            <div
                                class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary"
                            >
                                <AudioLines
                                    v-if="isCurrent(entry.item.file)"
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
                                        {{ entry.item.file.name }}
                                    </span>
                                    <!-- 录制中标记：正被 FFmpeg 写入，禁删/禁重命名
                                        颜色：以 --primary 为种子色派生（跟随主题强调色），
                                        与「本月」徽标（primary/10）通过透明度分层保持差异
                                        （录制中 15% 底更深 + 实心脉冲点，状态更突出） -->
                                    <Badge
                                        as="span"
                                        v-if="entry.item.file.is_active"
                                        class="bg-primary/15 px-1.5 py-0.5 text-[10px] font-medium leading-tight text-primary"
                                    >
                                        <span
                                            class="size-1.5 animate-pulse rounded-full bg-primary"
                                            aria-hidden="true"
                                        />
                                        {{ t("files.recordingActive") }}
                                    </Badge>
                                </p>
                            </div>
                            <span
                                class="shrink-0 text-xs tabular-nums text-muted-foreground"
                            >
                                {{
                                    formatDateShort(
                                        systemTimeToDate(
                                            entry.item.file.created_at,
                                        ),
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
                                            @select="playFiles([entry.item.file])"
                                        >
                                            <Play />
                                            {{ t("files.play") }}
                                        </DropdownMenuItem>
                                        <DropdownMenuItem
                                            :disabled="
                                                entry.item.file.is_active
                                            "
                                            @select="
                                                startRename(entry.item.file)
                                            "
                                        >
                                            <Pencil />
                                            {{ t("files.rename") }}
                                        </DropdownMenuItem>
                                        <DropdownMenuSeparator />
                                        <DropdownMenuItem
                                            variant="destructive"
                                            :disabled="
                                                entry.item.file.is_active
                                            "
                                            @select="
                                                requestDeleteFile(
                                                    entry.item.file,
                                                )
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
                </div>
            </template>
        </section>

        <!-- 删除确认（文件 / 文件夹本月内容） -->
        <ConfirmDialog
            :open="!!deleteTarget"
            :title="deleteConfirmTitle"
            :message="deleteConfirmMessage"
            destructive
            @confirm="confirmDelete"
            @cancel="deleteTarget = null"
        />

        <!-- 立即清理确认（run_cleanup_now：按保留天数/总量上限删除旧文件，不可恢复） -->
        <ConfirmDialog
            :open="cleanupOpen"
            :title="t('files.cleanupConfirmTitle')"
            :message="t('files.cleanupConfirmMessage')"
            :confirm-text="t('files.cleanupNow')"
            destructive
            @confirm="confirmCleanup"
            @cancel="cleanupOpen = false"
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

        <!-- 内置播放器（底部浮条；多文件队列播放时显示进度 x/y）。
             UI 在文件页，但音频生命周期全局（playerStore 单例 audio 挂
             document.body）：切页不停止播放，切回时 UI 从 store 恢复。 -->
        <Card
            v-if="player.currentFile"
            class="fixed bottom-4 left-1/2 z-50 flex w-[min(560px,calc(100vw-2rem))] -translate-x-1/2 flex-row items-center gap-3 rounded-xl border-border/70 bg-background/95 p-3 shadow-lg backdrop-blur"
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
                    <Badge
                        v-if="player.isQueuePlay"
                        class="shrink-0 border-transparent bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-primary"
                    >
                        {{
                            t("files.playerProgress", {
                                current: player.queueIndex + 1,
                                total: player.queue.length,
                            })
                        }}
                    </Badge>
                </div>
                <div class="mt-1 flex items-center gap-2">
                    <span
                        class="w-9 shrink-0 text-right text-[10px] tabular-nums text-muted-foreground"
                    >
                        {{ formatTime(player.currentTime) }}
                    </span>
                    <Slider
                        :model-value="[player.currentTime]"
                        :min="0"
                        :max="player.duration || 0"
                        :step="0.1"
                        class="h-1.5 flex-1"
                        :aria-label="t('files.playerSeek')"
                        @update:model-value="(v: number[] | undefined) => seekAudio(v ?? [])"
                    />
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
                <Slider
                    v-model="playerVolume"
                    :min="0"
                    :max="1"
                    :step="0.01"
                    :aria-label="t('files.playerVolume')"
                    class="w-14"
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
        </Card>
    </div>
</template>
