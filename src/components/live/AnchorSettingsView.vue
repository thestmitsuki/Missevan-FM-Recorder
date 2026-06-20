<template>
  <div class="settings-overlay" @click.self="$emit('close')">
    <div class="settings-panel glass-heavy">
      <!-- 头部 -->
      <div class="panel-header">
        <button class="icon-btn" @click="$emit('close')">
          <span class="material-symbols-outlined">close</span>
        </button>
        <h2 class="panel-title">{{ anchor?.name }}</h2>
      </div>

      <div class="panel-body">
        <!-- 基本信息 -->
        <section class="section">
          <h3 class="section-title">{{ $t('live.settings.anchorInfo') }}</h3>
          <div class="info-cards">
            <div class="info-chip">
              <span class="chip-label">{{ $t('live.settings.id') }}</span>
              <span class="chip-value mono">{{ anchor?.id }}</span>
            </div>
            <div class="info-chip">
              <span class="chip-label">{{ $t('live.settings.name') }}</span>
              <span class="chip-value">{{ anchor?.name }}</span>
            </div>
            <div class="info-chip">
              <span class="chip-label">{{ $t('live.settings.titleLabel') }}</span>
              <span class="chip-value">{{ anchor?.title || '—' }}</span>
            </div>
            <div class="info-chip">
              <span class="chip-label">{{ $t('live.settings.status') }}</span>
              <span class="chip-value">
                <span class="status-badge" :class="anchor?.is_live ? 'live' : 'offline'">
                  <span class="badge-dot"></span>
                  {{ anchor?.is_live ? $t('live.live') : $t('live.offline') }}
                </span>
              </span>
            </div>
          </div>
        </section>

        <div class="section-divider"></div>

        <!-- 主播配置 -->
        <section class="section">
          <h3 class="section-title">{{ $t('live.settings.systemConfig') }}</h3>
          <div class="config-fields">
            <label class="field-wrap">
              <span class="field-label">{{ $t('live.settings.url') }}</span>
              <input v-model="localConfig.url" class="field-input" />
            </label>
            <label class="field-wrap">
              <span class="field-label">{{ $t('live.settings.proxy') }}</span>
              <input v-model="localConfig.proxy" class="field-input" placeholder="http://127.0.0.1:7890" />
            </label>
            <label class="field-wrap">
              <span class="field-label">{{ $t('live.settings.cookie') }}</span>
              <input v-model="localConfig.cookie" class="field-input" type="password" placeholder="留空则不使用" />
            </label>
            <div class="field-row">
              <label class="switch-wrap">
                <span class="field-label">{{ $t('live.settings.enableCheck') }}</span>
                <label class="switch">
                  <input type="checkbox" v-model="localConfig.enable_check" />
                  <span class="switch-slider"></span>
                </label>
              </label>
              <label class="field-wrap field-small">
                <span class="field-label">{{ $t('live.settings.checkInterval') }}(s)</span>
                <input type="number" v-model.number="localConfig.check_interval_secs" class="field-input" min="10" max="600" />
              </label>
            </div>
          </div>
        </section>
      </div>

      <!-- 底部 -->
      <div class="panel-footer">
        <button class="btn btn-ghost btn-danger" @click="confirmDelete">
          <span class="material-symbols-outlined">delete</span>
          {{ $t('common.delete') }}
        </button>
        <div class="footer-right">
          <button class="btn btn-ghost" @click="$emit('close')">{{ $t('common.cancel') }}</button>
          <button class="btn btn-primary" @click="saveAndClose">{{ $t('live.settings.save') }}</button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { reactive, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { useAnchorStore } from '../../stores/anchorStore'
import { useNotificationStore } from '../../stores/notificationStore'
import type { AnchorInfo, AnchorConfig } from '../../types'

const props = defineProps<{ anchor: AnchorInfo | null }>()
const emit = defineEmits<{ (e: 'close'): void; (e: 'settings-changed'): void }>()

const { t } = useI18n()
const anchorStore = useAnchorStore()
const notificationStore = useNotificationStore()

const localConfig = reactive({
  url: '', proxy: '', cookie: '', enable_check: true, check_interval_secs: 30,
})

onMounted(() => {
  if (props.anchor) {
    const c = props.anchor.config || {}
    localConfig.url = c.url || ''
    localConfig.proxy = c.proxy || ''
    localConfig.cookie = c.cookie || ''
    localConfig.enable_check = c.enable_check ?? true
    localConfig.check_interval_secs = c.check_interval_secs || 30
  }
})

async function saveAndClose() {
  if (!props.anchor) return
  try {
    const updated: AnchorConfig = {
      id: props.anchor.id, name: props.anchor.name,
      url: localConfig.url, proxy: localConfig.proxy || null,
      cookie: localConfig.cookie || null, enable_check: localConfig.enable_check,
      check_interval_secs: localConfig.check_interval_secs,
    }
    await anchorStore.updateAnchorConfig(updated)
    emit('settings-changed'); emit('close')
  } catch { notificationStore.show(t('common.error'), 'error') }
}

async function confirmDelete() {
  if (!props.anchor) return
  if (confirm(t('common.confirmDelete'))) {
    await anchorStore.removeAnchor(props.anchor.id); emit('close')
  }
}
</script>

<style scoped>
.settings-overlay {
  position: fixed; inset: 0;
  background: var(--color-overlay);
  z-index: var(--z-modal);
  display: flex; align-items: center; justify-content: center;
}

.settings-panel {
  width: min(520px, calc(100vw - var(--space-12)));
  max-height: 85vh;
  border-radius: var(--radius-2xl);
  box-shadow: var(--shadow-modal), var(--glass-shadow);
  backdrop-filter: blur(60px) saturate(1.6);
  -webkit-backdrop-filter: blur(60px) saturate(1.6);
  display: flex; flex-direction: column; overflow: hidden;
}

/* 头部 */
.panel-header {
  display: flex; align-items: center; gap: var(--space-4);
  padding: var(--space-6) var(--space-8);
  border-bottom: 0.5px solid var(--color-border);
  flex-shrink: 0;
}

.icon-btn {
  width: 30px; height: 30px; border-radius: var(--radius-full);
  display: flex; align-items: center; justify-content: center;
  color: var(--color-text-secondary);
  transition: all var(--duration-fast);
}
.icon-btn:hover { background: var(--color-surface-secondary); color: var(--color-text); }
.icon-btn .material-symbols-outlined { font-size: 18px; }

.panel-title {
  font-size: var(--font-lg); font-weight: 700; letter-spacing: -0.02em;
  flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}

/* 内容 */
.panel-body { flex: 1; overflow-y: auto; padding: var(--space-8); }
.section { margin-bottom: var(--space-6); }
.section-title {
  font-size: var(--font-sm); font-weight: 600; color: var(--color-text-secondary);
  text-transform: uppercase; letter-spacing: 0.05em;
  margin-bottom: var(--space-5);
}
.section-divider { height: 0.5px; background: var(--color-border); margin: var(--space-6) 0; }

/* 信息卡片 */
.info-cards {
  display: grid; grid-template-columns: 1fr 1fr; gap: var(--space-3);
}
.info-chip {
  padding: var(--space-4); border-radius: var(--radius-md);
  background: var(--color-surface-secondary);
  display: flex; flex-direction: column; gap: var(--space-1);
}
.chip-label { font-size: var(--font-xs); color: var(--color-text-secondary); font-weight: 500; }
.chip-value { font-size: var(--font-sm); color: var(--color-text); font-weight: 500; }
.chip-value.mono { font-family: var(--font-mono); font-size: var(--font-xs); }

.status-badge {
  display: inline-flex; align-items: center; gap: var(--space-2);
  padding: var(--space-1) var(--space-3); border-radius: var(--radius-full);
  font-size: var(--font-xs); font-weight: 600;
}
.status-badge.live { background: var(--color-danger-light); color: var(--color-danger); }
.status-badge.offline { background: var(--color-surface-tertiary); color: var(--color-text-tertiary); }
.badge-dot { width: 5px; height: 5px; border-radius: 50%; background: currentColor; }

/* 配置字段 */
.config-fields { display: flex; flex-direction: column; gap: var(--space-5); }
.field-wrap { display: flex; flex-direction: column; gap: var(--space-2); }
.field-label { font-size: var(--font-sm); font-weight: 600; color: var(--color-text); }
.field-input {
  padding: var(--space-4) var(--space-5);
  border: 1.5px solid var(--color-border);
  border-radius: var(--radius-md);
  font-size: var(--font-sm);
  background: var(--color-surface-solid);
  color: var(--color-text);
  transition: all var(--duration-fast);
}
.field-input:focus {
  outline: none; border-color: var(--color-primary);
  box-shadow: 0 0 0 4px var(--color-border-focus);
}
.field-input::placeholder { color: var(--color-text-tertiary); }

.field-row {
  display: flex; gap: var(--space-6); align-items: flex-end;
}
.field-small { flex: 0 0 120px; }

/* Toggle Switch (iOS 风格) */
.switch-wrap {
  display: flex; flex-direction: column; gap: var(--space-3); flex: 1;
}
.switch {
  position: relative; display: inline-block; width: 44px; height: 28px; cursor: pointer;
}
.switch input { opacity: 0; width: 0; height: 0; }
.switch-slider {
  position: absolute; inset: 0;
  background: var(--color-surface-secondary);
  border-radius: var(--radius-full);
  transition: all var(--duration-normal) var(--ease-out);
}
.switch-slider::before {
  content: ''; position: absolute; top: 2px; left: 2px;
  width: 24px; height: 24px; border-radius: 50%;
  background: white; box-shadow: var(--shadow-sm);
  transition: all var(--duration-normal) var(--ease-spring);
}
.switch input:checked + .switch-slider { background: var(--color-success); }
.switch input:checked + .switch-slider::before { transform: translateX(16px); }

/* 底部 */
.panel-footer {
  display: flex; justify-content: space-between; align-items: center;
  padding: var(--space-6) var(--space-8);
  border-top: 0.5px solid var(--color-border);
  flex-shrink: 0;
}
.footer-right { display: flex; gap: var(--space-4); }

/* 按钮 */
.btn {
  display: inline-flex; align-items: center; gap: var(--space-2);
  padding: var(--space-4) var(--space-8);
  border-radius: var(--radius-full);
  font-size: var(--font-sm); font-weight: 600;
  transition: all var(--duration-fast) var(--ease-out);
}
.btn-primary { background: var(--color-primary); color: white; }
.btn-primary:hover { background: var(--color-primary-hover); }
.btn-ghost { background: var(--color-surface-secondary); color: var(--color-text); }
.btn-ghost:hover { background: var(--color-surface-tertiary); }
.btn-danger { color: var(--color-danger); background: transparent; }
.btn-danger:hover { background: var(--color-danger-light); }

@media (max-width: 479px) {
  .info-cards { grid-template-columns: 1fr; }
  .field-row { flex-direction: column; align-items: stretch; }
  .field-small { flex: 1; }
  .panel-footer { flex-direction: column; gap: var(--space-4); }
  .footer-right { width: 100%; }
  .footer-right .btn { flex: 1; justify-content: center; }
}
</style>
