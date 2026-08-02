import { defineStore } from "pinia";
import { computed, ref } from "vue";
import type { RecordingFile, FileGroup } from "@/types";
import { systemTimeToDate } from "@/types/file";
import { onRecordingFilesChanged } from "@/services/events";
import { api } from "@/services/api";

/** 文件类型筛选（按扩展名，小写） */
export type FileTypeFilter = "all" | "m4a" | "mp3";

/** 日期范围筛选：起止日期，格式 YYYY-MM-DD（空字符串 = 不限） */
export interface DateRangeFilter {
  start: string;
  end: string;
}

/** 取文件名扩展名（小写，无点） */
function extOf(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot >= 0 ? name.slice(dot + 1).toLowerCase() : "";
}

/** 文件日期（本地时区）转为 YYYY-MM-DD 字符串，与 input type=date 对齐 */
function dateIsoOf(file: RecordingFile): string {
  const d = systemTimeToDate(file.created_at);
  const mm = String(d.getMonth() + 1).padStart(2, "0");
  const dd = String(d.getDate()).padStart(2, "0");
  return `${d.getFullYear()}-${mm}-${dd}`;
}

export const useFileStore = defineStore("file", () => {
  // ── 数据 ──
  const files = ref<RecordingFile[]>([]);
  const groups = ref<FileGroup[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);
  let unlisten: (() => void) | null = null;

  // ── 搜索与筛选状态（事件更新后保留；搜索+筛选叠加生效）──
  /** 搜索框关键词：匹配文件名 + 主播名（不区分大小写） */
  const searchQuery = ref("");
  /** 筛选面板：主播名模糊匹配（与搜索叠加） */
  const anchorQuery = ref("");
  /** 筛选面板：文件类型（扩展名） */
  const typeFilter = ref<FileTypeFilter>("all");
  /** 筛选面板：日期范围（起止 YYYY-MM-DD） */
  const dateRange = ref<DateRangeFilter>({ start: "", end: "" });

  /** 单个文件是否命中当前搜索 + 筛选条件 */
  function matchesFilter(file: RecordingFile): boolean {
    const q = searchQuery.value.trim().toLowerCase();
    if (
      q &&
      !file.name.toLowerCase().includes(q) &&
      !file.anchor_name.toLowerCase().includes(q)
    ) {
      return false;
    }
    const aq = anchorQuery.value.trim().toLowerCase();
    if (aq && !file.anchor_name.toLowerCase().includes(aq)) return false;
    if (typeFilter.value !== "all" && extOf(file.name) !== typeFilter.value) {
      return false;
    }
    const { start, end } = dateRange.value;
    if (start || end) {
      const iso = dateIsoOf(file);
      if (start && iso < start) return false;
      if (end && iso > end) return false;
    }
    return true;
  }

  /** 过滤后的单文件（日期分组展示用） */
  const filteredFiles = computed(() => files.value.filter(matchesFilter));

  /** 过滤后的分段组：组内文件过滤，空组剔除 */
  const filteredGroups = computed(() => {
    const result: FileGroup[] = [];
    for (const g of groups.value) {
      const inner = g.files.filter(matchesFilter);
      if (inner.length > 0) {
        result.push({ ...g, files: inner });
      }
    }
    return result;
  });

  /** 是否处于"无任何文件"（未过滤）状态 */
  const hasAnyFiles = computed(
    () => files.value.length > 0 || groups.value.length > 0,
  );

  /** 是否处于"过滤后无结果"状态 */
  const hasFilteredFiles = computed(
    () => filteredFiles.value.length > 0 || filteredGroups.value.length > 0,
  );

  /** 是否有生效的筛选条件（用于"清除筛选"按钮显隐） */
  const hasActiveFilters = computed(
    () =>
      searchQuery.value.trim() !== "" ||
      anchorQuery.value.trim() !== "" ||
      typeFilter.value !== "all" ||
      dateRange.value.start !== "" ||
      dateRange.value.end !== "",
  );

  function clearFilters() {
    searchQuery.value = "";
    anchorQuery.value = "";
    typeFilter.value = "all";
    dateRange.value = { start: "", end: "" };
  }

  // ── 数据操作（统一经 api.ts 封装，页面/store 不直接 invoke） ──
  async function fetchFiles(search?: string) {
    loading.value = true;
    error.value = null;
    try {
      const result = await api.getRecordingFiles(search);
      files.value = result.files;
      groups.value = result.groups;
    } catch (err) {
      error.value = String(err);
    } finally {
      loading.value = false;
    }
  }

  /** 手动触发后端重新扫描录制目录（随后经事件自动更新，无需再次 fetch） */
  async function refreshFiles() {
    await api.refreshRecordingFiles();
  }

  async function renameFile(oldPath: string, newName: string) {
    await api.renameRecordingFile(oldPath, newName);
  }

  async function deleteFile(path: string) {
    await api.deleteRecordingFile(path);
  }

  /** 获取播放 URL（play_recording_file 返回 file:// URL，供外部播放器使用） */
  async function getPlayUrl(path: string): Promise<string> {
    return await api.playRecordingFile(path);
  }

  // ── 事件监听（收敛于 events.ts，此处仅订阅转发）──
  function startListener() {
    if (unlisten) return;
    unlisten = onRecordingFilesChanged((payload) => {
      files.value = payload.files;
      groups.value = payload.groups;
    });
  }

  function stopListener() {
    unlisten?.();
    unlisten = null;
  }

  return {
    files,
    groups,
    loading,
    error,
    searchQuery,
    anchorQuery,
    typeFilter,
    dateRange,
    filteredFiles,
    filteredGroups,
    hasAnyFiles,
    hasFilteredFiles,
    hasActiveFilters,
    clearFilters,
    fetchFiles,
    refreshFiles,
    renameFile,
    deleteFile,
    getPlayUrl,
    startListener,
    stopListener,
  };
});
