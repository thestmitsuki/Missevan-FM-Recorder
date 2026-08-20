/**
 * 固定行高虚拟列表：视口范围计算（零依赖手写实现）。
 *
 * 适用于「扁平化条目数组 + 固定行高 + 绝对定位占位」的手写虚拟滚动：
 * 只渲染视口内（含 overscan 缓冲）的条目，DOM 数量与数据总量解耦。
 */

/** 虚拟列表条目所需的最小子集（top 为条目在列表内容中的偏移，height 为行高） */
export interface VirtualSpan {
  top: number;
  height: number;
}

export interface VirtualRange {
  /** 首个可见条目下标（含 overscan，已钳制到 [0, len-1]） */
  start: number;
  /** 末个可见条目下标（含 overscan，已钳制到 [0, len-1]） */
  end: number;
}

/**
 * 计算当前滚动位置下应渲染的条目下标区间（闭区间，含 overscan）。
 *
 * 前置条件：`items` 按 top 严格递增（由扁平化时逐项累加行高保证），
 * 因此可用二分查找定位首条可见条目；后续用顺序扫描找末条（视口内条目
 * 通常远小于总条目数）。`items` 为空时返回 { start: 0, end: -1 }。
 */
export function computeVisibleRange<T extends VirtualSpan>(
  items: readonly T[],
  scrollTop: number,
  viewportHeight: number,
  overscan = 6,
): VirtualRange {
  const len = items.length;
  if (len === 0 || viewportHeight <= 0) {
    return { start: 0, end: -1 };
  }
  // 防御：overscan 钳制到非负（负值会使 start 越过二分结果，破坏区间正确性）
  overscan = Math.max(0, overscan);

  // 二分：首个 top + height > scrollTop 的条目
  let lo = 0;
  let hi = len;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (items[mid].top + items[mid].height <= scrollTop) {
      lo = mid + 1;
    } else {
      hi = mid;
    }
  }

  const start = Math.max(0, lo - overscan);
  const bottom = scrollTop + viewportHeight;
  let end = start;
  while (end < len && items[end].top < bottom) {
    end++;
  }
  // end 此时为视口外第一条（或 len），回退 1 得到最后可见下标，再加 overscan
  end = Math.min(len - 1, end - 1 + overscan);
  return { start, end };
}
