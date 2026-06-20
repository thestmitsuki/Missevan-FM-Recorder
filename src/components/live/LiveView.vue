<template>
  <div class="live-view">
    <!-- 头部 -->
    <div class="page-header">
      <div class="page-header-left">
        <h1 class="page-title">{{ $t('live.title') }}</h1>
        <div class="header-meta">
          <span class="meta-count">{{ anchorStore.anchors.length }} 主播</span>
          <span v-if="anchorStore.liveCount > 0" class="live-count">
            <span class="live-dot record-pulse"></span>
            {{ anchorStore.liveCount }} 直播中
          </span>
          <span v-if="anchorStore.recordingCount > 0" class="rec-count">
            <span class="rec-dot pulse-dot"></span>
            {{ anchorStore.recordingCount }} 录制中
          </span>
        </div>
      </div>
    </div>

    <!-- 卡片网格：添加卡始终为第一格，主播卡片紧随其后 -->
    <div class="cards-grid">
      <!-- 添加主播卡（始终首位，不参与增删动画） -->
      <div class="add-card" @click="showAddModal = true">
        <div class="add-card-media">
          <span class="material-symbols-outlined add-icon">add</span>
        </div>
        <div class="add-card-body">
          <span class="add-card-label">{{ $t('live.addAnchor') }}</span>
        </div>
      </div>

      <!-- 主播卡片（增删有动画） -->
      <TransitionGroup name="card" tag="div" class="cards-grid-inner">
        <AnchorCard
          v-for="anchor in anchorStore.anchors"
          :key="anchor.id"
          :anchor="anchor"
          :avatar="anchorStore.getAvatar(anchor.id)"
          @start-recording="anchorStore.startRecording(anchor.id)"
          @stop-recording="anchorStore.stopRecording(anchor.id)"
          @open-folder="openFolder"
          @open-settings="openSettings"
          @refresh="handleRefresh"
        />
      </TransitionGroup>
    </div>

    <!-- 空状态提示（仅在无主播时显示，添加卡已提供入口） -->
    <div v-if="anchorStore.anchors.length === 0" class="empty-hint">
      <p>{{ $t('live.noAnchors') }} — 点击上方卡片添加</p>
    </div>

    <!-- 添加主播弹窗 -->
    <Teleport to="body">
      <Transition name="modal">
        <div v-if="showAddModal" class="modal-overlay" @click.self="showAddModal = false">
          <div class="modal-panel glass-heavy">
            <div class="modal-handle"></div>
            <h3 class="modal-title">{{ $t('live.addAnchor') }}</h3>

            <div class="field-group">
              <label class="field-label">{{ $t('live.namePlaceholder') }}</label>
              <input
                v-model="newAnchorName"
                class="field-input"
                :placeholder="$t('live.namePlaceholder')"
                @keyup.enter="addAnchor"
              />
            </div>

            <div class="field-group">
              <label class="field-label">{{ $t('live.urlPlaceholder') }}</label>
              <input
                v-model="newAnchorUrl"
                class="field-input"
                :placeholder="$t('live.urlPlaceholder')"
                @keyup.enter="addAnchor"
              />
            </div>

            <div class="modal-actions">
              <button class="btn btn-ghost" @click="showAddModal = false">{{ $t('common.cancel') }}</button>
              <button class="btn btn-primary" @click="addAnchor">{{ $t('live.add') }}</button>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>

    <!-- 主播设置弹窗 -->
    <Teleport to="body">
      <Transition name="modal">
        <AnchorSettingsView
          v-if="settingsAnchor"
          :anchor="settingsAnchor"
          @close="settingsAnchor = null"
          @settings-changed="handleSettingsChanged"
        />
      </Transition>
    </Teleport>
  </div>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import { useAnchorStore } from '../../stores/anchorStore'
import { useNotificationStore } from '../../stores/notificationStore'
import { invoke } from '@tauri-apps/api/core'
import AnchorCard from './AnchorCard.vue'
import AnchorSettingsView from './AnchorSettingsView.vue'
import type { AnchorInfo } from '../../types'

const anchorStore = useAnchorStore()
const notificationStore = useNotificationStore()

const showAddModal = ref(false)
const newAnchorName = ref('')
const newAnchorUrl = ref('')
const settingsAnchor = ref<AnchorInfo | null>(null)

async function addAnchor() {
  if (!newAnchorName.value.trim() || !newAnchorUrl.value.trim()) {
    notificationStore.show('请填写完整信息', 'warning')
    return
  }
  try {
    await anchorStore.addAnchor(newAnchorName.value.trim(), newAnchorUrl.value.trim())
    showAddModal.value = false
    newAnchorName.value = ''
    newAnchorUrl.value = ''
  } catch {}
}

async function openFolder() {
  try {
    const config: any = await invoke('get_global_config')
    await invoke('open_folder', { path: config.output_dir + '/猫耳FM直播' })
  } catch (e) { notificationStore.show(`打开文件夹失败: ${e}`, 'error') }
}

async function handleRefresh() {
  notificationStore.show('正在刷新...', 'info', 1000)
  await anchorStore.fetchAnchors(true)
}

function openSettings(anchor: AnchorInfo) { settingsAnchor.value = anchor }
async function handleSettingsChanged() {
  await anchorStore.fetchAnchors(true)
  settingsAnchor.value = null
}
</script>

<style scoped>
.live-view { max-width: var(--content-max-width); margin: 0 auto; width: 100%; }

/* 页面头部 */
.page-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  margin-bottom: var(--space-10);
  gap: var(--space-6);
}

.page-header-left { display: flex; flex-direction: column; gap: var(--space-3); }

.page-title {
  font-size: var(--font-hero);
  font-weight: 800;
  letter-spacing: -0.04em;
  color: var(--color-text);
  line-height: 1.1;
}

.header-meta {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  font-size: var(--font-sm);
  color: var(--color-text-secondary);
}

.meta-count { font-weight: 500; }

.live-count {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  color: var(--color-danger);
  font-weight: 500;
}

.live-dot { width: 6px; height: 6px; border-radius: 50%; background: var(--color-danger); }

.rec-count {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  color: var(--color-success);
  font-weight: 500;
}

.rec-dot { width: 6px; height: 6px; }

/* -------- 卡片网格 (CSS Grid 实现完美对齐) -------- */
.cards-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(var(--card-min-width), 1fr));
  gap: var(--space-6);
}

.cards-grid-inner {
  display: contents;  /* 让 TransitionGroup 的子项直接参与外层 grid 布局 */
}

/* -------- 添加主播卡 (与 AnchorCard 完全等尺寸) -------- */
.add-card {
  background: var(--color-surface-solid);
  border-radius: var(--radius-xl);
  overflow: hidden;
  box-shadow: var(--shadow-sm);
  border: 1.5px dashed var(--color-border);
  cursor: pointer;
  transition: all var(--duration-normal) var(--ease-out);
  display: flex;
  flex-direction: column;
}

.add-card:hover {
  border-color: var(--color-primary);
  background: var(--color-primary-light);
  box-shadow: var(--shadow-md);
  transform: translateY(-3px);
}

.add-card:active {
  transform: translateY(0) scale(0.97);
}

/* 封面区 — 16:9 居中 + 图标 */
.add-card-media {
  position: relative;
  width: 100%;
  aspect-ratio: 16 / 9;
  background: var(--color-surface-secondary);
  display: flex;
  align-items: center;
  justify-content: center;
  transition: background var(--duration-normal);
}

.add-card:hover .add-card-media {
  background: var(--color-surface-tertiary);
}

.add-icon {
  font-size: 42px !important;
  color: var(--color-text-tertiary);
  transition: all var(--duration-normal) var(--ease-spring);
  width: 64px;
  height: 64px;
  border-radius: var(--radius-full);
  background: var(--color-surface-solid);
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: var(--shadow-sm);
}

.add-card:hover .add-icon {
  background: var(--color-primary);
  color: white;
  box-shadow: 0 4px 20px var(--color-primary-glow);
  transform: scale(1.05);
}

/* 信息区 — 居中文字 */
.add-card-body {
  padding: var(--space-6) var(--space-8);
  display: flex;
  align-items: center;
  justify-content: center;
  flex: 1;
}

.add-card-label {
  font-size: var(--font-md);
  font-weight: 600;
  color: var(--color-text-secondary);
  letter-spacing: -0.01em;
  transition: color var(--duration-fast);
}

.add-card:hover .add-card-label {
  color: var(--color-primary);
}

/* 空状态提示 */
.empty-hint {
  text-align: center;
  padding: var(--space-16) var(--space-8);
  color: var(--color-text-secondary);
  font-size: var(--font-sm);
}

/* -------- Modal 弹窗 -------- */
.modal-overlay {
  position: fixed; inset: 0;
  background: var(--color-overlay);
  z-index: var(--z-modal);
  display: flex; align-items: center; justify-content: center;
}
.modal-panel {
  width: min(400px, calc(100vw - var(--space-12)));
  padding: var(--space-10) var(--space-9);
  border-radius: var(--radius-2xl);
  box-shadow: var(--shadow-modal), var(--glass-shadow);
  backdrop-filter: blur(60px) saturate(1.6);
  -webkit-backdrop-filter: blur(60px) saturate(1.6);
}
.modal-handle {
  width: 36px; height: 4px; border-radius: 2px;
  background: var(--color-text-tertiary); opacity: 0.3;
  margin: 0 auto var(--space-6);
}
.modal-title {
  font-size: var(--font-xl); font-weight: 700;
  letter-spacing: -0.02em; margin-bottom: var(--space-8);
  color: var(--color-text);
}
.field-group { margin-bottom: var(--space-6); }
.field-label {
  display: block; font-size: var(--font-sm); font-weight: 600;
  color: var(--color-text); margin-bottom: var(--space-3); letter-spacing: -0.01em;
}
.field-input {
  width: 100%; padding: var(--space-5) var(--space-6);
  border: 1.5px solid var(--color-border); border-radius: var(--radius-md);
  font-size: var(--font-md); background: var(--color-surface-solid);
  color: var(--color-text); transition: all var(--duration-fast) var(--ease-out);
}
.field-input:focus { outline: none; border-color: var(--color-primary); box-shadow: 0 0 0 4px var(--color-border-focus); }
.field-input::placeholder { color: var(--color-text-tertiary); }
.modal-actions { display: flex; justify-content: flex-end; gap: var(--space-4); margin-top: var(--space-8); }

.btn {
  display: inline-flex; align-items: center; gap: var(--space-2);
  padding: var(--space-4) var(--space-8); border-radius: var(--radius-full);
  font-size: var(--font-sm); font-weight: 600;
  transition: all var(--duration-fast) var(--ease-out);
}
.btn-primary { background: var(--color-primary); color: white; }
.btn-primary:hover { background: var(--color-primary-hover); }
.btn-ghost { background: var(--color-surface-secondary); color: var(--color-text); }
.btn-ghost:hover { background: var(--color-surface-tertiary); }

/* 响应式 */
@media (max-width: 479px) {
  .page-header { flex-direction: column; align-items: stretch; }
  .page-title { font-size: var(--font-xxl); }
  .header-meta { flex-wrap: wrap; }
}
</style>
