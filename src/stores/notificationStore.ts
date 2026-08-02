import { defineStore } from "pinia";
import { ref, computed } from "vue";
import type { Notification } from "@/types";
import { onNotification } from "@/services/events";

let unlisten: (() => void) | null = null;

export const useNotificationStore = defineStore("notification", () => {
  const notifications = ref<Notification[]>([]);
  const maxNotifications = ref(50);

  const unreadCount = computed(() => notifications.value.length);

  const latestNotification = computed(() =>
    notifications.value.length > 0
      ? notifications.value[notifications.value.length - 1]
      : null,
  );

  const hasErrors = computed(() =>
    notifications.value.some(
      (n) => n.level === "Error" || n.level === "Critical",
    ),
  );

  function addNotification(notification: Notification) {
    notifications.value.push(notification);
    // 保持队列上限
    while (notifications.value.length > maxNotifications.value) {
      notifications.value.shift();
    }
  }

  function removeNotification(id: string) {
    notifications.value = notifications.value.filter((n) => n.id !== id);
  }

  function clearAll() {
    notifications.value = [];
  }

  function clearBySource(source: string) {
    notifications.value = notifications.value.filter(
      (n) => n.source !== source,
    );
  }

  // 启动监听（仅需调用一次）
  function startListening() {
    if (unlisten) return;
    unlisten = onNotification((notification) => {
      addNotification(notification);
    });
  }

  function stopListening() {
    if (unlisten) {
      unlisten();
      unlisten = null;
    }
  }

  return {
    notifications,
    maxNotifications,
    unreadCount,
    latestNotification,
    hasErrors,
    addNotification,
    removeNotification,
    clearAll,
    clearBySource,
    startListening,
    stopListening,
  };
});
