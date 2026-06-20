<template>
  <div class="files-view">
    <div class="page-header">
      <div class="page-header-left">
        <h1 class="page-title">{{ $t('files.title') }}</h1>
        <span class="header-count">{{ audioFiles.length }} 个文件</span>
      </div>
    </div>

    <!-- 搜索 + 排序 -->
    <div class="toolbar">
      <div class="search-box">
        <span class="material-symbols-outlined search-icon">search</span>
        <input v-model="searchQuery" class="search-input" :placeholder="$t('files.searchPlaceholder')" />
        <button v-if="searchQuery" class="search-clear" @click="searchQuery = ''">
          <span class="material-symbols-outlined">close</span>
        </button>
      </div>
      <div class="sort-group">
        <button
          v-for="opt in sortOptions"
          :key="opt.key"
          class="sort-chip"
          :class="{ active: sortBy === opt.key }"
          @click="sortBy = opt.key"
        >
          <span class="material-symbols-outlined">{{ opt.icon }}</span>
          {{ opt.label }}
        </button>
      </div>
    </div>

    <!-- 文件列表 -->
    <div class="list-container">
      <!-- 加载态 -->
      <div v-if="loading" class="state-view">
        <div class="skeleton-list">
          <div v-for="n in 4" :key="n" class="skeleton-item">
            <div class="skeleton skeleton-icon"></div>
            <div class="skeleton-lines">
              <div class="skeleton skeleton-line w-60"></div>
              <div class="skeleton skeleton-line w-30"></div>
            </div>
          </div>
        </div>
      </div>

      <!-- 空状态 -->
      <div v-else-if="audioFiles.length === 0" class="state-view">
        <div class="state-icon-wrap">
          <span class="material-symbols-outlined">{{ searchQuery ? 'search_off' : 'folder_off' }}</span>
        </div>
        <h3 class="state-title">{{ searchQuery ? $t('files.noSearchResults') : $t('files.noFiles') }}</h3>
      </div>

      <!-- 按文件夹分组 -->
      <div v-else v-for="(group, folderName) in groupedFiles" :key="folderName" class="file-group">
        <div class="file-group-header" @click="toggleGroup(folderName)">
          <span class="material-symbols-outlined group-arrow" :class="{ expanded: expandedGroups[folderName] }">
            chevron_right
          </span>
          <span class="group-icon">
            <span class="material-symbols-outlined">folder</span>
          </span>
          <span class="group-name">{{ folderName || $t('files.uncategorized') }}</span>
          <span class="group-count">{{ group.length }} 个文件</span>
        </div>

        <TransitionGroup v-if="expandedGroups[folderName]" name="card" tag="div" class="file-list">
          <div v-for="file in group" :key="file.path" class="file-row">
            <div class="file-row-left">
              <div class="file-icon-wrap">
                <span class="material-symbols-outlined file-icon">audiotrack</span>
              </div>
              <div class="file-meta">
                <span class="file-name">{{ file.name }}</span>
                <span class="file-detail">
                  {{ formatSize(file.size) }}
                  <span class="file-dot">·</span>
                  {{ file.modified }}
                </span>
              </div>
            </div>
            <div class="file-row-actions">
              <button class="row-btn" @click="openFile(file)" :title="$t('files.open')">
                <span class="material-symbols-outlined">play_arrow</span>
              </button>
              <button class="row-btn" @click="openRenameDialog(file)" :title="$t('files.rename')">
                <span class="material-symbols-outlined">edit</span>
              </button>
              <button class="row-btn row-btn-danger" @click="openDeleteDialog(file)" :title="$t('files.delete')">
                <span class="material-symbols-outlined">delete</span>
              </button>
            </div>
          </div>
        </TransitionGroup>
      </div>
    </div>

    <!-- 重命名对话框 -->
    <Teleport to="body">
      <Transition name="modal">
        <div v-if="renameDialog.visible" class="modal-overlay" @click.self="closeRenameDialog">
          <div class="modal-panel glass-heavy">
            <div class="modal-handle"></div>
            <h3 class="modal-title">{{ $t('files.renameDialog.title') }}</h3>
            <input v-model="renameDialog.newName" class="field-input" autofocus @keyup.enter="confirmRename" />
            <div class="modal-actions">
              <button class="btn btn-ghost" @click="closeRenameDialog">{{ $t('files.renameDialog.cancel') }}</button>
              <button class="btn btn-primary" @click="confirmRename">{{ $t('files.renameDialog.confirm') }}</button>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>

    <!-- 删除确认 -->
    <Teleport to="body">
      <Transition name="modal">
        <div v-if="deleteDialog.visible" class="modal-overlay" @click.self="closeDeleteDialog">
          <div class="modal-panel glass-heavy">
            <div class="modal-handle"></div>
            <h3 class="modal-title">{{ $t('files.deleteDialog.title') }}</h3>
            <p class="modal-desc">{{ $t('files.deleteDialog.message', { name: deleteDialog.fileName }) }}</p>
            <div class="modal-actions">
              <button class="btn btn-ghost" @click="closeDeleteDialog">{{ $t('files.deleteDialog.cancel') }}</button>
              <button class="btn btn-danger" @click="confirmDelete">{{ $t('files.deleteDialog.confirm') }}</button>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { invoke } from '@tauri-apps/api/core'
import { useNotificationStore } from '../../stores/notificationStore'
import type { AudioFileInfo } from '../../types'

const { t } = useI18n()
const notificationStore = useNotificationStore()

const audioFiles = ref<AudioFileInfo[]>([])
const loading = ref(false)
const searchQuery = ref('')
const sortBy = ref<'date' | 'name' | 'size'>('date')
const expandedGroups = ref<Record<string, boolean>>({})

const sortOptions = [
  { key: 'date' as const, icon: 'schedule', label: t('files.sortDate') },
  { key: 'name' as const, icon: 'sort_by_alpha', label: t('files.sortName') },
  { key: 'size' as const, icon: 'storage', label: t('files.sortSize') },
]

const renameDialog = ref({ visible: false, file: null as AudioFileInfo | null, newName: '' })
const deleteDialog = ref({ visible: false, file: null as AudioFileInfo | null, fileName: '' })

// 按文件夹分组
const groupedFiles = computed(() => {
  let list = audioFiles.value
  if (searchQuery.value.trim()) {
    const q = searchQuery.value.trim().toLowerCase()
    list = list.filter(f => f.name.toLowerCase().includes(q))
  }
  // 排序
  const sorted = [...list].sort((a, b) => {
    let cmp = 0
    if (sortBy.value === 'name') cmp = a.name.localeCompare(b.name)
    else if (sortBy.value === 'size') cmp = a.size - b.size
    else cmp = a.modified.localeCompare(b.modified)
    return -cmp
  })
  // 分组
  const groups: Record<string, AudioFileInfo[]> = {}
  for (const file of sorted) {
    const folder = file.folder || t('files.uncategorized')
    if (!groups[folder]) groups[folder] = []
    groups[folder].push(file)
  }
  return groups
})

function toggleGroup(name: string) {
  expandedGroups.value[name] = !expandedGroups.value[name]
}

function expandAll() {
  for (const name of Object.keys(groupedFiles.value)) {
    expandedGroups.value[name] = true
  }
}

async function loadFiles() {
  loading.value = true
  try {
    audioFiles.value = await invoke<AudioFileInfo[]>('get_audio_files')
    expandAll()
  } catch (err) {
    notificationStore.show(t('common.error'), 'error')
  } finally { loading.value = false }
}

function openFile(file: AudioFileInfo) {
  invoke('open_with_system', { path: file.path }).catch(() => notificationStore.show(t('common.error'), 'error'))
}

function formatSize(bytes: number) {
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB'
  return (bytes / (1024 * 1024)).toFixed(2) + ' MB'
}

// 重命名
function openRenameDialog(file: AudioFileInfo) {
  const dot = file.name.lastIndexOf('.')
  renameDialog.value = { visible: true, file, newName: dot === -1 ? file.name : file.name.slice(0, dot) }
}
function closeRenameDialog() { renameDialog.value.visible = false; renameDialog.value.file = null }
async function confirmRename() {
  const { file, newName } = renameDialog.value
  if (!file || !newName.trim()) { notificationStore.show(t('files.renameDialog.emptyError'), 'warning'); return }
  const dot = file.name.lastIndexOf('.')
  const ext = dot === -1 ? '' : file.name.slice(dot)
  try {
    await invoke('rename_audio_file', { oldPath: file.path, newName: newName.trim() + ext })
    await loadFiles(); closeRenameDialog(); notificationStore.show(t('files.renameDialog.success'), 'info')
  } catch { notificationStore.show(t('files.renameDialog.fail'), 'error') }
}

// 删除
function openDeleteDialog(file: AudioFileInfo) { deleteDialog.value = { visible: true, file, fileName: file.name } }
function closeDeleteDialog() { deleteDialog.value.visible = false; deleteDialog.value.file = null }
async function confirmDelete() {
  const { file } = deleteDialog.value
  if (!file) return
  try {
    await invoke('delete_audio_file', { path: file.path })
    await loadFiles(); closeDeleteDialog(); notificationStore.show(t('files.deleteDialog.success'), 'info')
  } catch { notificationStore.show(t('files.deleteDialog.fail'), 'error') }
}

onMounted(loadFiles)
</script>

<style scoped>
.files-view { max-width: var(--content-max-width); margin: 0 auto; width: 100%; }
.page-header { margin-bottom: var(--space-8); }
.page-header-left { display: flex; align-items: baseline; gap: var(--space-4); }
.page-title { font-size: var(--font-hero); font-weight: 800; letter-spacing: -0.04em; line-height: 1.1; }
.header-count { font-size: var(--font-sm); color: var(--color-text-secondary); font-weight: 500; }

/* 工具栏 */
.toolbar { display: flex; gap: var(--space-4); margin-bottom: var(--space-6); flex-wrap: wrap; }
.search-box { flex: 1; position: relative; min-width: 180px; }
.search-icon { position: absolute; left: var(--space-5); top: 50%; transform: translateY(-50%); color: var(--color-text-tertiary); font-size: 18px !important; pointer-events: none; }
.search-input { width: 100%; padding: var(--space-4) var(--space-8) var(--space-4) var(--space-12); border: 1.5px solid var(--color-border); border-radius: var(--radius-lg); font-size: var(--font-sm); background: var(--color-surface-solid); color: var(--color-text); transition: all var(--duration-fast); }
.search-input:focus { outline: none; border-color: var(--color-primary); box-shadow: 0 0 0 4px var(--color-border-focus); }
.search-clear { position: absolute; right: var(--space-3); top: 50%; transform: translateY(-50%); width: 26px; height: 26px; border-radius: var(--radius-full); display: flex; align-items: center; justify-content: center; color: var(--color-text-tertiary); transition: all var(--duration-fast); }
.search-clear:hover { background: var(--color-surface-secondary); color: var(--color-text); }
.search-clear .material-symbols-outlined { font-size: 16px; }
.sort-group { display: flex; gap: var(--space-2); flex-wrap: wrap; }
.sort-chip { display: inline-flex; align-items: center; gap: var(--space-1); padding: var(--space-3) var(--space-5); border-radius: var(--radius-full); font-size: var(--font-xs); font-weight: 600; color: var(--color-text-secondary); background: var(--color-surface-solid); border: 1px solid var(--color-border); transition: all var(--duration-fast); }
.sort-chip:hover { background: var(--color-surface-secondary); }
.sort-chip.active { background: var(--color-primary-light); color: var(--color-primary); border-color: transparent; }
.sort-chip .material-symbols-outlined { font-size: 14px; }

/* 列表容器 */
.list-container { background: var(--color-surface-solid); border-radius: var(--radius-xl); overflow: hidden; box-shadow: var(--shadow-sm); border: 0.5px solid var(--color-border); }

/* 文件分组 */
.file-group:not(:last-child) { border-bottom: 0.5px solid var(--color-border); }

.file-group-header {
  display: flex; align-items: center; gap: var(--space-4);
  padding: var(--space-4) var(--space-6);
  cursor: pointer;
  transition: background var(--duration-fast);
  user-select: none;
}
.file-group-header:hover { background: var(--color-surface-secondary); }

.group-arrow { font-size: 18px !important; color: var(--color-text-tertiary); transition: transform var(--duration-fast); }
.group-arrow.expanded { transform: rotate(90deg); }

.group-icon .material-symbols-outlined { font-size: 18px; color: var(--color-primary); }
.group-name { font-size: var(--font-sm); font-weight: 600; flex: 1; }
.group-count { font-size: var(--font-xs); color: var(--color-text-tertiary); }

.file-list { display: flex; flex-direction: column; }
.file-row {
  display: flex; justify-content: space-between; align-items: center;
  padding: var(--space-4) var(--space-8) var(--space-4) var(--space-14);
  border-top: 0.5px solid var(--color-border-light);
  transition: background var(--duration-fast);
}
.file-row:hover { background: var(--color-surface-secondary); }
.file-row-left { display: flex; align-items: center; gap: var(--space-4); flex: 1; min-width: 0; }
.file-icon-wrap { width: 32px; height: 32px; border-radius: var(--radius-sm); background: var(--color-primary-light); display: flex; align-items: center; justify-content: center; flex-shrink: 0; }
.file-icon { color: var(--color-primary) !important; font-size: 16px !important; }
.file-meta { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
.file-name { font-weight: 600; font-size: var(--font-sm); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.file-detail { font-size: var(--font-xs); color: var(--color-text-secondary); }
.file-dot { margin: 0 4px; }
.file-row-actions { display: flex; gap: 2px; flex-shrink: 0; margin-left: var(--space-4); }
.row-btn { width: 30px; height: 30px; border-radius: var(--radius-sm); display: flex; align-items: center; justify-content: center; color: var(--color-text-tertiary); transition: all var(--duration-fast); }
.row-btn:hover { background: var(--color-primary-light); color: var(--color-primary); }
.row-btn-danger:hover { background: var(--color-danger-light); color: var(--color-danger); }
.row-btn .material-symbols-outlined { font-size: 16px; }

/* 状态 */
.state-view { padding: var(--space-24) var(--space-8); display: flex; flex-direction: column; align-items: center; gap: var(--space-5); }
.state-icon-wrap { width: 64px; height: 64px; border-radius: var(--radius-full); background: var(--color-surface-secondary); display: flex; align-items: center; justify-content: center; }
.state-icon-wrap .material-symbols-outlined { font-size: 28px; color: var(--color-text-tertiary); }
.state-title { font-size: var(--font-md); font-weight: 600; color: var(--color-text-secondary); }

/* Skeleton */
.skeleton-list { padding: var(--space-6) var(--space-8); display: flex; flex-direction: column; gap: var(--space-6); }
.skeleton-item { display: flex; align-items: center; gap: var(--space-5); }
.skeleton-icon { width: 32px; height: 32px; border-radius: var(--radius-sm); flex-shrink: 0; }
.skeleton-lines { display: flex; flex-direction: column; gap: var(--space-2); flex: 1; }
.skeleton-line { height: 12px; border-radius: 4px; }
.w-60 { width: 60%; } .w-30 { width: 30%; }

/* Modal */
.modal-overlay { position: fixed; inset: 0; background: var(--color-overlay); z-index: var(--z-modal); display: flex; align-items: center; justify-content: center; }
.modal-panel { width: min(380px, calc(100vw - var(--space-12))); padding: var(--space-10) var(--space-9); border-radius: var(--radius-2xl); box-shadow: var(--shadow-modal), var(--glass-shadow); }
.modal-handle { width: 36px; height: 4px; border-radius: 2px; background: var(--color-text-tertiary); opacity: 0.3; margin: 0 auto var(--space-6); }
.modal-title { font-size: var(--font-xl); font-weight: 700; letter-spacing: -0.02em; margin-bottom: var(--space-6); }
.modal-desc { font-size: var(--font-sm); color: var(--color-text); margin-bottom: var(--space-6); line-height: 1.5; }
.field-input { width: 100%; padding: var(--space-4) var(--space-5); border: 1.5px solid var(--color-border); border-radius: var(--radius-md); font-size: var(--font-sm); background: var(--color-surface-solid); color: var(--color-text); margin-bottom: var(--space-6); }
.field-input:focus { outline: none; border-color: var(--color-primary); box-shadow: 0 0 0 4px var(--color-border-focus); }
.modal-actions { display: flex; justify-content: flex-end; gap: var(--space-4); }

.btn { display: inline-flex; align-items: center; gap: var(--space-2); padding: var(--space-4) var(--space-8); border-radius: var(--radius-full); font-size: var(--font-sm); font-weight: 600; transition: all var(--duration-fast) var(--ease-out); }
.btn-primary { background: var(--color-primary); color: white; }
.btn-primary:hover { background: var(--color-primary-hover); }
.btn-ghost { background: var(--color-surface-secondary); color: var(--color-text); }
.btn-ghost:hover { background: var(--color-surface-tertiary); }
.btn-danger { background: var(--color-danger); color: white; }
.btn-danger:hover { background: #d63031; }

@media (max-width: 479px) {
  .toolbar { flex-direction: column; }
  .file-row { padding: var(--space-3) var(--space-5) var(--space-3) var(--space-10); }
}
</style>
