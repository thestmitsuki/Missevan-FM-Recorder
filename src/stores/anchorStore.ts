import { defineStore } from "pinia";
import { computed, ref, watch } from "vue";
import type { AnchorConfig, AnchorStatusUpdate, RecordingStatus } from "@/types";
import { api } from "@/services/api";
import { onRecordingStatusChanged } from "@/services/events";

/** 录制状态筛选 */
export type RecordFilter = "all" | "recording" | "not-recording";
/** 直播状态筛选 */
export type LiveFilter = "all" | "live" | "not-live";
/** 视图模式 */
export type ViewMode = "card" | "list";

/** 筛选条件（与 localStorage 持久化结构一致） */
export interface LiveFilters {
  searchQuery: string;
  /** 标签筛选（多选：固定 5 标签子集；空数组 = 全部，不过滤） */
  tagFilter: string[];
  recordFilter: RecordFilter;
  liveFilter: LiveFilter;
}

const VIEW_MODE_KEY = "live_view_mode";
const FILTERS_KEY = "live_filters";

function loadViewMode(): ViewMode {
  try {
    return localStorage.getItem(VIEW_MODE_KEY) === "list" ? "list" : "card";
  } catch {
    return "card";
  }
}

/**
 * 标签筛选迁移（兼容 localStorage 历史数据）：
 * - 当前版本为 string[]（多选，空数组 = 全部）；
 * - 中间版本为 string | null（单选），迁移为单元素数组 / 空数组；
 * - 更早版本为 string[]（含「无标签」哨兵 "__none__"），剔除哨兵与空串后保留。
 */
function loadTagFilter(raw: unknown): string[] {
  if (Array.isArray(raw)) {
    return raw.filter(
      (t): t is string =>
        typeof t === "string" && t !== "" && t !== "__none__",
    );
  }
  if (typeof raw === "string") return raw ? [raw] : [];
  return [];
}

function loadFilters(): LiveFilters {
  const fallback: LiveFilters = {
    searchQuery: "",
    tagFilter: [],
    recordFilter: "all",
    liveFilter: "all",
  };
  try {
    const raw = localStorage.getItem(FILTERS_KEY);
    if (!raw) return fallback;
    const parsed = JSON.parse(raw) as Partial<LiveFilters>;
    return {
      searchQuery:
        typeof parsed.searchQuery === "string" ? parsed.searchQuery : "",
      tagFilter: loadTagFilter(parsed.tagFilter),
      recordFilter:
        parsed.recordFilter === "recording" ||
        parsed.recordFilter === "not-recording"
          ? parsed.recordFilter
          : "all",
      liveFilter:
        parsed.liveFilter === "live" || parsed.liveFilter === "not-live"
          ? parsed.liveFilter
          : "all",
    };
  } catch {
    return fallback;
  }
}

// 事件监听取消函数（模块级单例，与 notificationStore 一致）
let unlisten: (() => void) | null = null;

export const useAnchorStore = defineStore("anchor", () => {
  // ── 数据 ──
  const anchors = ref<AnchorConfig[]>([]);
  const recordingStatuses = ref<RecordingStatus[]>([]);
  // 初始为 true：直播页首帧显示 Skeleton，避免加载完成前闪现空态
  const loading = ref(true);
  const error = ref<unknown>(null);

  /**
   * 状态起始时间（规格「时长前端计数」）：anchor_id -> 首次获知该状态的 epoch ms。
   * 后端不提供直播/录制的起始时间戳，故以「首次获知状态」为计数起点，
   * 之后由前端 setInterval 递增；状态消失时删除。
   */
  const liveSince = ref<Record<string, number>>({});
  const recordingSince = ref<Record<string, number>>({});

  // ── 视图模式与筛选（localStorage 持久化，loadFilters 只执行一次）──
  const initialFilters = loadFilters();
  const viewMode = ref<ViewMode>(loadViewMode());
  const searchQuery = ref(initialFilters.searchQuery);
  const tagFilter = ref<string[]>(initialFilters.tagFilter);
  const recordFilter = ref<RecordFilter>(initialFilters.recordFilter);
  const liveFilter = ref<LiveFilter>(initialFilters.liveFilter);

  watch(viewMode, (v) => {
    try {
      localStorage.setItem(VIEW_MODE_KEY, v);
    } catch {
      /* localStorage 不可用时忽略 */
    }
  });

  watch(
    [searchQuery, tagFilter, recordFilter, liveFilter],
    () => {
      const filters: LiveFilters = {
        searchQuery: searchQuery.value,
        tagFilter: tagFilter.value,
        recordFilter: recordFilter.value,
        liveFilter: liveFilter.value,
      };
      try {
        localStorage.setItem(FILTERS_KEY, JSON.stringify(filters));
      } catch {
        /* 忽略 */
      }
    },
    { deep: true },
  );

  // ── 派生数据 ──
  const liveAnchors = computed(
    () =>
      recordingStatuses.value
        .filter((s) => s.is_recording)
        .map((s) => anchors.value.find((a) => a.id === s.anchor_id))
        .filter(Boolean) as AnchorConfig[],
  );

  const recordingCount = computed(
    () => recordingStatuses.value.filter((s) => s.is_recording).length,
  );

  /** anchor_id -> RecordingStatus 快速查找 */
  const statusMap = computed<Record<string, RecordingStatus>>(() => {
    const map: Record<string, RecordingStatus> = {};
    for (const s of recordingStatuses.value) map[s.anchor_id] = s;
    return map;
  });

  /**
   * 筛选后的主播列表（实时生效）：
   * - 名称模糊匹配（不区分大小写）
   * - 标签多选：勾选多个标签时，命中任一勾选标签的主播通过（OR）；
   *   未勾选（空数组）不过滤，显示全部
   * - 录制/直播状态单选
   */
  const filteredAnchors = computed(() => {
    const q = searchQuery.value.trim().toLowerCase();
    const tags = tagFilter.value;
    const record = recordFilter.value;
    const live = liveFilter.value;
    return anchors.value.filter((a) => {
      if (q && !(a.name ?? "").toLowerCase().includes(q)) return false;
      if (tags.length > 0 && !tags.some((t) => (a.tags ?? []).includes(t))) {
        return false;
      }
      const st = statusMap.value[a.id];
      if (record === "recording" && !st?.is_recording) return false;
      if (record === "not-recording" && st?.is_recording) return false;
      if (live === "live" && !st?.is_live) return false;
      if (live === "not-live" && st?.is_live) return false;
      return true;
    });
  });

  // ── 动作 ──
  async function fetchAnchors() {
    loading.value = true;
    error.value = null;
    try {
      const list = await api.getAnchors();
      // tags 由后端落盘持久化（Task A/3）；`?? []` 仅兜底旧版后端响应
      anchors.value = list.map((a) => ({ ...a, tags: a.tags ?? [] }));
    } catch (e) {
      error.value = e;
    } finally {
      loading.value = false;
    }
  }

  /**
   * 全量状态回拉后对齐时长起点表：
   * - 已处于直播/录制且尚无起点 → 以「当前」为起点（后端不提供起始时间戳）
   * - 状态已消失 → 清理起点
   */
  function syncSinceFromStatuses() {
    const liveIds = new Set(
      recordingStatuses.value
        .filter((s) => s.is_live)
        .map((s) => s.anchor_id),
    );
    const recordingIds = new Set(
      recordingStatuses.value
        .filter((s) => s.is_recording)
        .map((s) => s.anchor_id),
    );
    for (const s of recordingStatuses.value) {
      if (s.is_live && liveSince.value[s.anchor_id] === undefined) {
        liveSince.value[s.anchor_id] = Date.now();
      }
      if (s.is_recording && recordingSince.value[s.anchor_id] === undefined) {
        recordingSince.value[s.anchor_id] = Date.now();
      }
    }
    for (const id of Object.keys(liveSince.value)) {
      if (!liveIds.has(id)) delete liveSince.value[id];
    }
    for (const id of Object.keys(recordingSince.value)) {
      if (!recordingIds.has(id)) delete recordingSince.value[id];
    }
  }

  async function fetchRecordingStatuses() {
    error.value = null;
    try {
      recordingStatuses.value = await api.getRecordingStatus();
      syncSinceFromStatuses();
    } catch (e) {
      error.value = e;
    }
  }

  async function addAnchor(anchor: AnchorConfig) {
    // tags 由后端持久化落盘（Task A/3），回拉即真实值，无需重新套用
    await api.addAnchor(anchor);
    await fetchAnchors();
  }

  async function removeAnchor(id: string) {
    await api.removeAnchor(id);
    await fetchAnchors();
    // 清理已删除主播的残留状态与时长起点
    recordingStatuses.value = recordingStatuses.value.filter(
      (s) => s.anchor_id !== id,
    );
    delete liveSince.value[id];
    delete recordingSince.value[id];
  }

  async function updateAnchor(anchorId: string, updated: AnchorConfig) {
    // tags 由后端持久化落盘（Task A/3），回拉即真实值，无需重新套用
    await api.updateAnchor(anchorId, updated);
    await fetchAnchors();
    await fetchRecordingStatuses();
  }

  /** 立即从猫耳 API 刷新主播信息（名称/头像；后端返回体含 tags 与最新 avatar_url） */
  async function refreshAnchor(anchorId: string) {
    const fresh = await api.refreshAnchor(anchorId);
    const idx = anchors.value.findIndex((a) => a.id === anchorId);
    if (idx !== -1) {
      anchors.value[idx] = fresh;
    }
  }

  async function stopRecording(anchorId: string) {
    // 走 stop_anchors_recording（M4 修复：该命令原本注册后无前端调用方；
    // 含 pre_record_delay 延迟窗口内的启动取消；stop_recording 由调试页使用）
    await api.stopAnchorsRecording(anchorId);
    await fetchRecordingStatuses();
  }

  function isAnchorRecording(anchorId: string): boolean {
    return statusMap.value[anchorId]?.is_recording ?? false;
  }

  /**
   * 事件推送更新：只更新对应主播的单条状态，不重建列表（规格「状态实时更新」）。
   * 同时维护时长起点：状态由 off -> on 记起点，on -> off 清起点。
   */
  function updateStatusFromEvent(update: AnchorStatusUpdate) {
    const prev = statusMap.value[update.anchor_id];
    const prevLive = prev?.is_live ?? false;
    const prevRecording = prev?.is_recording ?? false;

    if (update.is_live && !prevLive) {
      liveSince.value[update.anchor_id] = Date.now();
    } else if (!update.is_live && prevLive) {
      delete liveSince.value[update.anchor_id];
    }
    if (update.is_recording && !prevRecording) {
      recordingSince.value[update.anchor_id] = Date.now();
    } else if (!update.is_recording && prevRecording) {
      delete recordingSince.value[update.anchor_id];
    }

    const idx = recordingStatuses.value.findIndex(
      (s) => s.anchor_id === update.anchor_id,
    );
    if (idx !== -1) {
      recordingStatuses.value[idx] = { ...update };
    } else {
      recordingStatuses.value.push({ ...update });
    }
  }

  /** 直播时长起点（epoch ms）；未直播返回 undefined */
  function liveSinceOf(anchorId: string): number | undefined {
    return liveSince.value[anchorId];
  }

  /** 录制时长起点（epoch ms）；未录制返回 undefined */
  function recordingSinceOf(anchorId: string): number | undefined {
    return recordingSince.value[anchorId];
  }

  function clearFilters() {
    searchQuery.value = "";
    tagFilter.value = [];
    recordFilter.value = "all";
    liveFilter.value = "all";
  }

  // ── 事件订阅（统一走 events.ts 层，页面内不直接 listen）──
  function startListening() {
    if (unlisten) return;
    unlisten = onRecordingStatusChanged((update) =>
      updateStatusFromEvent(update),
    );
  }

  function stopListening() {
    unlisten?.();
    unlisten = null;
  }

  return {
    anchors,
    recordingStatuses,
    loading,
    error,
    viewMode,
    searchQuery,
    tagFilter,
    recordFilter,
    liveFilter,
    liveAnchors,
    recordingCount,
    statusMap,
    filteredAnchors,
    fetchAnchors,
    fetchRecordingStatuses,
    addAnchor,
    removeAnchor,
    updateAnchor,
    refreshAnchor,
    stopRecording,
    isAnchorRecording,
    liveSinceOf,
    recordingSinceOf,
    updateStatusFromEvent,
    clearFilters,
    startListening,
    stopListening,
  };
});
