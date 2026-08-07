import { defineStore } from "pinia";
import { ref, computed } from "vue";
import type { GlobalConfig } from "@/types";
import { api } from "@/services/api";

/** 与后端 GlobalConfig::default() 对齐（前端无配置时的兜底值；Task 3 已核对） */
export const DEFAULT_CONFIG: GlobalConfig = {
  // 与后端默认 "./recordings" 一致：设置页「恢复默认值」不得产生非法空路径
  output_dir: "./recordings",
  record_format: "m4a",
  segment_seconds: 0,
  disk_space_limit_gb: 10,
  ffmpeg_path: null,
  anchor_ids: [],
  check_interval_secs: 120, // 规格 §7.8：检测间隔默认 120s（Task 14 与后端统一）
  max_retries: 3,
  retry_delay_secs: 5,
  autostart: false,
  close_behavior: "tray",
  show_tray: true,
  check_updates: true,
  bitrate_kbps: 128,
  audio_only: true,
  filename_template: "{anchor_name}/{date}_{time}_{anchor_name}_{index}.{ext}",
  max_concurrent_recordings: 3,
  pre_record_delay_secs: 0,
  post_record_action: "none",
  post_record_command: "",
  auto_cleanup_enabled: false,
  retention_days: 30,
  max_total_gb: 0,
  cleanup_time: "03:00",
  proxy_type: "none",
  proxy_addr: "",
  proxy_port: 0,
  proxy_auth: false,
  proxy_username: "",
  proxy_password: "",
  api_timeout_secs: 10,
  stream_timeout_secs: 30,
  custom_dns: "",
  notifications_enabled: true,
  notify_recording_start: true,
  notify_recording_end: true,
  notify_recording_error: true,
  notify_live_start: true,
  notify_live_end: true,
  notify_disk_warning: true,
  notify_update: true,
  notify_system: true,
  notify_sound: true,
  log_level: "info",
  detector_concurrency: 5,
  ffprobe_path: "",
  // 与后端默认一致：检测随机抖动上限 60s（0 = 不抖动）
  detector_jitter_secs: 60,
  // 快捷键映射：由 ShortcutSection 经 set_shortcut 命令落盘，表单默认空
  shortcuts: {},
  wizard_completed: true,
};

export const useConfigStore = defineStore("config", () => {
  const config = ref<GlobalConfig>({ ...DEFAULT_CONFIG });
  const loading = ref(false);
  const dirty = ref(false);

  const hasOutputDir = computed(() => config.value.output_dir.length > 0);

  const segmentMinutes = computed(() => {
    if (!config.value.segment_seconds) return null;
    return Math.round(config.value.segment_seconds / 60);
  });

  async function fetchConfig() {
    loading.value = true;
    try {
      config.value = await api.getConfig();
      dirty.value = false;
    } finally {
      loading.value = false;
    }
  }

  async function saveConfig() {
    await api.saveConfig(config.value);
    dirty.value = false;
  }

  function updateConfig(patch: Partial<GlobalConfig>) {
    config.value = { ...config.value, ...patch };
    dirty.value = true;
  }

  async function pickOutputDir() {
    const dir = await api.pickOutputDir();
    if (dir) {
      config.value.output_dir = dir;
      dirty.value = true;
    }
    return dir;
  }

  function resetConfig() {
    config.value = { ...DEFAULT_CONFIG };
    dirty.value = true;
  }

  return {
    config,
    loading,
    dirty,
    hasOutputDir,
    segmentMinutes,
    fetchConfig,
    saveConfig,
    updateConfig,
    pickOutputDir,
    resetConfig,
  };
});
