/**
 * fileStore 单元测试：文件夹树（folders）下的搜索/筛选与空文件夹剔除。
 *
 * 覆盖点：
 * - 搜索：文件名 + 主播名模糊匹配（不区分大小写）
 * - 筛选面板：主播名 / 文件类型（扩展名）/ 日期范围（起止，含与搜索叠加）
 * - filteredFolders：文件夹内文件过滤 + 空文件夹剔除
 * - clearFilters / hasActiveFilters / allFiles
 *
 * 说明：fileStore 无排序逻辑（排序由视图层负责），故不测排序；
 * fetch 动作依赖 tauri IPC，未触发，无需 mock。
 */
import { describe, it, expect, beforeEach } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useFileStore } from "./fileStore";
import { systemTimeToDate } from "@/types/file";
import type { FileFolder, RecordingFile } from "@/types";

/** 由 epoch 秒构造本地时区 YYYY-MM-DD（与 store 内 dateIsoOf 同路径，测试跨时区稳定） */
function isoOf(secs: number): string {
  const d = systemTimeToDate({ secs_since_epoch: secs, nanos_since_epoch: 0 });
  const mm = String(d.getMonth() + 1).padStart(2, "0");
  const dd = String(d.getDate()).padStart(2, "0");
  return `${d.getFullYear()}-${mm}-${dd}`;
}

/** 三个不同月份的文件（UTC 零点，本地日期随时区偏移但相对关系不变） */
const F1: RecordingFile = {
  id: "f1",
  name: "2024-01-01_主播A_第1节.m4a",
  path: "/rec/主播A-1/a.m4a",
  size: 100,
  duration: 10,
  anchor_name: "主播A",
  created_at: { secs_since_epoch: 1704067200, nanos_since_epoch: 0 },
};
const F2: RecordingFile = {
  id: "f2",
  name: "2024-02-01_主播B_第1节.mp3",
  path: "/rec/主播B-2/b.mp3",
  size: 200,
  duration: 20,
  anchor_name: "主播B",
  created_at: { secs_since_epoch: 1706745600, nanos_since_epoch: 0 },
};
const F3: RecordingFile = {
  id: "f3",
  name: "2024-03-01_主播A_第2节.mp3",
  path: "/rec/主播A-1/c.mp3",
  size: 300,
  duration: 30,
  anchor_name: "主播A",
  created_at: { secs_since_epoch: 1709251200, nanos_since_epoch: 0 },
};

const ISO1 = isoOf(F1.created_at.secs_since_epoch);
const ISO2 = isoOf(F2.created_at.secs_since_epoch);
const ISO3 = isoOf(F3.created_at.secs_since_epoch);

/** 两个主播文件夹（F1/F3 同文件夹、F2 独立文件夹） */
function seedFolders(): FileFolder[] {
  return [
    {
      name: "主播A",
      path: "/rec/主播A-1",
      files: [F1, F3],
    },
    {
      name: "主播B",
      path: "/rec/主播B-2",
      files: [F2],
    },
  ];
}

/** 全部命中的单文件 id（按文件夹顺序展开） */
function fileIds(folders: FileFolder[]): string[] {
  return folders.flatMap((f) => f.files.map((x) => x.id));
}

function seededStore() {
  setActivePinia(createPinia());
  const store = useFileStore();
  store.folders = seedFolders();
  return store;
}

describe("fileStore · filteredFolders 搜索与筛选", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("无筛选时返回全部文件夹/文件", () => {
    const store = seededStore();
    expect(fileIds(store.filteredFolders)).toEqual(["f1", "f3", "f2"]);
    expect(store.hasAnyFiles).toBe(true);
  });

  it("搜索命中文件名（不区分大小写）", () => {
    const store = seededStore();
    store.searchQuery = "M4A"; // 命中 F1 文件名扩展名
    expect(fileIds(store.filteredFolders)).toEqual(["f1"]);
    store.searchQuery = "第2节";
    expect(fileIds(store.filteredFolders)).toEqual(["f3"]);
  });

  it("搜索命中主播名", () => {
    const store = seededStore();
    store.searchQuery = "主播A";
    expect(fileIds(store.filteredFolders)).toEqual(["f1", "f3"]);
  });

  it("筛选面板主播名（anchorQuery）与搜索叠加", () => {
    const store = seededStore();
    store.anchorQuery = "主播B";
    expect(fileIds(store.filteredFolders)).toEqual(["f2"]);
    store.searchQuery = "主播A"; // 搜索命中 A，主播筛选为 B → 无交集
    expect(store.filteredFolders).toEqual([]);
  });

  it("文件类型筛选：扩展名小写匹配", () => {
    const store = seededStore();
    store.typeFilter = "m4a";
    expect(fileIds(store.filteredFolders)).toEqual(["f1"]);
    store.typeFilter = "mp3";
    // 文件夹主序：主播A 文件夹（F3）在前，主播B 文件夹（F2）在后
    expect(fileIds(store.filteredFolders)).toEqual(["f3", "f2"]);
  });

  it("日期范围：start 过滤早于起点的文件", () => {
    const store = seededStore();
    store.dateRange = { start: ISO2, end: "" };
    // 文件夹主序：主播A 文件夹（F3）在前，主播B 文件夹（F2）在后
    expect(fileIds(store.filteredFolders)).toEqual(["f3", "f2"]);
  });

  it("日期范围：起点为最晚文件日期时仅保留该文件", () => {
    const store = seededStore();
    store.dateRange = { start: ISO3, end: "" };
    expect(fileIds(store.filteredFolders)).toEqual(["f3"]);
  });

  it("日期范围：end 过滤晚于终点的文件", () => {
    const store = seededStore();
    store.dateRange = { start: "", end: ISO1 };
    expect(fileIds(store.filteredFolders)).toEqual(["f1"]);
  });

  it("日期范围：起止闭区间", () => {
    const store = seededStore();
    store.dateRange = { start: ISO1, end: ISO2 };
    expect(fileIds(store.filteredFolders)).toEqual(["f1", "f2"]);
  });

  it("搜索 + 类型 + 日期三条件叠加", () => {
    const store = seededStore();
    store.searchQuery = "主播A";
    store.typeFilter = "mp3";
    store.dateRange = { start: ISO2, end: "" };
    expect(fileIds(store.filteredFolders)).toEqual(["f3"]);
  });

  it("hasActiveFilters：任一条件生效即为 true，clearFilters 后为 false", () => {
    const store = seededStore();
    expect(store.hasActiveFilters).toBe(false);
    store.searchQuery = "  "; // 纯空白不算生效
    expect(store.hasActiveFilters).toBe(false);
    store.typeFilter = "mp3";
    expect(store.hasActiveFilters).toBe(true);
    store.dateRange = { start: ISO1, end: "" };
    expect(store.hasActiveFilters).toBe(true);
    store.clearFilters();
    expect(store.hasActiveFilters).toBe(false);
    expect(fileIds(store.filteredFolders)).toHaveLength(3);
  });
});

describe("fileStore · 空文件夹剔除与 allFiles", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("全部文件被过滤 → 文件夹整体剔除", () => {
    setActivePinia(createPinia());
    const store = useFileStore();
    store.folders = seedFolders();
    store.searchQuery = "主播B";
    expect(store.filteredFolders).toHaveLength(1);
    expect(store.filteredFolders[0].path).toBe("/rec/主播B-2");
    expect(store.filteredFolders[0].files.map((f) => f.id)).toEqual(["f2"]);
    expect(store.hasFilteredFiles).toBe(true);

    store.searchQuery = "不存在的关键词";
    expect(store.filteredFolders).toEqual([]);
    expect(store.hasFilteredFiles).toBe(false);
  });

  it("allFiles 展开文件夹内全部文件", () => {
    setActivePinia(createPinia());
    const store = useFileStore();
    store.folders = seedFolders();
    expect(store.allFiles.map((f) => f.id)).toEqual(["f1", "f3", "f2"]);
  });
});
