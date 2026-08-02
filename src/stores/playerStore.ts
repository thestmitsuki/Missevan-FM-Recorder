import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
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
 * - 分段组连续播放 = queue 顺序播放，ended 自动切下一段（仍在全局 audio 上）。
 *
 * 加载失败提示（修复「切到文件页误报音频加载失败」根因）：
 * - 旧实现把 `<audio :src="audioUrl">` 放在组件内，页面每次挂载时 src 为空串，
 *   Chromium 对空 src 触发 error 事件 → 误报「音频加载失败」（音频实际正常）。
 * - 本 store 从不绑定空 src（无 src 的 Audio 实例不会触发 error 事件），
 *   @error 仅在**真实错误**（文件缺失/被移动/asset scope 拦截，src 非空且
 *   有播放意图）时提示；加载中、成功、空 src 均不提示。
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
  /** 是否分段组连续播放（队列 > 1） */
  const isGroupPlay = computed(() => queue.value.length > 1);

  // ── audio 单例（跨页面存活；首次播放时惰性创建）──
  let audio: HTMLAudioElement | null = null;

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
      // 分段组连续播放：顺序切下一段；单文件播放结束则停止
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
      // 真实错误：文件缺失 / 被移动 / asset scope 拦截 → 停止并提示
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
    el.src = convertFileSrc(file.path);
    el.volume = volume.value;
    playing.value = false;
    try {
      await el.play();
      playing.value = true;
    } catch {
      // AbortError（换源/停止竞态）静默；其余失败（文件缺失/格式不支持）
      // 由 @error 统一提示，避免重复 toast
      playing.value = false;
    }
  }

  function togglePlay() {
    const el = audio;
    if (!el || !currentFile.value) return;
    if (el.paused) {
      void playCurrent();
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
    isGroupPlay,
    playFiles,
    togglePlay,
    seek,
    setVolume,
    stopPlayback,
    syncState,
  };
});
