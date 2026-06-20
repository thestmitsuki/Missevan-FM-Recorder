<template>
  <div class="anchor-card" :class="{ 'is-recording': anchor.is_recording, 'is-live': anchor.is_live }">
    <!-- 封面区 -->
    <div class="card-media">
      <img
        :src="avatar || TRANSPARENT_PLACEHOLDER"
        class="card-cover"
        @error="onImageError"
        loading="lazy"
      />
      <!-- 状态叠加 -->
      <div class="media-overlay">
        <div v-if="anchor.is_recording" class="tag tag-recording">
          <span class="pulse-dot"></span>
          {{ $t('live.recording') }}
        </div>
        <div v-if="anchor.is_live" class="tag tag-live">
          <span class="live-dot record-pulse"></span>
          {{ $t('live.live') }}
        </div>
      </div>

      <!-- 录制状态条 -->
      <div v-if="anchor.is_recording" class="recording-bar">
        <div class="recording-bar-track">
          <div class="recording-bar-fill"></div>
        </div>
      </div>
    </div>

    <!-- 信息区 -->
    <div class="card-body">
      <div class="card-body-top">
        <h3 class="card-name">{{ anchor.name }}</h3>
        <p class="card-title">{{ anchor.title || '—' }}</p>
      </div>

      <div class="card-footer">
        <div class="card-stats">
          <span class="card-stat" :class="{ 'text-green': anchor.is_recording }">
            <span class="material-symbols-outlined stat-icon">fiber_manual_record</span>
            {{ anchor.is_recording ? $t('live.recording') : $t('live.notRecording') }}
          </span>
          <span v-if="anchor.config?.enable_check" class="card-stat text-muted">
            <span class="material-symbols-outlined stat-icon">visibility</span>
            {{ $t('live.monitoring') }}
          </span>
        </div>
        <button class="more-btn" ref="settingsBtnRef" @click="toggleDropdown">
          <span class="material-symbols-outlined">more_horiz</span>
        </button>
      </div>
    </div>

    <!-- 下拉菜单 -->
    <Teleport to="body">
      <Transition name="dropdown">
        <div
          v-if="dropdownOpen"
          ref="dropdownRef"
          class="dropdown"
          :style="dropdownStyle"
          @click.stop
        >
          <button class="dropdown-item" @click="handleStartStop">
            <span class="material-symbols-outlined">
              {{ anchor.is_recording ? 'pause_circle' : 'play_circle' }}
            </span>
            <span>{{ anchor.is_recording ? $t('live.dropdown.stop') : $t('live.dropdown.start') }}</span>
          </button>
          <button class="dropdown-item" @click="emit('open-folder')">
            <span class="material-symbols-outlined">folder_open</span>
            {{ $t('live.dropdown.folder') }}
          </button>
          <button class="dropdown-item" @click="emit('open-settings', anchor)">
            <span class="material-symbols-outlined">settings</span>
            {{ $t('live.dropdown.settings') }}
          </button>
          <div class="dropdown-divider"></div>
          <button class="dropdown-item" @click="emit('refresh')">
            <span class="material-symbols-outlined">refresh</span>
            {{ $t('live.dropdown.refresh') }}
          </button>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, onUnmounted, nextTick } from 'vue'
import { computePosition, autoUpdate, flip, shift, offset } from '@floating-ui/dom'
import type { AnchorInfo } from '../../types'

const props = defineProps<{ anchor: AnchorInfo; avatar?: string }>()
const emit = defineEmits<{
  (e: 'start-recording'): void
  (e: 'stop-recording'): void
  (e: 'open-folder'): void
  (e: 'open-settings', anchor: AnchorInfo): void
  (e: 'refresh'): void
}>()

const TRANSPARENT_PLACEHOLDER = 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7'

const dropdownOpen = ref(false)
const dropdownRef = ref<HTMLElement | null>(null)
const settingsBtnRef = ref<HTMLElement | null>(null)
const dropdownStyle = ref({ top: '0px', left: '0px' })
let cleanup: (() => void) | null = null

function onImageError(e: Event) { (e.target as HTMLImageElement).src = TRANSPARENT_PLACEHOLDER }

function toggleDropdown() {
  dropdownOpen.value = !dropdownOpen.value
  if (dropdownOpen.value) nextTick(() => {
    if (dropdownRef.value && settingsBtnRef.value) {
      updatePosition()
      cleanup = autoUpdate(settingsBtnRef.value, dropdownRef.value, updatePosition)
    }
  })
  else { cleanup?.(); cleanup = null }
}

function updatePosition() {
  if (!settingsBtnRef.value || !dropdownRef.value) return
  computePosition(settingsBtnRef.value, dropdownRef.value, {
    placement: 'bottom-end',
    middleware: [flip(), shift({ padding: 10 }), offset(6)],
  }).then(({ x, y }) => { dropdownStyle.value = { top: `${y}px`, left: `${x}px` } })
}

function handleClickOutside(e: MouseEvent) {
  const t = e.target as HTMLElement
  if (dropdownOpen.value && !t.closest('.dropdown') && !t.closest('.more-btn')) {
    dropdownOpen.value = false; cleanup?.(); cleanup = null
  }
}

onMounted(() => document.addEventListener('click', handleClickOutside))
onUnmounted(() => { document.removeEventListener('click', handleClickOutside); cleanup?.() })

function handleStartStop() {
  if (props.anchor.is_recording) emit('stop-recording')
  else emit('start-recording')
  dropdownOpen.value = false
}
</script>

<style scoped>
/* -------- 卡片 Apple 风格 -------- */
.anchor-card {
  background: var(--color-surface-solid);
  border-radius: var(--radius-xl);
  overflow: hidden;
  box-shadow: var(--shadow-sm);
  transition: all var(--duration-normal) var(--ease-out);
  border: 0.5px solid var(--color-border);
}

.anchor-card:hover {
  transform: translateY(-3px);
  box-shadow: var(--shadow-lg);
}

.anchor-card.is-recording {
  box-shadow: 0 0 0 1px var(--color-success), var(--shadow-sm);
}

/* 媒体区 */
.card-media {
  position: relative;
  aspect-ratio: 16 / 9;
  background: var(--color-surface-secondary);
  overflow: hidden;
}

.card-cover {
  width: 100%;
  height: 100%;
  object-fit: cover;
  transition: transform var(--duration-slower) var(--ease-out);
}

.anchor-card:hover .card-cover {
  transform: scale(1.05);
}

.media-overlay {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  display: flex;
  justify-content: space-between;
  padding: var(--space-4);
}

.tag {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-4);
  border-radius: var(--radius-full);
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.02em;
  backdrop-filter: blur(20px);
  -webkit-backdrop-filter: blur(20px);
}

.tag-live {
  background: rgba(255, 59, 48, 0.85);
  color: white;
}

.live-dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: white;
}

.tag-recording {
  background: rgba(0, 0, 0, 0.6);
  color: white;
}

.tag-recording .pulse-dot { width: 5px; height: 5px; }

/* 录制进度条 */
.recording-bar {
  position: absolute;
  bottom: 0;
  left: 0;
  right: 0;
  height: 2px;
  background: rgba(0, 0, 0, 0.15);
}

.recording-bar-track {
  height: 100%;
  width: 100%;
}

.recording-bar-fill {
  height: 100%;
  background: var(--color-success);
  animation: progress-pulse 2s ease-in-out infinite;
}

/* 信息区 */
.card-body {
  padding: var(--space-6) var(--space-8) var(--space-5);
}

.card-body-top {
  margin-bottom: var(--space-5);
}

.card-name {
  font-size: var(--font-lg);
  font-weight: 700;
  letter-spacing: -0.02em;
  margin-bottom: var(--space-1);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.card-title {
  font-size: var(--font-sm);
  color: var(--color-text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  letter-spacing: -0.01em;
}

/* 底部 */
.card-footer {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding-top: var(--space-5);
  border-top: 0.5px solid var(--color-border);
}

.card-stats {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  font-size: var(--font-xs);
  color: var(--color-text-secondary);
}

.card-stat {
  display: inline-flex;
  align-items: center;
  gap: 3px;
}

.stat-icon { font-size: 14px !important; }

.text-green { color: var(--color-success); }
.text-muted { color: var(--color-text-tertiary); }

/* 更多按钮 */
.more-btn {
  width: 30px;
  height: 30px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--color-text-secondary);
  transition: all var(--duration-fast);
  flex-shrink: 0;
}

.more-btn:hover {
  background: var(--color-surface-secondary);
  color: var(--color-text);
}

.more-btn .material-symbols-outlined { font-size: 20px; }
</style>

<!-- 下拉菜单全局样式 -->
<style>
.dropdown {
  position: fixed;
  z-index: var(--z-dropdown, 500);
  background: var(--color-surface-solid);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-dropdown);
  min-width: 180px;
  padding: var(--space-2);
  border: 0.5px solid var(--color-border);
  backdrop-filter: blur(40px);
  -webkit-backdrop-filter: blur(40px);
}

.dropdown-item {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  width: 100%;
  padding: var(--space-4) var(--space-5);
  border-radius: var(--radius-sm);
  font-size: var(--font-sm);
  color: var(--color-text);
  transition: background var(--duration-fast);
  text-align: left;
  font-weight: 500;
}

.dropdown-item:hover {
  background: var(--color-surface-secondary);
}

.dropdown-item .material-symbols-outlined {
  font-size: 18px;
  color: var(--color-text-secondary);
}

.dropdown-divider {
  height: 0.5px;
  background: var(--color-border);
  margin: var(--space-1) var(--space-4);
}
</style>
