import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { RecordingFile } from "@/types";
import { useNotificationStore } from "@/stores/notificationStore";
import { i18n } from "@/locales";

/**
 * 全局播放器 store（Task：播放器组件跨页面不注销）
 *
 * 音频生命周期全局化：
 * - audio 元素为**单例**，首次播放时创建并挂到 document.body——页面切换
 *   （FilesView 卸载）不影响播放；切回文件页时 UI 从 store 恢复。
 * - 队列/播放态/进度/音量全部收敛于此，FilesView 播放条只消费 store。
 * - 多文件队列播放（「播放全部」）= queue 顺序播放，ended 自动切下一个
 *   （仍在全局 audio 上）。
 *
 * 音源（H6，全平台统一）：后端启一个仅 127.0.0.1 回环 + 随机端口 + token 的 HTTP
 * 流式服务，前端 `play_http` 拿到带 token 的 URL 设给 `<audio>`。WebKitGTK 与
 * Chromium 都原生支持 127.0.0.1 http（含 Range/206 seek），不整文件进内存
 * （边下边播），也不暴露绝对路径。替换此前的 blob URL / convertFileSrc(asset://)
 * 方案（Linux 上 asset:// 不被 GStreamer 认、blob 整文件进内存卡顿）。
 *
 * 加载失败提示（修复「切到文件页误报音频加载失败」根因）：
 * - 旧实现把 `<audio :src="audioUrl">` 放在组件内，页面每次挂载时 src 为空串，
 *   Chromium 对空 src 触发 error 事件 → 误报「音频加载失败」（音频实际正常）。
 * - 本 store 从不绑定空 src（无 src 的 Audio 实例不会触发 error 事件），
 *   @error 仅在**真实错误**（文件缺失/服务未启动/URL 失效，src 非空且有播放意图）
 *   时提示；加载中、成功、空 src 均不提示。
 */
export const usePlayerStore = defineStore("player", () => {
  // ── 状态（UI 直接消费）──
  const queue = ref<RecordingFile[]>([]);
  const queueIndex = ref(0);
  const playing = ref(false);
  const currentTime = ref(0);
  const duration = ref(0);
  const volume = ref(1);

  const currentFile = computed(
    () => queue.value[queueIndex.value] ?? null,
  );
  /** 是否多文件队列播放（队列 > 1；「播放全部」时显示进度 x/y） */
  const isQueuePlay = computed(() => queue.value.length > 1);

  // ── audio 单例（跨页面存活；首次播放时惰性创建）──
  let audio: HTMLAudioElement | null = null;

  // ── HTTP 流式音源生命周期管理（H6）──
  // 播放意图代际：play_http 为异步，期间切歌/停止后旧结果必须丢弃（防覆盖新 src）。
  // 换源/停止时调用后端 stop_http 撤销 token，令旧 URL 立即 404（内存/安全双释放）。
  let playbackSeq = 0;

  function ensureAudio(): HTMLAudioElement {
    if (audio) return audio;
    audio = new Audio();
    audio.className = "hidden";
    document.body.appendChild(audio);
    attachAudioListeners(audio);
    return audio;
  }

  function attachAudioListeners(el: HTMLAudioElement) {
    el.addEventListener("timeupdate", () => {
      currentTime.value = el.currentTime;
    });
    el.addEventListener("loadedmetadata", () => {
      if (isFinite(el.duration)) duration.value = el.duration;
    });
    el.addEventListener("play", () => {
      playing.value = true;
    });
    el.addEventListener("pause", () => {
      playing.value = false;
    });
    el.addEventListener("ended", () => {
      // 多文件队列播放：顺序切下一个；单文件播放结束则停止
      if (queueIndex.value < queue.value.length - 1) {
        queueIndex.value += 1;
        void playCurrent();
      } else {
        stopPlayback();
      }
    });
    el.addEventListener("error", () => {
      // 误报防护 1：空 src（页面挂载/停止后未设置源）→ 不视为失败、不提示
      if (!el.currentSrc && !el.getAttribute("src")) return;
      // 误报防护 2：无播放意图的残余 error 事件（同一次失败的重复触发）→ 忽略
      if (queue.value.length === 0) return;
      // 真实错误：文件缺失 / 被移动 / URL 失效 → 停止并提示
      notifyLoadError();
    });
  }

  /** 换源/停止时撤销旧 HTTP URL（切歌/停止后旧文件不再可访问、立即 404） */
  async function releaseHttp() {
    try {
      await invoke("stop_http");
    } catch {
      /* 服务不可用等降级场景忽略 */
    }
  }

  /** 加载失败统一提示（@error 事件与 play_http 失败共用；停止并清空队列） */
  function notifyLoadError() {
    stopPlayback();
    useNotificationStore().addNotification({
      id: `player-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      code: "player-error",
      level: "Error",
      title: i18n.global.t("files.playerError"),
      message: i18n.global.t("files.playerError"),
      suggestion: null,
      source: "files",
      timestamp: new Date().toISOString(),
      actionable: false,
    });
  }

  // ── 播放控制 ──
  function playFiles(files: RecordingFile[]) {
    if (!files.length) return;
    queue.value = files;
    queueIndex.value = 0;
    void playCurrent();
  }

  async function playCurrent() {
    const file = currentFile.value;
    if (!file) return;
    const el = ensureAudio();
    const seq = ++playbackSeq;
    el.volume = volume.value;
    playing.value = false;
    try {
      // 向 HTTP 服务注册当前文件，拿带 token 的流式 URL
      const url = await invoke<string>("play_http", { path: file.path });
      // 竞态防护：等待期间已切歌/停止（seq 过期）→ 丢弃本次结果（并撤销其 token）
      if (seq !== playbackSeq) {
        void releaseHttp();
        return;
      }
      el.src = url;
      await el.play();
      playing.value = true;
    } catch {
      playing.value = false;
      // play_http 失败（文件缺失/服务未启动）：src 未设置，@error 不会触发 → 主动提示
      if (seq === playbackSeq && !el.src) {
        notifyLoadError();
      }
    }
  }

  function togglePlay() {
    const el = audio;
    if (!el || !currentFile.value) return;
    if (el.paused) {
      // 恢复播放：不重新设置 src —— 重新赋值 src 会重置播放位置到 0
      // （表现为暂停后点播放进度被清空）。队列/进度仍在 audio 上，直接 play。
      el.volume = volume.value;
      void el.play().catch(() => {
        playing.value = false;
      });
    } else {
      el.pause();
    }
  }

  /** 按比例跳转（0~1） */
  function seek(ratio: number) {
    const el = audio;
    if (!el || !duration.value) return;
    el.currentTime = Math.min(1, Math.max(0, ratio)) * duration.value;
  }

  function setVolume(v: number) {
    volume.value = v;
    if (audio) audio.volume = v;
  }

  /** 停止并清空队列（用户点击关闭按钮时调用） */
  function stopPlayback() {
    // 使在途 play_http 结果过期 + 撤销旧 URL（服务端 404）
    playbackSeq += 1;
    void releaseHttp();
    if (audio) {
      audio.pause();
      audio.removeAttribute("src"); // removeAttribute 不触发 load，无空 src 误报
    }
    queue.value = [];
    queueIndex.value = 0;
    playing.value = false;
    currentTime.value = 0;
    duration.value = 0;
  }

  /**
   * UI 恢复：页面挂载时把单例 audio 的实时值同步进 store
   * （页面卸载期间播放继续，timeupdate 已持续更新；此调用兜底精确同步）。
   */
  function syncState() {
    const el = audio;
    if (!el) return;
    currentTime.value = el.currentTime;
    if (isFinite(el.duration)) duration.value = el.duration;
    volume.value = el.volume;
    playing.value = !el.paused;
  }

  return {
    queue,
    queueIndex,
    playing,
    currentTime,
    duration,
    volume,
    currentFile,
    isQueuePlay,
    playFiles,
    togglePlay,
    seek,
    setVolume,
    stopPlayback,
    syncState,
  };
});