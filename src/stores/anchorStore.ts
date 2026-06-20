import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useNotificationStore } from './notificationStore'
import type { AnchorConfig, AnchorInfo } from '../types'

export const useAnchorStore = defineStore('anchor', () => {
  const anchors = ref<AnchorInfo[]>([])
  const loading = ref(false)
  const error = ref<string | null>(null)
  const notificationStore = useNotificationStore()

  let refreshInterval: ReturnType<typeof setInterval> | null = null
  let isRefreshing = false

  // 响应式头像缓存
  const avatarCache = ref<Map<string, string>>(new Map())
  const TRANSPARENT_PLACEHOLDER =
    'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7'

  // 正在录制数量
  const recordingCount = computed(() =>
    anchors.value.filter((a) => a.is_recording).length
  )

  // 开播数量
  const liveCount = computed(() =>
    anchors.value.filter((a) => a.is_live).length
  )

  // 获取主播列表（含直播状态）
  async function fetchAnchors(force = false) {
    if (loading.value && !force) return
    loading.value = true
    error.value = null
    try {
      const result = await invoke<AnchorInfo[]>('get_anchors')
      anchors.value = result
      // 并发限流加载头像 (最大 3 并发)
      const withAvatar = result.filter((a) => a.avatar)
      await loadAvatarsConcurrently(withAvatar, 3)
    } catch (e) {
      error.value = e as string
      notificationStore.show(`获取主播列表失败: ${e}`, 'error')
    } finally {
      loading.value = false
    }
  }

  // 并发限流加载头像
  async function loadAvatarsConcurrently(list: AnchorInfo[], concurrency = 3) {
    for (let i = 0; i < list.length; i += concurrency) {
      const chunk = list.slice(i, i + concurrency)
      await Promise.all(
        chunk.map((a) => (a.avatar ? loadAvatar(a.id, a.avatar!) : Promise.resolve()))
      )
    }
  }

  async function loadAvatar(anchorId: string, url: string) {
    if (avatarCache.value.has(anchorId)) return
    try {
      const base64 = await invoke<string>('get_avatar_base64', {
        avatarUrl: url,
      })
      avatarCache.value.set(anchorId, base64)
      // 更新对应主播的 avatar 为 base64
      const anchor = anchors.value.find((a) => a.id === anchorId)
      if (anchor) {
        anchor.avatar = base64
      }
    } catch (e) {
      console.error(`加载头像失败 ${anchorId}:`, e)
      avatarCache.value.set(anchorId, TRANSPARENT_PLACEHOLDER)
    }
  }

  function getAvatar(anchorId: string): string {
    return avatarCache.value.get(anchorId) ?? TRANSPARENT_PLACEHOLDER
  }

  // 开始录制
  async function startRecording(anchorId: string) {
    try {
      await invoke('start_recording_anchor', { anchorId })
      notificationStore.show('开始录制', 'info')
      await fetchAnchors(true)
    } catch (e) {
      notificationStore.show(`启动录制失败: ${e}`, 'error')
    }
  }

  // 停止录制
  async function stopRecording(anchorId: string) {
    try {
      await invoke('stop_recording_anchor', { anchorId })
      notificationStore.show('已停止录制', 'info')
      await fetchAnchors(true)
    } catch (e) {
      notificationStore.show(`停止录制失败: ${e}`, 'error')
    }
  }

  // 添加主播
  async function addAnchor(name: string, url: string) {
    try {
      const newAnchor = await invoke<AnchorConfig>('add_anchor', { name, url })
      await fetchAnchors(true)
      notificationStore.show(`主播 ${name} 添加成功`, 'info')
      return newAnchor
    } catch (e) {
      notificationStore.show(`添加主播失败: ${e}`, 'error')
      throw e
    }
  }

  // 删除主播
  async function removeAnchor(anchorId: string) {
    try {
      await invoke('remove_anchor', { anchorId })
      await fetchAnchors(true)
      notificationStore.show('主播已删除', 'info')
    } catch (e) {
      notificationStore.show(`删除主播失败: ${e}`, 'error')
    }
  }

  // 更新主播配置
  async function updateAnchorConfig(anchorConfig: AnchorConfig) {
    try {
      await invoke('update_anchor_config', { anchorConfig })
      await fetchAnchors(true)
      notificationStore.show('主播配置已更新', 'info')
    } catch (e) {
      notificationStore.show(`更新配置失败: ${e}`, 'error')
    }
  }

  // 自动轮询（每 5 秒）
  function startAutoRefresh() {
    if (refreshInterval) return
    refreshInterval = window.setInterval(() => {
      if (!isRefreshing) {
        isRefreshing = true
        fetchAnchors(false).finally(() => {
          isRefreshing = false
        })
      }
    }, 5000)
  }

  function stopAutoRefresh() {
    if (refreshInterval) {
      clearInterval(refreshInterval)
      refreshInterval = null
    }
  }

  // 初始化
  fetchAnchors()
  startAutoRefresh()

  return {
    anchors,
    loading,
    error,
    recordingCount,
    liveCount,
    fetchAnchors,
    getAvatar,
    startRecording,
    stopRecording,
    addAnchor,
    removeAnchor,
    updateAnchorConfig,
    startAutoRefresh,
    stopAutoRefresh,
  }
})
