<template>
  <div class="settings-view">
    <div class="page-header">
      <h1 class="page-title">{{ $t('settings.title') }}</h1>
    </div>

    <!-- 录制设置 -->
    <section class="settings-section">
      <h2 class="section-header">录制</h2>
      <div class="section-card">
        <!-- 输出目录 -->
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-icon">
              <span class="material-symbols-outlined">folder</span>
            </span>
            <div class="setting-text">
              <span class="setting-label">{{ $t('settings.outputDir') }}</span>
              <span class="setting-value">{{ localOutputDir }}</span>
            </div>
          </div>
          <button class="btn btn-ghost btn-sm" @click="pickOutputDir">
            <span class="material-symbols-outlined">folder_open</span>
            {{ $t('settings.browse') }}
          </button>
        </div>

        <div class="setting-divider"></div>

        <!-- 录制格式 -->
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-icon">
              <span class="material-symbols-outlined">audio_file</span>
            </span>
            <div class="setting-text">
              <span class="setting-label">{{ $t('settings.recordFormat') }}</span>
            </div>
          </div>
          <div class="segmented-control">
            <button
              class="segmented-item"
              :class="{ active: localFormat === 'M4A' }"
              @click="localFormat = 'M4A'"
            >M4A</button>
            <button
              class="segmented-item"
              :class="{ active: localFormat === 'MP3' }"
              @click="localFormat = 'MP3'"
            >MP3</button>
          </div>
        </div>

        <div class="setting-divider"></div>

        <!-- 分段时长 -->
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-icon">
              <span class="material-symbols-outlined">content_cut</span>
            </span>
            <div class="setting-text">
              <span class="setting-label">{{ $t('settings.segmentSeconds') }}</span>
              <span class="setting-hint">{{ $t('settings.segmentHint') }}</span>
            </div>
          </div>
          <div class="input-suffix">
            <input
              type="number"
              class="inline-input"
              v-model.number="localSegmentSeconds"
              min="0"
              max="3600"
              step="60"
            />
            <span class="suffix">秒</span>
          </div>
        </div>
      </div>
    </section>

    <!-- 存储设置 -->
    <section class="settings-section">
      <h2 class="section-header">存储</h2>
      <div class="section-card">
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-icon">
              <span class="material-symbols-outlined">storage</span>
            </span>
            <div class="setting-text">
              <span class="setting-label">{{ $t('settings.diskSpace') }}</span>
            </div>
          </div>
          <div class="input-suffix">
            <input
              type="number"
              class="inline-input"
              step="0.5"
              min="0.1"
              v-model.number="localDiskSpace"
            />
            <span class="suffix">GB</span>
          </div>
        </div>
      </div>
    </section>

    <!-- FFmpeg 设置 -->
    <section class="settings-section">
      <h2 class="section-header">FFmpeg</h2>
      <div class="section-card">
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-icon">
              <span class="material-symbols-outlined">videocam</span>
            </span>
            <div class="setting-text" style="flex:1">
              <span class="setting-label">{{ $t('settings.ffmpegPath') }}</span>
              <input
                v-model="localFfmpegPath"
                class="text-input"
                :placeholder="$t('settings.ffmpegHint')"
              />
            </div>
          </div>
        </div>
        <div class="setting-hint-row">
          <span class="material-symbols-outlined">info</span>
          {{ $t('settings.ffmpegHint') }}
        </div>
      </div>
    </section>

    <!-- 操作 -->
    <div class="settings-actions">
      <button class="btn btn-primary btn-large" @click="saveConfig">
        <span class="material-symbols-outlined">save</span>
        {{ $t('settings.save') }}
      </button>
      <button class="btn btn-ghost btn-large" @click="runChecks">
        <span class="material-symbols-outlined">check_circle</span>
        {{ $t('settings.checkAll') }}
      </button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { useConfigStore } from '../../stores/configStore'
import { useNotificationStore } from '../../stores/notificationStore'

const { t } = useI18n()
const configStore = useConfigStore()
const notificationStore = useNotificationStore()

const localOutputDir = ref('./downloads')
const localFormat = ref<'M4A' | 'MP3'>('M4A')
const localSegmentSeconds = ref(0)
const localDiskSpace = ref(1.0)
const localFfmpegPath = ref('')

onMounted(async () => {
  await configStore.loadConfig()
  if (configStore.config) {
    localOutputDir.value = configStore.config.output_dir
    localFormat.value = configStore.config.record_format
    localSegmentSeconds.value = configStore.config.segment_seconds
    localDiskSpace.value = configStore.config.disk_space_limit_gb
    localFfmpegPath.value = configStore.config.ffmpeg_path || ''
  }
})

async function saveConfig() {
  try {
    await configStore.saveConfig({
      output_dir: localOutputDir.value,
      record_format: localFormat.value,
      segment_seconds: localSegmentSeconds.value,
      disk_space_limit_gb: localDiskSpace.value,
      ffmpeg_path: localFfmpegPath.value.trim() || null,
    })
    notificationStore.show(t('settings.saveSuccess'), 'info')
  } catch { notificationStore.show(t('settings.saveFail'), 'error') }
}

async function runChecks() {
  await configStore.runChecks()
  notificationStore.show(t('settings.checkTriggered'), 'info')
}

async function pickOutputDir() {
  notificationStore.show('请直接在输入框中输入路径', 'info')
}
</script>

<style scoped>
.settings-view { max-width: 600px; margin: 0 auto; width: 100%; }

.page-header { margin-bottom: var(--space-8); }
.page-title { font-size: var(--font-hero); font-weight: 800; letter-spacing: -0.04em; line-height: 1.1; }

/* 设置段落 */
.settings-section { margin-bottom: var(--space-8); }
.section-header {
  font-size: var(--font-xs); font-weight: 600; color: var(--color-text-secondary);
  text-transform: uppercase; letter-spacing: 0.05em;
  margin-bottom: var(--space-4); padding: 0 var(--space-2);
}

.section-card {
  background: var(--color-surface-solid);
  border-radius: var(--radius-xl);
  overflow: hidden;
  box-shadow: var(--shadow-sm);
  border: 0.5px solid var(--color-border);
}

/* 设置行 */
.setting-row {
  display: flex; align-items: center; justify-content: space-between;
  padding: var(--space-6) var(--space-8);
  gap: var(--space-6);
}

.setting-info {
  display: flex; align-items: center; gap: var(--space-5);
  min-width: 0; flex: 1;
}

.setting-icon {
  width: 32px; height: 32px; border-radius: var(--radius-sm);
  background: var(--color-surface-secondary);
  display: flex; align-items: center; justify-content: center;
  flex-shrink: 0;
}
.setting-icon .material-symbols-outlined { font-size: 18px; color: var(--color-text-secondary); }

.setting-text { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
.setting-label { font-size: var(--font-md); font-weight: 600; letter-spacing: -0.01em; }
.setting-value { font-size: var(--font-xs); color: var(--color-text-tertiary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.setting-hint { font-size: var(--font-xs); color: var(--color-text-tertiary); }

.setting-divider { height: 0.5px; background: var(--color-border); margin: 0 var(--space-8); }

.setting-hint-row {
  display: flex; align-items: center; gap: var(--space-2);
  padding: 0 var(--space-8) var(--space-6);
  font-size: var(--font-xs); color: var(--color-text-tertiary);
}
.setting-hint-row .material-symbols-outlined { font-size: 14px; }

/* 分割按钮 (iOS 风格) */
.segmented-control {
  display: inline-flex;
  background: var(--color-surface-secondary);
  border-radius: var(--radius-sm);
  padding: 2px;
  gap: 1px;
}
.segmented-item {
  padding: var(--space-2) var(--space-5);
  border-radius: 6px;
  font-size: var(--font-sm); font-weight: 600;
  color: var(--color-text-secondary);
  transition: all var(--duration-fast) var(--ease-out);
}
.segmented-item.active {
  background: var(--color-surface-solid);
  color: var(--color-text);
  box-shadow: var(--shadow-sm);
}

/* 输入框 */
.text-input {
  width: 100%; max-width: 360px;
  padding: var(--space-3) var(--space-4);
  border: 1.5px solid var(--color-border);
  border-radius: var(--radius-sm);
  font-size: var(--font-sm);
  background: var(--color-surface-secondary);
  color: var(--color-text);
  transition: all var(--duration-fast);
  margin-top: var(--space-2);
}
.text-input:focus { outline: none; border-color: var(--color-primary); box-shadow: 0 0 0 3px var(--color-border-focus); }

.inline-input {
  width: 80px;
  padding: var(--space-3) var(--space-4);
  border: 1.5px solid var(--color-border);
  border-radius: var(--radius-sm);
  font-size: var(--font-sm);
  background: var(--color-surface-secondary);
  color: var(--color-text);
  text-align: center;
  transition: all var(--duration-fast);
}
.inline-input:focus { outline: none; border-color: var(--color-primary); }

.input-suffix {
  display: flex; align-items: center; gap: var(--space-2);
}
.suffix { font-size: var(--font-sm); color: var(--color-text-secondary); font-weight: 500; }

/* 操作按钮 */
.settings-actions {
  display: flex; gap: var(--space-4); margin-top: var(--space-8);
}

.btn {
  display: inline-flex; align-items: center; gap: var(--space-2);
  border-radius: var(--radius-full);
  font-weight: 600;
  transition: all var(--duration-fast) var(--ease-out);
  white-space: nowrap;
}

.btn-large { padding: var(--space-5) var(--space-10); font-size: var(--font-md); flex: 1; justify-content: center; }
.btn-sm { padding: var(--space-3) var(--space-6); font-size: var(--font-xs); }

.btn-primary {
  background: var(--color-primary); color: white;
  box-shadow: 0 2px 8px var(--color-primary-glow);
}
.btn-primary:hover { background: var(--color-primary-hover); transform: translateY(-1px); }

.btn-ghost { background: var(--color-surface-solid); color: var(--color-text); border: 1px solid var(--color-border); }
.btn-ghost:hover { background: var(--color-surface-secondary); }

/* 响应式 */
@media (max-width: 479px) {
  .setting-row { padding: var(--space-5) var(--space-6); flex-wrap: wrap; }
  .settings-actions { flex-direction: column; }
}
</style>
