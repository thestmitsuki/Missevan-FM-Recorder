/**
 * Mock 模拟直播 store（调试页 Mock 面板）
 *
 * 职责：
 * - 持有 Mock 模式开关（权威来源，调试页检测循环面板的 "Mock" 标签据此显示）
 *   与模拟主播列表；
 * - 封装 Mock 后端命令（set_mock_mode / list_mock_anchors / add / update / remove /
 *   set_all_mock_live / reset_mock / get_mock_state）；
 * - 订阅 `mock:status_changed` 事件自动刷新（后端每次变更都会广播）。
 *
 * 数据不持久化（规格 Mock 章节：模拟数据仅存内存，重启即清空）。
 */
import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { api } from "@/services/api";
import { onMockStatusChanged } from "@/services/events";
import type { MockLiveData } from "@/types";

export const useMockStore = defineStore("mock", () => {
  /** Mock 模式开关（权威；开启后检测循环使用模拟数据） */
  const enabled = ref(false);
  /** 模拟主播列表 */
  const anchors = ref<MockLiveData[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);

  const count = computed(() => anchors.value.length);
  const liveCount = computed(() => anchors.value.filter((a) => a.is_live).length);

  let unlisten: (() => void) | null = null;

  /** 拉取 Mock 模式状态 */
  async function fetchState() {
    try {
      const state = await api.getMockState();
      enabled.value = state.enabled;
    } catch (e) {
      error.value = String(e);
    }
  }

  /** 拉取模拟主播列表 */
  async function fetchAnchors() {
    try {
      anchors.value = await api.listMockAnchors();
    } catch (e) {
      error.value = String(e);
    }
  }

  /** 拉取状态 + 列表（面板初始加载 / 手动刷新） */
  async function refresh() {
    loading.value = true;
    error.value = null;
    try {
      await Promise.all([fetchState(), fetchAnchors()]);
    } finally {
      loading.value = false;
    }
  }

  /** 切换 Mock 模式（后端命令；成功后以事件/返回为准刷新） */
  async function setMode(enable: boolean) {
    await api.setMockMode(enable);
    enabled.value = enable;
  }

  async function addAnchor(anchor: MockLiveData) {
    await api.addMockAnchor(anchor);
    await fetchAnchors();
  }

  async function updateAnchor(anchor: MockLiveData) {
    await api.updateMockAnchor(anchor);
    await fetchAnchors();
  }

  async function removeAnchor(roomId: string) {
    await api.removeMockAnchor(roomId);
    await fetchAnchors();
  }

  /** 全部开播 / 全部下播 */
  async function setAllLive(live: boolean) {
    await api.setAllMockLive(live);
    await fetchAnchors();
  }

  /** 重置所有模拟数据（清空主播表） */
  async function reset() {
    await api.resetMock();
    await fetchAnchors();
  }

  /** 订阅 `mock:status_changed`（页面卸载时调用 stopListening 清理） */
  function startListening() {
    if (unlisten) return;
    unlisten = onMockStatusChanged(() => {
      refresh();
    });
  }

  function stopListening() {
    if (unlisten) {
      unlisten();
      unlisten = null;
    }
  }

  return {
    enabled,
    anchors,
    loading,
    error,
    count,
    liveCount,
    refresh,
    setMode,
    addAnchor,
    updateAnchor,
    removeAnchor,
    setAllLive,
    reset,
    startListening,
    stopListening,
  };
});
