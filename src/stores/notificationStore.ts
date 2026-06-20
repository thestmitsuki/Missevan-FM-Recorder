import { defineStore } from 'pinia'
import { ref, readonly } from 'vue'
import type { NotificationItem, NotificationLevel } from '../types'

export const useNotificationStore = defineStore('notification', () => {
  const notifications = ref<NotificationItem[]>([])
  let nextId = 0
  const timers = new Map<number, number>()

  function show(
    message: string,
    level: NotificationLevel = 'info',
    duration = 3000
  ) {
    const id = nextId++
    const item: NotificationItem = {
      id,
      message,
      level,
      duration,
      visible: true,
    }
    notifications.value.push(item)
    const timer = window.setTimeout(() => hide(id), duration)
    timers.set(id, timer)
    return id
  }

  function hide(id: number) {
    const timer = timers.get(id)
    if (timer) {
      clearTimeout(timer)
      timers.delete(id)
    }
    const idx = notifications.value.findIndex((n) => n.id === id)
    if (idx !== -1) {
      notifications.value[idx].visible = false
      setTimeout(() => {
        notifications.value = notifications.value.filter((n) => n.id !== id)
      }, 300)
    }
  }

  function clearAll() {
    for (const [, timer] of timers) {
      clearTimeout(timer)
    }
    timers.clear()
    notifications.value = []
  }

  return {
    notifications: readonly(notifications),
    show,
    hide,
    clearAll,
  }
})
