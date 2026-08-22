/**
 * anchorStore 单元测试：筛选逻辑与 localStorage 历史数据迁移。
 *
 * 覆盖点：
 * - loadTagFilter 三种历史形态迁移（string[] 多选 / string|null 单选 / legacy __none__ 数组）
 * - filteredAnchors：标签多选 OR 语义、搜索/录制/直播筛选叠加、无状态主播边界
 * - clearFilters 重置全部筛选
 * - 筛选持久化（watch → localStorage）
 *
 * 说明：被测逻辑为纯前端筛选与迁移，不调用 api（fetch 动作未触发），
 * 故无需 mock tauri IPC；store 通过 createPinia/setActivePinia 独立实例化。
 */
import { describe, it, expect, beforeEach } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { flushPromises } from "@vue/test-utils";
import { useAnchorStore } from "./anchorStore";
import type { AnchorConfig, RecordingStatus } from "@/types";

/** 构造主播（仅测试所需字段） */
function anchor(
  id: string,
  name: string,
  tags: string[] = [],
): AnchorConfig {
  return {
    id,
    name,
    url: `https://www.missevan.com/${id}`,
    room_id: id,
    enable_check: true,
    tags,
  };
}

function status(id: string, is_recording: boolean, is_live: boolean): RecordingStatus {
  return { anchor_id: id, is_recording, is_live };
}

/** 预置 localStorage 中的 live_filters 后创建全新 store 实例 */
function storeWithStored(raw: unknown) {
  localStorage.setItem("live_filters", JSON.stringify(raw));
  setActivePinia(createPinia());
  return useAnchorStore();
}

describe("anchorStore · 标签筛选历史形态迁移（loadTagFilter）", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("当前版本 string[]：剔除空串，保留多选", () => {
    const store = storeWithStored({
      tagFilter: ["vocal", "dance", ""],
    });
    expect(store.tagFilter).toEqual(["vocal", "dance"]);
  });

  it("中间版本 string：单选迁移为单元素数组；空串迁移为空数组", () => {
    expect(storeWithStored({ tagFilter: "vocal" }).tagFilter).toEqual(["vocal"]);
    expect(storeWithStored({ tagFilter: "" }).tagFilter).toEqual([]);
  });

  it("中间版本 null：迁移为空数组（不过滤）", () => {
    expect(storeWithStored({ tagFilter: null }).tagFilter).toEqual([]);
  });

  it("早期版本含 __none__ 哨兵：剔除哨兵与空串后保留真实标签", () => {
    expect(storeWithStored({ tagFilter: ["__none__", "vocal", ""] }).tagFilter).toEqual([
      "vocal",
    ]);
  });

  it("非数组/非字符串垃圾数据：回退为空数组", () => {
    expect(storeWithStored({ tagFilter: 123 }).tagFilter).toEqual([]);
    expect(storeWithStored({ tagFilter: {} }).tagFilter).toEqual([]);
  });

  it("localStorage 缺失或 JSON 损坏：使用默认空筛选", () => {
    // 无 localStorage 数据
    setActivePinia(createPinia());
    const s1 = useAnchorStore();
    expect(s1.tagFilter).toEqual([]);
    expect(s1.searchQuery).toBe("");
    expect(s1.recordFilter).toBe("all");

    // 损坏 JSON
    localStorage.setItem("live_filters", "{not json");
    setActivePinia(createPinia());
    const s2 = useAnchorStore();
    expect(s2.tagFilter).toEqual([]);
    expect(s2.recordFilter).toBe("all");
  });
});

describe("anchorStore · filteredAnchors 筛选叠加", () => {
  beforeEach(() => {
    localStorage.clear();
    setActivePinia(createPinia());
  });

  function seededStore() {
    const store = useAnchorStore();
    store.anchors = [
      anchor("1", "Alice", ["vocal", "dance"]),
      anchor("2", "Bob", ["talk"]),
      anchor("3", "Carol", []),
      anchor("4", "Dave", ["vocal"]),
    ];
    store.recordingStatuses = [
      status("1", true, true), // Alice：录制中 + 直播中
      status("2", false, false), // Bob：均未开启
      // Carol / Dave：无状态记录（等价于均未开启）
    ];
    return store;
  }

  it("无筛选时返回全部主播", () => {
    const store = seededStore();
    expect(store.filteredAnchors.map((a) => a.id)).toEqual(["1", "2", "3", "4"]);
  });

  it("搜索：名称模糊匹配且不区分大小写", () => {
    const store = seededStore();
    store.searchQuery = "al";
    expect(store.filteredAnchors.map((a) => a.id)).toEqual(["1"]);
    store.searchQuery = "CAROL";
    expect(store.filteredAnchors.map((a) => a.id)).toEqual(["3"]);
    store.searchQuery = "  bob  "; // 首尾空白剔除
    expect(store.filteredAnchors.map((a) => a.id)).toEqual(["2"]);
  });

  it("标签多选 OR 语义：命中任一勾选标签即通过", () => {
    const store = seededStore();
    store.tagFilter = ["vocal"];
    expect(store.filteredAnchors.map((a) => a.id)).toEqual(["1", "4"]);
    store.tagFilter = ["vocal", "talk"];
    expect(store.filteredAnchors.map((a) => a.id)).toEqual(["1", "2", "4"]);
  });

  it("标签为空数组 = 不过滤", () => {
    const store = seededStore();
    store.tagFilter = [];
    expect(store.filteredAnchors).toHaveLength(4);
  });

  it("录制筛选 recording：仅录制中主播", () => {
    const store = seededStore();
    store.recordFilter = "recording";
    expect(store.filteredAnchors.map((a) => a.id)).toEqual(["1"]);
  });

  it("录制筛选 not-recording：非录制中（含无状态）主播", () => {
    const store = seededStore();
    store.recordFilter = "not-recording";
    expect(store.filteredAnchors.map((a) => a.id)).toEqual(["2", "3", "4"]);
  });

  it("直播筛选 live / not-live", () => {
    const store = seededStore();
    store.liveFilter = "live";
    expect(store.filteredAnchors.map((a) => a.id)).toEqual(["1"]);
    store.liveFilter = "not-live";
    expect(store.filteredAnchors.map((a) => a.id)).toEqual(["2", "3", "4"]);
  });

  it("搜索 + 标签 + 录制三条件叠加", () => {
    const store = seededStore();
    store.searchQuery = "a"; // Alice / Carol 命中
    store.tagFilter = ["vocal"]; // Alice / Dave 命中 → 交集 Alice
    store.recordFilter = "recording"; // 仅 Alice
    expect(store.filteredAnchors.map((a) => a.id)).toEqual(["1"]);
  });

  it("条件冲突时无结果（空数组）", () => {
    const store = seededStore();
    store.searchQuery = "bob";
    store.recordFilter = "recording"; // Bob 未录制
    expect(store.filteredAnchors).toEqual([]);
  });
});

describe("anchorStore · clearFilters 与持久化", () => {
  beforeEach(() => {
    localStorage.clear();
    setActivePinia(createPinia());
  });

  it("clearFilters 重置搜索/标签/录制/直播筛选", () => {
    const store = useAnchorStore();
    store.searchQuery = "x";
    store.tagFilter = ["vocal"];
    store.recordFilter = "recording";
    store.liveFilter = "live";
    store.clearFilters();
    expect(store.searchQuery).toBe("");
    expect(store.tagFilter).toEqual([]);
    expect(store.recordFilter).toBe("all");
    expect(store.liveFilter).toBe("all");
  });

  it("筛选变更写入 localStorage（watch 持久化）", async () => {
    const store = useAnchorStore();
    store.tagFilter = ["vocal"];
    await flushPromises();
    const saved = JSON.parse(localStorage.getItem("live_filters") ?? "{}");
    expect(saved.tagFilter).toEqual(["vocal"]);
  });

  it("clearFilters 后持久化为默认值", async () => {
    const store = useAnchorStore();
    store.searchQuery = "x";
    store.tagFilter = ["vocal"];
    store.recordFilter = "recording";
    await flushPromises();
    store.clearFilters();
    await flushPromises();
    const saved = JSON.parse(localStorage.getItem("live_filters") ?? "{}");
    expect(saved).toEqual({
      searchQuery: "",
      tagFilter: [],
      recordFilter: "all",
      liveFilter: "all",
    });
  });
});
