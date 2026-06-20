<template>
  <div class="app-container app-enter">
    <NotificationBanner />

    <!-- 桌面端侧边导航 + 移动端底部导航 -->
    <div class="app-layout">
      <!-- 侧边栏（桌面端） -->
      <nav class="sidebar glass" v-if="isDesktop">
        <div class="sidebar-brand">
          <div class="brand-icon">
            <span class="material-symbols-outlined">radio</span>
          </div>
          <span class="brand-text">猫耳FM</span>
        </div>

        <div class="sidebar-nav">
          <button
            v-for="tab in tabs"
            :key="tab.key"
            @click="currentTab = tab.key"
            class="sidebar-item"
            :class="{ active: currentTab === tab.key }"
          >
            <span class="material-symbols-outlined sidebar-item-icon">{{ tab.icon }}</span>
            <span class="sidebar-item-label">{{ $t(`nav.${tab.key}`) }}</span>
          </button>
        </div>

        <div class="sidebar-footer">
          <div class="recording-badge" v-if="anchorStore.recordingCount > 0">
            <span class="badge-dot record-pulse"></span>
            <span>{{ anchorStore.recordingCount }}</span>
          </div>
        </div>
      </nav>

      <!-- 主内容区 -->
      <main class="main-area">
        <div class="main-scroll">
          <Transition name="view-fade" mode="out-in">
            <KeepAlive>
              <component :is="currentView" :key="currentTab" />
            </KeepAlive>
          </Transition>
        </div>
      </main>
    </div>

    <!-- 移动端底部导航 -->
    <nav class="bottom-nav glass-heavy" v-if="!isDesktop">
      <button
        v-for="tab in tabs"
        :key="tab.key"
        @click="currentTab = tab.key"
        class="bottom-nav-item"
        :class="{ active: currentTab === tab.key }"
      >
        <span class="material-symbols-outlined bottom-nav-icon">{{ tab.icon }}</span>
        <span class="bottom-nav-label">{{ $t(`nav.${tab.key}`) }}</span>
      </button>
    </nav>

    <AudioPlayer ref="audioPlayerRef" />
  </div>
</template>

<script setup lang="ts">
import { ref, shallowRef, watch, provide } from 'vue'
import { useResponsive } from './composables/useResponsive'
import { useNotificationStore } from './stores/notificationStore'
import { useAnchorStore } from './stores/anchorStore'
import type { TabKey, TabItem } from './types'
import LiveView from './components/live/LiveView.vue'
import FilesView from './components/files/FilesView.vue'
import SettingsView from './components/settings/SettingsView.vue'
import AudioPlayer from './components/player/AudioPlayer.vue'
import NotificationBanner from './components/common/NotificationBanner.vue'

const notificationStore = useNotificationStore()
const anchorStore = useAnchorStore()
const { isDesktop } = useResponsive()

provide('showNotification', (message: string, level: 'info' | 'warning' | 'error' = 'info', duration = 3000) => {
  notificationStore.show(message, level, duration)
})

const tabs: TabItem[] = [
  { key: 'live', icon: 'mic' },
  { key: 'files', icon: 'folder' },
  { key: 'settings', icon: 'settings' },
]

const currentTab = ref<TabKey>('live')
const currentView = shallowRef(LiveView)

watch(currentTab, (tab) => {
  if (tab === 'live') currentView.value = LiveView
  else if (tab === 'files') currentView.value = FilesView
  else currentView.value = SettingsView
})
</script>

<style scoped>
.app-container {
  display: flex;
  flex-direction: column;
  height: 100dvh;
  width: 100%;
  background: var(--color-bg);
  font-family: var(--font-family);
  overflow: hidden;
}

/* -------- 桌面布局 (侧边栏 + 内容) -------- */
.app-layout {
  display: flex;
  flex: 1;
  overflow: hidden;
}

/* 侧边栏 */
.sidebar {
  width: 200px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  padding: var(--space-6) 0;
  border-right: 0.5px solid var(--color-border);
  z-index: var(--z-nav);
}

.sidebar-brand {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: var(--space-4) var(--space-8) var(--space-8);
  margin-bottom: var(--space-4);
}

.brand-icon {
  width: 32px;
  height: 32px;
  border-radius: var(--radius-md);
  background: var(--color-primary);
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 2px 8px var(--color-primary-glow);
}

.brand-icon .material-symbols-outlined {
  color: white;
  font-size: 20px;
}

.brand-text {
  font-size: var(--font-lg);
  font-weight: 700;
  letter-spacing: -0.03em;
  color: var(--color-text);
}

.sidebar-nav {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 0 var(--space-4);
}

.sidebar-item {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: var(--space-4) var(--space-6);
  border-radius: var(--radius-md);
  font-size: var(--font-sm);
  font-weight: 500;
  color: var(--color-text-secondary);
  transition: all var(--duration-fast) var(--ease-out);
  text-align: left;
  width: 100%;
}

.sidebar-item:hover {
  background: var(--color-surface-secondary);
  color: var(--color-text);
}

.sidebar-item.active {
  background: var(--color-primary-light);
  color: var(--color-primary);
}

.sidebar-item-icon {
  font-size: 20px !important;
}

.sidebar-footer {
  padding: var(--space-4);
  display: flex;
  justify-content: center;
}

.recording-badge {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-4);
  background: var(--color-danger-light);
  color: var(--color-danger);
  border-radius: var(--radius-full);
  font-size: var(--font-xs);
  font-weight: 600;
}

.badge-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--color-danger);
}

/* -------- 主内容区 -------- */
.main-area {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  min-width: 0;
}

.main-scroll {
  flex: 1;
  overflow-y: auto;
  overflow-x: hidden;
  padding: var(--space-10) var(--space-10) var(--space-10);
}

/* -------- 移动端底部导航 -------- */
.bottom-nav {
  position: fixed;
  bottom: 0;
  left: 0;
  right: 0;
  display: flex;
  justify-content: space-around;
  align-items: center;
  height: var(--nav-height);
  border-top: 0.5px solid var(--color-border);
  padding: var(--space-2) var(--space-4) var(--space-5);
  z-index: var(--z-nav);
}

.bottom-nav-item {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 1px;
  padding: var(--space-2) var(--space-8);
  border-radius: var(--radius-full);
  font-size: 10px;
  color: var(--color-text-secondary);
  transition: all var(--duration-fast) var(--ease-out);
  position: relative;
}

.bottom-nav-item.active {
  color: var(--color-primary);
}

.bottom-nav-icon {
  font-size: 24px !important;
  transition: transform var(--duration-fast) var(--ease-out);
}

.bottom-nav-item:active .bottom-nav-icon {
  transform: scale(0.85);
}

.bottom-nav-item.active .bottom-nav-icon {
  font-variation-settings: 'FILL' 1;
}

.bottom-nav-label {
  font-weight: 500;
  font-size: 10px;
  letter-spacing: 0.01em;
}

/* -------- 响应式调整 -------- */
@media (max-width: 767px) {
  .main-scroll {
    padding: var(--space-8) var(--space-6) calc(var(--nav-height) + var(--space-8)) var(--space-6);
  }
}

@media (max-width: 479px) {
  .main-scroll {
    padding: var(--space-6) var(--space-4) calc(var(--nav-height) + var(--space-6)) var(--space-4);
  }
}
</style>
