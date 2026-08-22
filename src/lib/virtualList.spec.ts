/**
 * computeVisibleRange 单元测试：空列表 / 视口边界 / overscan 钳制 / 二分定位正确性。
 *
 * 构造固定行高或变高条目（top 严格递增），覆盖：
 * - 空列表与非法视口高度
 * - 首/中/尾滚动位置的首末可见下标
 * - 条目底边恰与 scrollTop 重合的边界（应视作视口外）
 * - overscan 在列表两端钳制到 [0, len-1]
 */
import { describe, it, expect } from "vitest";
import { computeVisibleRange, type VirtualSpan } from "./virtualList";

/** 变高条目：top 严格递增，height 可变（模拟不同长度文件名换行等场景） */
const ITEMS: VirtualSpan[] = [
  { top: 0, height: 10 },
  { top: 10, height: 20 },
  { top: 30, height: 10 },
  { top: 40, height: 30 },
  { top: 70, height: 10 },
];

describe("computeVisibleRange", () => {
  it("空列表返回空区间 { start: 0, end: -1 }", () => {
    expect(computeVisibleRange([], 0, 100)).toEqual({ start: 0, end: -1 });
    expect(computeVisibleRange([], 12345, 100)).toEqual({ start: 0, end: -1 });
  });

  it("视口高度 <= 0 时返回空区间", () => {
    expect(computeVisibleRange(ITEMS, 0, 0)).toEqual({ start: 0, end: -1 });
    expect(computeVisibleRange(ITEMS, 0, -10)).toEqual({ start: 0, end: -1 });
  });

  it("overscan 为 0 时首屏仅渲染视口内条目", () => {
    // 视口 [0, 50)：top < 50 的条目为下标 0..3
    expect(computeVisibleRange(ITEMS, 0, 50, 0)).toEqual({ start: 0, end: 3 });
  });

  it("中部滚动位置：二分跳过已滚出条目，只渲染视口内 + overscan", () => {
    // scrollTop=25：下标 0（底 10）、1（底 30）已滚出；视口 [25, 45)：可见 1..3
    expect(computeVisibleRange(ITEMS, 25, 20, 0)).toEqual({ start: 1, end: 3 });
  });

  it("条目底边恰与 scrollTop 重合时视为已滚出（边界严格性）", () => {
    // 下标 1 底边 = 30 == scrollTop：不进入视口，首个可见为下标 2
    expect(computeVisibleRange(ITEMS, 30, 20, 0)).toEqual({ start: 2, end: 3 });
  });

  it("滚到底部：仅最后一条可见，start 被 overscan 回退", () => {
    // scrollTop=70：仅下标 4（底 80 > 70）可见；overscan 2 → [2, 4]
    expect(computeVisibleRange(ITEMS, 70, 50, 2)).toEqual({ start: 2, end: 4 });
  });

  it("overscan 在列表头部钳制为 0（不越界为负）", () => {
    const r = computeVisibleRange(ITEMS, 0, 50, 6);
    expect(r.start).toBe(0);
    expect(r.end).toBeGreaterThanOrEqual(3);
  });

  it("overscan 在列表尾部钳制为 len-1（不越界）", () => {
    const r = computeVisibleRange(ITEMS, 9999, 50, 100);
    expect(r.end).toBe(ITEMS.length - 1);
    expect(r.start).toBeGreaterThanOrEqual(0);
  });

  it("overscan 为负视为 0（防御）", () => {
    // 视口 [0,50)：可见 0..3，负 overscan 不扩张
    expect(computeVisibleRange(ITEMS, 0, 50, -1)).toEqual({ start: 0, end: 3 });
  });

  it("overscan 覆盖整个列表时返回全量区间", () => {
    expect(computeVisibleRange(ITEMS, 0, 50, 100)).toEqual({
      start: 0,
      end: 4,
    });
  });

  it("单条目列表各滚动位置均命中该条目", () => {
    const single: VirtualSpan[] = [{ top: 0, height: 40 }];
    expect(computeVisibleRange(single, 0, 20)).toEqual({ start: 0, end: 0 });
    expect(computeVisibleRange(single, 39, 20)).toEqual({ start: 0, end: 0 });
  });

  it("高视口 + 任意 overscan 时区间不越界（二分与扫描的正确性）", () => {
    // 构造 1000 条等高的长列表做压力验证
    const many: VirtualSpan[] = Array.from({ length: 1000 }, (_, i) => ({
      top: i * 24,
      height: 24,
    }));
    const r = computeVisibleRange(many, 500 * 24, 600, 8);
    // 视口内条目：[500*24, 500*24+600) → 下标 500..524，加 overscan 8
    expect(r.start).toBe(500 - 8);
    expect(r.end).toBe(524 + 8);
  });
});
