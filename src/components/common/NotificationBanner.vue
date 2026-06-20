<template>
  <div class="notification-container">
    <TransitionGroup name="slide-down">
      <div
        v-for="notif in notificationStore.notifications"
        :key="notif.id"
        v-show="notif.visible"
        class="toast glass"
        :class="notif.level"
      >
        <div class="toast-icon">
          <span class="material-symbols-outlined">
            {{ iconMap[notif.level] }}
          </span>
        </div>
        <span class="toast-message">{{ notif.message }}</span>
        <button class="toast-close" @click="notificationStore.hide(notif.id)">
          <span class="material-symbols-outlined">close</span>
        </button>
      </div>
    </TransitionGroup>
  </div>
</template>

<script setup lang="ts">
import { useNotificationStore } from '../../stores/notificationStore'

const notificationStore = useNotificationStore()

const iconMap: Record<string, string> = {
  info: 'check_circle',
  warning: 'warning',
  error: 'error',
}
</script>

<style scoped>
.notification-container {
  position: fixed;
  top: var(--space-6);
  right: var(--space-6);
  z-index: var(--z-notification);
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  max-width: min(400px, calc(100vw - var(--space-8)));
  width: 100%;
  pointer-events: none;
}

.toast {
  pointer-events: auto;
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: var(--space-5) var(--space-6);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-lg), var(--glass-shadow);
  transition: all var(--duration-normal) var(--ease-out);
}

.toast.info { background: rgba(0, 122, 255, 0.12); backdrop-filter: blur(40px); }
.toast.warning { background: rgba(255, 149, 0, 0.12); backdrop-filter: blur(40px); }
.toast.error { background: rgba(255, 59, 48, 0.12); backdrop-filter: blur(40px); }

.toast-icon .material-symbols-outlined {
  font-size: 20px;
}
.toast.info .toast-icon { color: var(--color-primary); }
.toast.warning .toast-icon { color: var(--color-warning); }
.toast.error .toast-icon { color: var(--color-danger); }

.toast-message {
  font-size: var(--font-sm);
  color: var(--color-text);
  white-space: pre-line;
  flex: 1;
  min-width: 0;
  line-height: 1.4;
}

.toast-close {
  flex-shrink: 0;
  width: 24px;
  height: 24px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--radius-full);
  color: var(--color-text-tertiary);
  transition: all var(--duration-fast);
  opacity: 0.6;
}

.toast-close:hover {
  opacity: 1;
  background: var(--color-surface-secondary);
}

.toast-close .material-symbols-outlined {
  font-size: 16px;
}

@media (max-width: 479px) {
  .notification-container {
    top: var(--space-2);
    right: var(--space-2);
    left: var(--space-2);
    max-width: none;
  }
}
</style>
