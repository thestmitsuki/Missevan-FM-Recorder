/**
 * 去抖工具（零依赖手写实现，供搜索/筛选输入等高频触发场景使用）。
 *
 * 语义与 lodash debounce 核心一致：在 `delay` 毫秒内连续调用只会执行最后一次；
 * `cancel()` 取消未执行的调用；`flush()` 立即执行挂起的调用。
 */

export interface Debounced<A extends unknown[]> {
  (...args: A): void;
  /** 取消挂起中的调用（组件卸载 / 重置时调用，防止卸载后写 store） */
  cancel(): void;
  /** 立即执行挂起的调用并清空计时器 */
  flush(): void;
}

/**
 * 创建去抖函数：`delay` 毫秒内的连续调用合并为最后一次执行。
 *
 * @param fn    实际执行的函数
 * @param delay 延迟毫秒数
 * @example
 *   const commit = debounce((v: string) => store.query = v, 280);
 *   watch(input, (v) => commit(v));
 *   onBeforeUnmount(() => commit.cancel());
 */
export function debounce<A extends unknown[]>(
  fn: (...args: A) => void,
  delay: number,
): Debounced<A> {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let lastArgs: A | null = null;

  function debounced(this: unknown, ...args: A): void {
    lastArgs = args;
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      if (lastArgs) fn.apply(this, lastArgs);
      lastArgs = null;
    }, delay);
  }

  debounced.cancel = () => {
    if (timer !== null) clearTimeout(timer);
    timer = null;
    lastArgs = null;
  };

  debounced.flush = () => {
    if (timer !== null) clearTimeout(timer);
    timer = null;
    if (lastArgs) {
      const args = lastArgs;
      lastArgs = null;
      fn(...args);
    }
  };

  return debounced;
}
