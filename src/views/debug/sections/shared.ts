/**
 * 调试页 section 共享工具：时间/大小/时长格式化、状态码与日志级别着色。
 */
import { onBeforeUnmount, onMounted } from "vue";
import { i18n } from "@/locales";

/** 格式化 RFC3339 时间为 HH:mm:ss（随当前 locale；解析失败回退原串） */
export function formatTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  try {
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return iso;
    return new Intl.DateTimeFormat(i18n.global.locale.value, {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      hour12: false,
    }).format(d);
  } catch {
    return iso;
  }
}

/** 字节数人性化显示 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = bytes;
  let i = -1;
  do {
    v /= 1024;
    i++;
  } while (v >= 1024 && i < units.length - 1);
  return `${v.toFixed(1)} ${units[i]}`;
}

/** 秒数 → "1h 2m 3s" / "2m 3s" / "3s" */
export function formatDuration(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = s % 60;
  if (h > 0) return `${h}h ${m}m ${r}s`;
  if (m > 0) return `${m}m ${r}s`;
  return `${r}s`;
}

/** 网络表格行背景色（200 绿 / 429 黄 / 5xx 与 0 红 / 其余中性） */
export function statusRowClass(status: number): string {
  if (status >= 200 && status < 300) return "bg-emerald-500/5";
  if (status === 429) return "bg-amber-500/10";
  if (status >= 500 || status === 0) return "bg-destructive/10";
  return "";
}

/** 日志级别文字颜色 */
export function levelClass(level: string): string {
  switch (level) {
    case "error":
      return "text-destructive";
    case "warn":
      return "text-amber-500";
    case "info":
      return "text-emerald-500";
    case "debug":
      return "text-sky-500";
    default:
      return "text-muted-foreground";
  }
}

/** 日志级别过滤 chip 选中态 */
export function levelActiveClass(level: string): string {
  switch (level) {
    case "error":
      return "border-destructive bg-destructive/15 text-destructive";
    case "warn":
      return "border-amber-500 bg-amber-500/15 text-amber-600 dark:text-amber-400";
    case "info":
      return "border-emerald-500 bg-emerald-500/15 text-emerald-600 dark:text-emerald-400";
    case "debug":
      return "border-sky-500 bg-sky-500/15 text-sky-600 dark:text-sky-400";
    default:
      return "border-muted-foreground/50 bg-muted text-muted-foreground";
  }
}

/**
 * 轮询组合式函数：挂载后立即执行一次，之后每 intervalMs 执行一次；
 * enabled() 返回 false 时跳过本轮（日志/网络模块的暂停）。
 * 卸载时自动清理定时器。
 */
export function usePolling(
  fn: () => void | Promise<void>,
  intervalMs = 2000,
  enabled: () => boolean = () => true,
) {
  let timer: ReturnType<typeof setInterval> | null = null;
  onMounted(() => {
    void fn();
    timer = setInterval(() => {
      if (enabled()) void fn();
    }, intervalMs);
  });
  onBeforeUnmount(() => {
    if (timer !== null) {
      clearInterval(timer);
      timer = null;
    }
  });
}
