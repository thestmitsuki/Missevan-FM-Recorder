import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useNotificationStore } from './notificationStore'
import type { GlobalConfig } from '../types'

export const useConfigStore = defineStore('config', () => {
  const config = ref<GlobalConfig | null>(null)
  const loading = ref(false)
  const notificationStore = useNotificationStore()

  async function loadConfig() {
    loading.value = true
    try {
      // 后端返回完整的 Config（含 anchors），前端只取全局字段
      const result: GlobalConfig & { anchors?: unknown[] } = await invoke('get_global_config')
      const { anchors: _anchors, ...globalFields } = result
      config.value = globalFields as GlobalConfig
    } catch (e) {
      notificationStore.show(`加载配置失败: ${e}`, 'error')
      console.error(e)
    } finally {
      loading.value = false
    }
  }

  async function saveConfig(newConfig: Partial<GlobalConfig>) {
    if (!config.value) return
    const merged = { ...config.value, ...newConfig }
    try {
      // 后端 update_global_config 接收完整的 Config（含 anchors）
      type FullConfig = GlobalConfig & { anchors: unknown[] }
      const anchors = (await invoke<FullConfig>('get_global_config')).anchors
      await invoke('update_global_config', {
        newConfig: { ...merged, anchors },
      })
      config.value = merged
      notificationStore.show('配置已保存', 'info')
    } catch (e) {
      notificationStore.show(`保存配置失败: ${e}`, 'error')
      throw e
    }
  }

  async function runChecks() {
    try {
      await invoke('run_all_checks_backend')
      notificationStore.show('检查已触发', 'info')
    } catch (e) {
      notificationStore.show(`触发检查失败: ${e}`, 'error')
    }
  }

  loadConfig()

  return {
    config,
    loading,
    loadConfig,
    saveConfig,
    runChecks,
  }
})
