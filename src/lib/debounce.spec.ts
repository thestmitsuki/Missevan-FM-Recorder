/**
 * debounce 单元测试：去抖时序（延迟触发 / 连续调用合并 / cancel / flush / 参数透传）。
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { debounce } from "./debounce";

describe("debounce", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("延迟到期前不执行，到期后执行一次", () => {
    const fn = vi.fn();
    const d = debounce(fn, 100);
    d("a");
    expect(fn).not.toHaveBeenCalled();

    vi.advanceTimersByTime(99);
    expect(fn).not.toHaveBeenCalled();

    vi.advanceTimersByTime(1);
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("连续调用合并：只执行最后一次，且参数为最后一次的", () => {
    const fn = vi.fn();
    const d = debounce(fn, 100);
    d(1);
    vi.advanceTimersByTime(50);
    d(2);
    vi.advanceTimersByTime(50);
    d(3);
    vi.advanceTimersByTime(100);

    expect(fn).toHaveBeenCalledTimes(1);
    expect(fn).toHaveBeenCalledWith(3);
  });

  it("多参数透传", () => {
    const fn = vi.fn();
    const d = debounce(fn, 50);
    d("k", 42, { x: true });
    vi.advanceTimersByTime(50);
    expect(fn).toHaveBeenCalledWith("k", 42, { x: true });
  });

  it("cancel 取消挂起调用：到期后不再执行", () => {
    const fn = vi.fn();
    const d = debounce(fn, 100);
    d("a");
    d.cancel();
    vi.advanceTimersByTime(200);
    expect(fn).not.toHaveBeenCalled();
  });

  it("cancel 后再次调用可重新调度", () => {
    const fn = vi.fn();
    const d = debounce(fn, 100);
    d("a");
    d.cancel();
    d("b");
    vi.advanceTimersByTime(100);
    expect(fn).toHaveBeenCalledTimes(1);
    expect(fn).toHaveBeenCalledWith("b");
  });

  it("flush 立即执行挂起调用并清空计时器", () => {
    const fn = vi.fn();
    const d = debounce(fn, 100);
    d("a");
    d.flush();
    expect(fn).toHaveBeenCalledTimes(1);
    expect(fn).toHaveBeenCalledWith("a");
    // flush 后计时器已清空，继续推进不会重复执行
    vi.advanceTimersByTime(200);
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("flush 后再次调用会重新调度（新周期）", () => {
    const fn = vi.fn();
    const d = debounce(fn, 100);
    d("a");
    d.flush();
    d("b");
    vi.advanceTimersByTime(100);
    expect(fn).toHaveBeenCalledTimes(2);
    expect(fn).toHaveBeenLastCalledWith("b");
  });

  it("无挂起调用时 flush/cancel 为无害操作", () => {
    const fn = vi.fn();
    const d = debounce(fn, 100);
    expect(() => d.flush()).not.toThrow();
    expect(() => d.cancel()).not.toThrow();
    expect(fn).not.toHaveBeenCalled();
  });

  it("执行后计时器复位：可再次触发新一轮去抖", () => {
    const fn = vi.fn();
    const d = debounce(fn, 100);
    d("a");
    vi.advanceTimersByTime(100);
    d("b");
    vi.advanceTimersByTime(100);
    expect(fn).toHaveBeenCalledTimes(2);
    expect(fn).toHaveBeenLastCalledWith("b");
  });
});
