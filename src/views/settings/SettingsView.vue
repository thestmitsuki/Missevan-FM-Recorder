<script setup lang="ts">
/**
 * 设置页（规格「设置页面功能规格」）——左导航 8 分类 + 右内容面板
 *
 * 交互模型（全部操作保存后生效）：
 * - 表单 = configStore.config 的深拷贝（reactive）+ 三个暂存字段（locale/theme/
 *   appearance 纯前端偏好），字段与后端 GlobalConfig 逐字对齐（snake_case 透传）。
 * - 任何修改 → dirty；切换分类/离开设置页时弹「是否保存更改？」（保存/不保存/取消）；
 * - 「保存更改」按钮 + 切换分类保存；autostart/show_tray/close_behavior 即时生效（M3 修复：
 *   set_autostart / reconcile_tray / 关闭事件实时读配置）；log_level 同样即时生效
 *   （U5：save_config 落盘后经 LogLevelReload 热更新，无需重启，无「重启生效」横幅）；
 * - 语言/主题/外观均为暂存态：save() 成功时统一提交（setLocale / themeStore.setMode /
 *   appearanceStore.update），即「保存后生效」；快捷键（H2）当前版本功能未启用，
 *   仅作占位展示、不再写入落盘配置（normalizeConfig 剔除）；离开守卫
 *   （onBeforeRouteLeave）保证未保存更改不会静默丢弃。
 * - 输入实时校验（路径/数字范围/必填 → 红边框+提示，errors 按分类分发）；
 * - 底部「恢复默认值」（当前分类）+「全部恢复默认」（二次确认）——全部为暂存态。
 *
 * 后端命令：export_config / import_config / reset_config / set_autostart 全部接线。
 */
import { computed, onMounted, reactive, ref, watch } from "vue";
import { onBeforeRouteLeave } from "vue-router";
import { useI18n } from "vue-i18n";
import { Info } from "@lucide/vue";
import type { GlobalConfig } from "@/types";
import { useConfigStore, DEFAULT_CONFIG } from "@/stores/configStore";
import {
    DEFAULT_APPEARANCE,
    useAppearanceStore,
} from "@/stores/appearanceStore";
import { useThemeStore } from "@/stores/themeStore";
import { isLinuxPlatform } from "@/services/platform";
import { setLocale, type AppLocale } from "@/locales";
import { api } from "@/services/api";
import { CATEGORIES, type CategoryId } from "./sections";
import type { SettingsCategory } from "./sections";
import {
    cloneForm,
    normalizeConfig,
    validateAll,
    type SectionErrors,
    type SettingsForm,
    type I18nT,
} from "./validation";
import SideNav, { type SideNavItem } from "@/components/common/SideNav.vue";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
    AlertDialog,
    AlertDialogCancel,
    AlertDialogContent,
    AlertDialogDescription,
    AlertDialogFooter,
    AlertDialogHeader,
    AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import ConfirmDialog from "@/components/common/ConfirmDialog.vue";
import AboutDialog from "./AboutDialog.vue";

const configStore = useConfigStore();
const appearanceStore = useAppearanceStore();
const themeStore = useThemeStore();
const { t, locale } = useI18n();

// ── 状态 ──
const activeId = ref<CategoryId>("general");
const loading = ref(true);
const saving = ref(false);
const errorMsg = ref<string | null>(null);
/** 开机自启（注册表）单独失败警告：不阻断保存（I3） */
const autostartWarn = ref<string | null>(null);
const savedFlash = ref(false);

/** 最近一次已保存（或已加载）的表单快照：dirty = 当前表单 ≠ 快照 */
let baseline: SettingsForm;
const form = reactive<SettingsForm>(formFromStore());
baseline = snapshotFrom(form);

/** 当前表单相对快照是否被修改（分类切换/离开守卫/导航角标使用；需先声明） */
const dirty = ref(false);

/** 关于对话框（规格 §2.1：从设置页「关于」入口打开） */
const aboutOpen = ref(false);

// ── 未保存提示（切换分类 / 离开设置页共用同一弹窗） ──
const unsavedOpen = ref(false);
const pendingCategory = ref<CategoryId | null>(null);

// ── 离开设置页守卫（C3：未保存更改不得静默丢弃） ──
/** true = 弹窗由「离开设置页」触发（区别于分类切换） */
const leavePending = ref(false);
/** 路由守卫挂起的导航确认器：true 继续导航 / false 停留 */
let leaveResolve: ((ok: boolean) => void) | null = null;

onBeforeRouteLeave(() => {
    if (!dirty.value) return true;
    leavePending.value = true;
    unsavedOpen.value = true;
    return new Promise<boolean>((resolve) => {
        leaveResolve = resolve;
    });
});

function settleLeave(ok: boolean) {
    const fn = leaveResolve;
    leaveResolve = null;
    leavePending.value = false;
    fn?.(ok);
}

//数据清理
function stableStringify(obj: unknown): string {
    return JSON.stringify(obj, (_, value) => {
        if (value && typeof value === "object" && !Array.isArray(value)) {
            // 将对象的键排序后重新构建，保证输出顺序一致
            return Object.keys(value)
                .sort()
                .reduce(
                    (acc, key) => {
                        acc[key] = value[key];
                        return acc;
                    },
                    {} as Record<string, unknown>,
                );
        }
        return value;
    });
}

// ── 表单模型：GlobalConfig 深拷贝 + 暂存字段（locale/theme/appearance） ──

/** 当前「纯前端偏好」暂存值（语言/主题/外观；保存时统一提交生效，I6） */
function stagedExtras(): Pick<SettingsForm, "locale" | "theme" | "appearance"> {
    return {
        locale: (locale.value as AppLocale) ?? "zh-CN",
        theme: themeStore.mode,
        appearance: { ...appearanceStore.prefs },
    };
}

/** 从后端配置 + 当前偏好构建全量表单（onMounted / 导入回填用） */
function formFromStore(): SettingsForm {
    return { ...cloneForm(configStore.config), ...stagedExtras() };
}

/** 深拷贝表单快照（含暂存字段；用于 baseline / 放弃修改） */
function snapshotFrom(src: SettingsForm): SettingsForm {
    return {
        ...cloneForm(src),
        locale: src.locale,
        theme: src.theme,
        appearance: { ...src.appearance },
    };
}

// ── 全部恢复默认确认 ──
const restoreAllOpen = ref(false);

// ── 重置所有设置（后端 reset_config：删配置 + 重启）确认 ──
const resetConfigOpen = ref(false);

// ── 导入（后端 import_config：replace/merge） ──
const importFileInput = ref<HTMLInputElement | null>(null);
const importMode = ref<"replace" | "merge">("merge");
const importModeOpen = ref(false);
/** 导入文件的原始 JSON 文本（合法 JSON 对象校验通过后交后端解析/校验） */
const pendingImportText = ref<string | null>(null);
/** 后端操作成功后的提示（导入成功重启生效 / 重置即将重启） */
const restartMsg = ref<string | null>(null);

const activeCategory = computed(
    () => CATEGORIES.find((c) => c.id === activeId.value) as SettingsCategory,
);

// ── 左导航（SideNav 通用组件，配置式）──
const navItems = computed<SideNavItem[]>(() =>
    CATEGORIES.map((cat) => ({
        id: cat.id,
        label: t(cat.labelKey),
        icon: cat.icon,
        badge: dirty.value,
    })),
);

const navFooterItems = computed<SideNavItem[]>(() => [
    { id: "about", label: t("about.title"), icon: Info },
]);

/** SideNav select：'about' 打开关于对话框，其余走分类切换（含未保存守卫） */
function onNavSelect(id: string) {
    if (id === "about") {
        aboutOpen.value = true;
        return;
    }
    selectCategory(id as CategoryId);
}

const hasErrors = ref(false);

// 各分类错误（key = 字段名）
const allErrors = reactive<Record<CategoryId, SectionErrors>>({
    general: {},
    recording: {},
    files: {},
    network: {},
    notification: {},
    appearance: {},
    shortcuts: {},
    advanced: {},
});

function recompute() {
    dirty.value = stableStringify(form) !== stableStringify(baseline);
    const tFn = t as I18nT;
    const all = validateAll(form, tFn);
    for (const id of Object.keys(all) as CategoryId[]) {
        // 就地更新（保持响应式）
        Object.assign(allErrors[id], all[id]);
        for (const k of Object.keys(allErrors[id])) {
            if (!(k in all[id])) delete allErrors[id][k];
        }
    }
    hasErrors.value = Object.values(allErrors).some(
        (e) => Object.keys(e).length > 0,
    );
}

/** 以 Record 视角读写表单/默认配置（用于按字段名批量操作） */
const formRecord = () => form as unknown as Record<string, unknown>;
const defaultRecord = () =>
    DEFAULT_CONFIG as unknown as Record<string, unknown>;

watch(form, recompute, { deep: true });

const errorsFor = (id: CategoryId) => allErrors[id];

// ── 分类切换 ──
function selectCategory(id: CategoryId) {
    if (id === activeId.value) return;
    if (dirty.value) {
        pendingCategory.value = id;
        unsavedOpen.value = true;
        return;
    }
    activeId.value = id;
}

function applyPending() {
    if (pendingCategory.value) {
        activeId.value = pendingCategory.value;
        pendingCategory.value = null;
    }
    unsavedOpen.value = false;
}

async function onUnsavedSave() {
    unsavedOpen.value = false;
    const ok = await save();
    if (ok) {
        if (leavePending.value) settleLeave(true);
        else applyPending();
    } else if (leavePending.value) {
        // 保存失败：留在设置页（错误横幅提示），导航中止
        settleLeave(false);
    } else {
        pendingCategory.value = null; // 保存失败：留在当前分类
    }
}

function onUnsavedDiscard() {
    unsavedOpen.value = false;
    if (leavePending.value) {
        settleLeave(true); // 不保存，直接离开（表单随组件销毁，无需回滚）
    } else {
        Object.assign(form, snapshotFrom(baseline));
        applyPending();
    }
}

function onUnsavedCancel() {
    unsavedOpen.value = false;
    if (leavePending.value) settleLeave(false);
    else pendingCategory.value = null;
}

// ── 保存 ──
async function save(): Promise<boolean> {
    if (hasErrors.value) {
        errorMsg.value = t("settings.errors.invalidFields");
        // 跳到第一个有错误的分类
        const first = CATEGORIES.find(
            (c) => Object.keys(allErrors[c.id]).length > 0,
        );
        if (first) activeId.value = first.id;
        return false;
    }
    saving.value = true;
    errorMsg.value = null;
    autostartWarn.value = null;
    restartMsg.value = null; // 新保存动作清除旧的后端操作提示
    try {
        // Linux 未集成系统托盘（决策 #2）：保存时强制 close_behavior=exit，
        // 保证落盘值语义正确（后端 decide_close_action 在 Linux 上恒回退 Exit）
        if (isLinuxPlatform() && form.close_behavior === "tray") {
            form.close_behavior = "exit";
        }
        configStore.updateConfig(normalizeConfig(form));
        // 核心配置先落盘（失败则中止保存）
        await configStore.saveConfig();
        // 开机自启（注册表）：仅在值变化时调用；失败仅提示该项，不阻断保存（I3）——
        // 避免注册表写失败导致其它字段全部存不上
        if (form.autostart !== baseline.autostart) {
            try {
                await api.setAutostart(form.autostart);
            } catch (e) {
                autostartWarn.value = t("settings.errors.autostartFailed", {
                    error: String(e),
                });
            }
        }
        // 暂存的语言/主题/外观提交生效（I6：「所有操作保存后生效」）
        applyStagedPrefs();
        baseline = snapshotFrom(form);
        recompute(); // C2：baseline 更新后刷新 dirty/校验/重启提示（保存后不再显示「未保存」）
        savedFlash.value = true;
        setTimeout(() => (savedFlash.value = false), 2500);
        return true;
    } catch (e) {
        errorMsg.value = t("settings.errors.saveFailed", { error: String(e) });
        return false;
    } finally {
        saving.value = false;
    }
}

/** 提交暂存的纯前端偏好（语言/主题/外观 → 应用 + localStorage 持久化） */
function applyStagedPrefs() {
    if (form.locale !== (locale.value as AppLocale)) setLocale(form.locale);
    if (form.theme !== themeStore.mode) themeStore.setMode(form.theme);
    if (
        JSON.stringify(form.appearance) !==
        JSON.stringify(appearanceStore.prefs)
    ) {
        appearanceStore.update(form.appearance);
    }
}

// ── 恢复默认 ──
/**
 * 单字段恢复默认值（表单形态归一）：DEFAULT_CONFIG.ffmpeg_path 为 null（= 自动探测），
 * SettingsForm 归一为 string（"" = 自动探测）——直接写入会把 null 注入表单
 * （Critical-1：validateAll/normalizeConfig 的 trim 会抛 TypeError），此处就地归一。
 */
function resetFieldToDefault(f: keyof GlobalConfig) {
    const v = defaultRecord()[f];
    formRecord()[f] = f === "ffmpeg_path" && v === null ? "" : v;
}

function restoreCurrentCategory() {
    const cat = activeCategory.value;
    if (cat.id === "appearance") {
        // 外观：暂存默认值（保存后生效，与其它分类一致；主题默认 = 跟随系统）
        form.theme = "system";
        form.appearance = { ...DEFAULT_APPEARANCE };
    } else if (cat.id === "shortcuts") {
        // 快捷键（H2）：功能未启用，无写入动作——恢复默认不再触碰 form.shortcuts，
        // 避免把默认键位带入保存链路（normalizeConfig 已剔除该字段，双保险）
        // 分类无归属字段（fields: []），落入下方 for 循环同样为无操作
    } else {
        for (const f of cat.fields) {
            resetFieldToDefault(f);
        }
    }
}

function restoreAll() {
    for (const cat of CATEGORIES) {
        if (cat.id === "appearance") {
            form.theme = "system";
            form.appearance = { ...DEFAULT_APPEARANCE };
        } else if (cat.id === "shortcuts") {
            // 快捷键（H2）：功能未启用，恢复默认不写入（见 restoreCurrentCategory 说明）
        } else {
            for (const f of cat.fields) {
                resetFieldToDefault(f);
            }
        }
    }
    restoreAllOpen.value = false;
}

// ── 输出目录浏览（既有接口 pick_output_dir） ──
async function browseOutputDir() {
    const dir = await configStore.pickOutputDir();
    if (dir) form.output_dir = dir;
}

// ── FFmpeg/ffprobe 文件浏览（Tauri dialog 插件，capabilities 已补 dialog:default） ──
async function browseExecutable(kind: "ffmpeg" | "ffprobe") {
    try {
        const path = await api.openExecutableDialog();
        if (path) {
            if (kind === "ffmpeg") form.ffmpeg_path = path;
            else form.ffprobe_path = path;
        }
    } catch (e) {
        errorMsg.value = t("settings.errors.browseFailed", {
            error: String(e),
        });
    }
}

// ── 导入配置（后端 import_config：文件选择器读取 JSON → 替换/合并 → 落盘） ──
function onImportClick() {
    importFileInput.value?.click();
}

async function onImportFileChanged(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    input.value = ""; // 允许重复选择同一文件
    if (!file) return;
    try {
        const text = await file.text();
        const parsed = JSON.parse(text) as Record<string, unknown>;
        if (typeof parsed !== "object" || parsed === null)
            throw new Error("not-object");
        pendingImportText.value = text;
        importModeOpen.value = true;
    } catch {
        errorMsg.value = t("settings.errors.importFailed");
    }
}

async function applyImport(mode: "replace" | "merge") {
    importModeOpen.value = false;
    const text = pendingImportText.value;
    pendingImportText.value = null;
    if (!text) return;
    saving.value = true;
    errorMsg.value = null;
    try {
        // 后端解析/校验并落盘（含主播：merge 按 id 去重合并、重复 id 跳过保留本地；
        // replace 全替换）。字段白名单、ffmpeg_path null 归一等由后端统一处理
        await api.importConfig(text, mode);
        // 重新拉取后端配置回填表单（导入已写盘；主播列表在 Live 页加载时同步）
        await configStore.fetchConfig();
        Object.assign(form, formFromStore());
        baseline = snapshotFrom(form);
        recompute();
        // 部分字段（log_level 等）重启后完全生效
        restartMsg.value = t("settings.advanced.importSuccess");
        savedFlash.value = true;
        setTimeout(() => (savedFlash.value = false), 2500);
    } catch (e) {
        errorMsg.value = t("settings.errors.importFailedDetail", {
            error: String(e),
        });
    } finally {
        saving.value = false;
    }
}

// ── 重置所有设置（后端 reset_config：删配置 + 重启应用） ──
function onResetConfigClick() {
    resetConfigOpen.value = true;
}

function confirmResetConfig() {
    resetConfigOpen.value = false;
    // reset_config 成功后 app.restart() 不会返回——命令不 resolve，不 await；
    // 仅捕获启动前错误（如删除配置失败）
    api.resetConfig().catch((e) => {
        errorMsg.value = t("settings.errors.saveFailed", { error: String(e) });
    });
    restartMsg.value = t("settings.advanced.resetRestarting");
}

// ── 全部恢复默认（客户端表单级：恢复默认值，需保存后生效） ──
function onResetAll() {
    restoreAllOpen.value = true;
}

// ── 初始化 ──
// 在 onMounted 中临时关闭 watch，一次性完成表单填充和基线设置后再打开
const pauseWatch = ref(false);

watch(
    form,
    () => {
        if (pauseWatch.value) return;
        recompute();
    },
    { deep: true },
);

onMounted(async () => {
    try {
        await configStore.fetchConfig();
    } catch {
        /* 使用默认值 */
    }
    pauseWatch.value = true;
    Object.assign(form, formFromStore());
    // Linux 未集成系统托盘：表单中历史遗留的 tray 值归一为 exit，
    // 避免禁用状态下仍显示选中（在基线快照前处理，不产生 dirty）
    if (isLinuxPlatform() && form.close_behavior === "tray") {
        form.close_behavior = "exit";
    }
    baseline = snapshotFrom(form);
    pauseWatch.value = false;
    recompute();
    loading.value = false;
});
</script>

<template>
    <div class="settings-layout" v-if="!loading">
        <!-- ── 左导航：8 分类（SideNav 通用组件，贴左全高） ── -->
        <SideNav
            :title="t('settings.title')"
            :items="navItems"
            :footer-items="navFooterItems"
            :active-id="activeId"
            @select="onNavSelect"
        />

        <!-- ── 右内容面板 ── -->
        <section class="settings-panel">
            <header class="settings-header">
                <div class="min-w-0">
                    <h2 class="settings-title">
                        {{ t(activeCategory.labelKey) }}
                    </h2>
                    <p class="mt-0.5 text-sm text-muted-foreground">
                        {{ t(activeCategory.descKey) }}
                    </p>
                </div>
                <div class="flex shrink-0 items-center gap-2">
                    <Badge
                        v-if="dirty"
                        variant="secondary"
                        class="bg-primary/10 text-primary"
                    >
                        {{ t("settings.unsavedBadge") }}
                    </Badge>
                    <Button
                        :disabled="saving || (!dirty && !hasErrors)"
                        @click="save"
                    >
                        {{
                            saving
                                ? t("settings.saving")
                                : t("settings.saveChanges")
                        }}
                    </Button>
                </div>
            </header>

            <!-- 错误 / 成功反馈 -->
            <div
                v-if="errorMsg"
                class="settings-banner settings-banner-error"
                role="alert"
            >
                {{ errorMsg }}
            </div>
            <!-- 开机自启（注册表）单独失败：保存已成功，仅提示该项（I3） -->
            <div
                v-if="autostartWarn"
                class="settings-banner settings-banner-warn"
                role="alert"
            >
                {{ autostartWarn }}
            </div>
            <div
                v-if="savedFlash"
                class="settings-banner settings-banner-success"
                role="status"
            >
                {{ t("settings.saved") }}
            </div>
            <!-- 后端操作反馈（导入成功重启生效 / 重置即将重启） -->
            <div
                v-if="restartMsg"
                class="settings-banner settings-banner-info"
                role="status"
            >
                {{ restartMsg }}
            </div>

            <!-- 当前分类内容（page-scroll 保留作滚动容器） -->
            <div class="settings-section-scroll page-scroll">
                <component
                    :is="activeCategory.component"
                    :config="form"
                    :errors="errorsFor(activeId)"
                    @browse-output-dir="browseOutputDir"
                    @browse="
                        (kind: unknown) =>
                            browseExecutable(
                                kind === 'ffprobe' ? 'ffprobe' : 'ffmpeg',
                            )
                    "
                    @import-config="onImportClick"
                    @reset-config="onResetConfigClick"
                />
            </div>

            <!-- 底部操作 -->
            <footer class="settings-footer">
                <Button variant="outline" @click="restoreCurrentCategory">
                    {{ t("settings.restoreCategory") }}
                </Button>
                <Button
                    variant="outline"
                    class="text-destructive hover:text-destructive"
                    @click="onResetAll"
                >
                    {{ t("settings.restoreAll") }}
                </Button>
            </footer>
        </section>
    </div>

    <!-- 加载态 -->
    <div
        v-else
        class="flex h-full items-center justify-center text-sm text-muted-foreground"
    >
        {{ t("settings.loading") }}
    </div>

    <!-- 未保存提示：是否保存更改？ -->
    <AlertDialog
        :open="unsavedOpen"
        @update:open="(v) => !v && onUnsavedCancel()"
    >
        <AlertDialogContent class="max-w-sm">
            <AlertDialogHeader>
                <AlertDialogTitle>{{
                    t("settings.unsavedTitle")
                }}</AlertDialogTitle>
                <AlertDialogDescription>{{
                    t("settings.unsavedMessage")
                }}</AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
                <AlertDialogCancel>{{ t("common.cancel") }}</AlertDialogCancel>
                <Button
                    variant="outline"
                    :disabled="saving"
                    @click="onUnsavedDiscard"
                >
                    {{ t("settings.discard") }}
                </Button>
                <Button :disabled="saving" @click="onUnsavedSave">
                    {{ t("settings.saveChanges") }}
                </Button>
            </AlertDialogFooter>
        </AlertDialogContent>
    </AlertDialog>

    <!-- 全部恢复默认（二次确认） -->
    <ConfirmDialog
        :open="restoreAllOpen"
        :title="t('settings.restoreAllTitle')"
        :message="t('settings.restoreAllMessage')"
        :confirm-text="t('settings.restoreAll')"
        :cancel-text="t('common.cancel')"
        destructive
        @confirm="restoreAll"
        @cancel="restoreAllOpen = false"
    />

    <!-- 重置所有设置（后端 reset_config：删配置 + 重启，二次确认） -->
    <ConfirmDialog
        :open="resetConfigOpen"
        :title="t('settings.advanced.resetBtn')"
        :message="t('settings.advanced.resetConfirmMessage')"
        :confirm-text="t('settings.advanced.resetConfirm')"
        :cancel-text="t('common.cancel')"
        destructive
        @confirm="confirmResetConfig"
        @cancel="resetConfigOpen = false"
    />

    <!-- 导入模式选择：替换 / 合并 -->
    <AlertDialog
        :open="importModeOpen"
        @update:open="(v) => !v && (importModeOpen = false)"
    >
        <AlertDialogContent class="max-w-sm">
            <AlertDialogHeader>
                <AlertDialogTitle>{{
                    t("settings.advanced.importModeTitle")
                }}</AlertDialogTitle>
                <AlertDialogDescription>
                    {{ t("settings.advanced.importModeDesc") }}
                </AlertDialogDescription>
            </AlertDialogHeader>
            <div class="flex flex-col gap-2 px-6">
                <label class="flex cursor-pointer items-center gap-2 text-sm">
                    <input
                        v-model="importMode"
                        type="radio"
                        value="merge"
                        class="size-4 accent-primary"
                    />
                    {{ t("settings.advanced.importModeMerge") }}
                </label>
                <label class="flex cursor-pointer items-center gap-2 text-sm">
                    <input
                        v-model="importMode"
                        type="radio"
                        value="replace"
                        class="size-4 accent-primary"
                    />
                    {{ t("settings.advanced.importModeReplace") }}
                </label>
            </div>
            <AlertDialogFooter>
                <AlertDialogCancel>{{ t("common.cancel") }}</AlertDialogCancel>
                <Button @click="applyImport(importMode)">{{
                    t("common.confirm")
                }}</Button>
            </AlertDialogFooter>
        </AlertDialogContent>
    </AlertDialog>

    <!-- 隐藏的文件选择器（导入） -->
    <input
        ref="importFileInput"
        type="file"
        accept=".json,application/json"
        class="hidden"
        @change="onImportFileChanged"
    />

    <!-- 关于对话框（规格 §2.1：应用名/版本/构建日期/依赖/许可证/法律声明/检查更新） -->
    <AboutDialog v-model:open="aboutOpen" />
</template>

<style scoped>
/* 布局：左 SideNav（贴左全高，样式在 SideNav.vue）+ 右面板 */
.settings-layout {
    display: flex;
    height: 100%;
    min-height: 0;
    flex: 1;
}

/* ── 右面板（居中限宽，不贴边） ── */
.settings-panel {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
    max-width: 1200px;
    margin: 0 auto;
}
.settings-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    padding: 20px 24px 16px;
}
.settings-title {
    font-size: 1.25rem;
    font-weight: 600;
    line-height: 1.3;
}
.settings-section-scroll {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
    padding: 16px 24px;
}
.settings-banner {
    margin: 12px 24px 0;
    padding: 10px 14px;
    border-radius: 8px;
    font-size: 0.8125rem;
}
.settings-banner-info {
    background: color-mix(in oklab, var(--primary) 8%, transparent);
    color: var(--primary);
    border: 1px solid color-mix(in oklab, var(--primary) 20%, transparent);
}
.settings-banner-error {
    background: color-mix(in oklab, var(--destructive) 8%, transparent);
    color: var(--destructive);
    border: 1px solid color-mix(in oklab, var(--destructive) 25%, transparent);
}
/* 部分失败警告（如开机自启设置失败但配置已保存） */
.settings-banner-warn {
    background: color-mix(in oklab, #f59e0b 10%, transparent);
    color: #b45309;
    border: 1px solid color-mix(in oklab, #f59e0b 25%, transparent);
}
.settings-banner-success {
    background: color-mix(in oklab, var(--primary) 10%, transparent);
    color: var(--primary);
    border: 1px solid color-mix(in oklab, var(--primary) 25%, transparent);
}
.settings-footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 16px 24px;
}
</style>
