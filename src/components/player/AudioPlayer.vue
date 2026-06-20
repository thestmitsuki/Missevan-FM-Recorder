<template>
  <Teleport to="body">
    <Transition name="modal">
      <div v-if="visible" class="player-overlay" @click.self="close">
        <div class="player-panel glass-heavy">
          <div class="player-artwork">
            <div class="artwork-frame">
              <img
                :src="albumArt || TRANSPARENT_PLACEHOLDER"
                class="artwork-img"
                referrerpolicy="no-referrer"
                @error="handleImageError"
              />
            </div>
          </div>

          <div class="player-info">
            <h3 class="player-title">{{ currentTrack?.name || $t('common.loading') }}</h3>
            <p class="player-artist">猫耳FM</p>
          </div>

          <div class="player-info-note">
            <span class="material-symbols-outlined">info</span>
            将通过系统默认播放器打开
          </div>

          <div class="player-controls">
            <button class="ctrl-btn ctrl-primary" @click="playWithSystem">
              <span class="material-symbols-outlined">play_circle_filled</span>
            </button>
          </div>

          <button class="player-close" @click="close">
            <span class="material-symbols-outlined">close</span>
          </button>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useNotificationStore } from '../../stores/notificationStore'

const TRANSPARENT_PLACEHOLDER = 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7'

interface Track { name: string; path: string }

const visible = ref(false)
const currentTrack = ref<Track | null>(null)
const albumArt = ref(TRANSPARENT_PLACEHOLDER)
const notificationStore = useNotificationStore()

let cachedAvatarBase64: string | null = null

async function fetchAnchorAvatar(): Promise<string> {
  if (cachedAvatarBase64) return cachedAvatarBase64
  try {
    const anchors = await invoke<any[]>('get_anchors')
    const avatarUrl = anchors?.[0]?.avatar
    if (!avatarUrl) throw new Error('no avatar')
    const base64 = await invoke<string>('get_avatar_base64', { avatarUrl })
    cachedAvatarBase64 = base64
    return base64
  } catch { return TRANSPARENT_PLACEHOLDER }
}

async function open(track: Track) {
  currentTrack.value = track
  visible.value = true
  albumArt.value = await fetchAnchorAvatar()
}

function close() {
  visible.value = false
  currentTrack.value = null
}

async function playWithSystem() {
  if (!currentTrack.value) return
  try {
    await invoke('open_with_system', { path: currentTrack.value.path })
    close()
  } catch (e) {
    notificationStore.show(`播放失败: ${e}`, 'error')
  }
}

function handleImageError() { albumArt.value = TRANSPARENT_PLACEHOLDER }

defineExpose({ open })
</script>

<style scoped>
.player-overlay {
  position: fixed; inset: 0;
  background: var(--color-overlay);
  display: flex; align-items: center; justify-content: center;
  z-index: var(--z-modal);
}
.player-panel {
  width: min(340px, calc(100vw - var(--space-12)));
  padding: var(--space-12);
  border-radius: var(--radius-2xl);
  box-shadow: var(--shadow-2xl), var(--glass-shadow);
  backdrop-filter: blur(60px) saturate(1.6);
  -webkit-backdrop-filter: blur(60px) saturate(1.6);
  position: relative;
}
.player-artwork { display: flex; justify-content: center; margin-bottom: var(--space-10); }
.artwork-frame {
  width: 180px; height: 180px; border-radius: var(--radius-xl); overflow: hidden;
  box-shadow: var(--shadow-xl); background: var(--color-surface-secondary);
}
.artwork-img { width: 100%; height: 100%; object-fit: cover; }
.player-info { text-align: center; margin-bottom: var(--space-4); }
.player-title { font-size: var(--font-lg); font-weight: 700; letter-spacing: -0.02em; margin-bottom: var(--space-2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.player-artist { font-size: var(--font-sm); color: var(--color-text-secondary); }
.player-info-note {
  text-align: center; font-size: var(--font-xs); color: var(--color-text-tertiary);
  margin-bottom: var(--space-8); display: flex; align-items: center; justify-content: center; gap: var(--space-2);
}
.player-info-note .material-symbols-outlined { font-size: 14px; }
.player-controls { display: flex; justify-content: center; align-items: center; }
.ctrl-primary { opacity: 1; color: var(--color-text); transition: all var(--duration-fast); }
.ctrl-primary:hover { transform: scale(1.05); }
.ctrl-primary:active { transform: scale(0.92); }
.ctrl-primary .material-symbols-outlined { font-size: 56px; }
.player-close {
  position: absolute; top: var(--space-6); right: var(--space-6);
  width: 28px; height: 28px; border-radius: var(--radius-full);
  display: flex; align-items: center; justify-content: center;
  color: var(--color-text-tertiary); transition: all var(--duration-fast);
}
.player-close:hover { background: var(--color-surface-secondary); color: var(--color-text); }
.player-close .material-symbols-outlined { font-size: 18px; }
</style>
