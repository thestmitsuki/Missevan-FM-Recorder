<script setup lang="ts">
/**
 * 关于对话框（规格 §2.1 关于窗口）——从设置页「关于」入口打开。
 *
 * 内容：应用名称 / 版本号 / 构建日期（后端 get_app_info）/ 开发者与许可证声明 /
 * 法律声明与免责条款 / 开源依赖列表（可折叠）/ 检查更新按钮
 * （check_update → GitHub Releases API；有新版本提供下载链接，open_browser 打开）/
 * 报告问题按钮（open_browser 打开 GitHub Issues 新建页，预填标题与系统信息）。
 *
 * 可访问性：基于 ui/dialog（reka-ui）——焦点进入弹窗、Tab 循环、Esc 关闭内建；
 * 检查更新结果区域 role="status" aria-live="polite" 播报。
 */
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { Bug, Download, LoaderCircle } from "@lucide/vue";
import pkg from "../../../package.json";
import { Button } from "@/components/ui/button";
import {
    Collapsible,
    CollapsibleContent,
    CollapsibleTrigger,
} from "@/components/ui/collapsible";
import {
    Dialog,
    DialogContent,
    DialogDescription,
    DialogFooter,
    DialogHeader,
    DialogTitle,
} from "@/components/ui/dialog";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { Label } from "@/components/ui/label";
import { useDebugStore } from "@/stores/debugStore";
import { api } from "@/services/api";
import type { AppInfo } from "@/types";

const open = defineModel<boolean>("open", { default: false });
const { t } = useI18n();
const debugStore = useDebugStore();

const info = ref<AppInfo | null>(null);
const infoError = ref<string | null>(null);
const depsOpen = ref(false);

/** 报告问题：GitHub Issues 新建页面（预填标题与系统信息，规格 §2.2） */
const ISSUES_BASE_URL =
    "https://github.com/thestmitsuki/Missevan-FM-Recorder/issues/new";
const reportError = ref<string | null>(null);

/** 打开 GitHub Issues 新建页，标题 + 正文预填（URL 编码），正文含系统信息 */
function reportIssue() {
    const version = info.value?.version ?? pkg.version;
    const title = t("help.issueTitle", { version });
    const kv = (label: string, value: string) => `**${label}**: ${value}`;
    const body = [
        kv(t("help.bodyAppVersion"), version),
        kv(t("help.bodyOs"), info.value?.os ?? "—"),
        kv(t("help.bodyRust"), info.value?.rust_version ?? "—"),
        kv(t("help.bodyTauri"), info.value?.tauri_version ?? "—"),
        kv(t("help.bodyBuildDate"), info.value?.build_date ?? "—"),
        "",
        `**${t("help.bodyDescription")}**`,
        t("help.bodyDescriptionPlaceholder"),
        "",
        `**${t("help.bodySteps")}**`,
        "1. ",
        "2. ",
        "3. ",
        "",
        `**${t("help.bodyDiagnostics")}**`,
        t("help.bodyDiagnosticsHint"),
    ].join("\n");
    const url = `${ISSUES_BASE_URL}?title=${encodeURIComponent(title)}&body=${encodeURIComponent(body)}`;
    api.openBrowser(url).catch((e) => {
        reportError.value = t("help.reportFailed", { error: String(e) });
    });
}

/** 检查更新结果：null=未检查 / "checking" / "update" / "uptodate" / "error" */
const checkState = ref<"idle" | "checking" | "update" | "uptodate" | "error">(
    "idle",
);
const checkText = ref("");
const checkUrl = ref<string | null>(null);

/**
 * 开源依赖列表——前端 npm 依赖版本**动态读取 package.json**
 * （`import pkg from "../../../package.json"`，Vite 构建时打包进 bundle，
 * 版本号与 package.json 永远一致；显示时去掉 ^/~ 前缀）；后端核心依赖为
 * 静态清单（Cargo.toml/Cargo.lock 来源），其中 Tauri/Rust 版本优先取
 * 运行时 get_app_info 返回的真实版本。
 */
interface DependencyRow {
    name: string;
    version: string;
    license: string;
}

/** 前端 npm 依赖（显示名 + package.json 键名 + 许可证声明） */
const FRONTEND_DEPS: { name: string; pkg: string; license: string }[] = [
    { name: "Vue", pkg: "vue", license: "MIT" },
    { name: "Pinia", pkg: "pinia", license: "MIT" },
    { name: "Vue Router", pkg: "vue-router", license: "MIT" },
    { name: "vue-i18n", pkg: "vue-i18n", license: "MIT" },
    { name: "Tailwind CSS", pkg: "tailwindcss", license: "MIT" },
    { name: "reka-ui", pkg: "reka-ui", license: "MIT" },
    { name: "@lucide/vue", pkg: "@lucide/vue", license: "ISC" },
    { name: "vue-sonner", pkg: "vue-sonner", license: "MIT" },
    {
        name: "@tauri-apps/api",
        pkg: "@tauri-apps/api",
        license: "MIT / Apache-2.0",
    },
    {
        name: "@tauri-apps/plugin-dialog",
        pkg: "@tauri-apps/plugin-dialog",
        license: "MIT / Apache-2.0",
    },
    { name: "@tanstack/vue-table", pkg: "@tanstack/vue-table", license: "MIT" },
    { name: "@vueuse/core", pkg: "@vueuse/core", license: "MIT" },
    {
        name: "class-variance-authority",
        pkg: "class-variance-authority",
        license: "MIT",
    },
    { name: "clsx", pkg: "clsx", license: "MIT" },
    { name: "tailwind-merge", pkg: "tailwind-merge", license: "MIT" },
];

/** 后端核心依赖（静态清单；版本来源 Cargo.toml / Cargo.lock；Tauri/Rust 运行时覆写） */
const BACKEND_DEPS: DependencyRow[] = [
    { name: "Tauri", version: "2", license: "MIT / Apache-2.0" },
    { name: "Rust", version: "1.77", license: "MIT / Apache-2.0" },
    { name: "reqwest", version: "0.12", license: "MIT / Apache-2.0" },
    { name: "tokio", version: "1", license: "MIT" },
    { name: "FFmpeg", version: "—", license: "LGPL / GPL" },
];

/** 合并后的依赖表（前端动态 + 后端静态；Tauri/Rust 版本取运行时信息） */
const DEPENDENCIES = computed<DependencyRow[]>(() => {
    const frontend: DependencyRow[] = FRONTEND_DEPS.map((d) => ({
        name: d.name,
        // 版本号动态取自 package.json（去掉 ^ / ~ 前缀）
        version: String(
            (pkg.dependencies as Record<string, string> | undefined)?.[d.pkg] ??
                (pkg.devDependencies as Record<string, string> | undefined)?.[
                    d.pkg
                ] ??
                "—",
        ).replace(/^[\^~]/, ""),
        license: d.license,
    }));
    const backend: DependencyRow[] = BACKEND_DEPS.map((d) => {
        // 运行时可得的真实版本优先（get_app_info）
        if (d.name === "Tauri" && info.value?.tauri_version) {
            return { ...d, version: info.value.tauri_version };
        }
        if (d.name === "Rust" && info.value?.rust_version) {
            return { ...d, version: info.value.rust_version };
        }
        return d;
    });
    return [...frontend, ...backend];
});

/** 语义化版本比较：a > b → 1，a < b → -1，相等 → 0（"1.2" 与 "1.2.0" 视为相等） */
function compareVersions(a: string, b: string): number {
    const pa = a.split(".").map((x) => parseInt(x, 10) || 0);
    const pb = b.split(".").map((x) => parseInt(x, 10) || 0);
    for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
        const diff = (pa[i] ?? 0) - (pb[i] ?? 0);
        if (diff !== 0) return diff > 0 ? 1 : -1;
    }
    return 0;
}

async function loadInfo() {
    infoError.value = null;
    try {
        info.value = await api.getAppInfo();
    } catch (e) {
        infoError.value = String(e);
    }
}

async function checkUpdate() {
    if (checkState.value === "checking") return;
    checkState.value = "checking";
    checkText.value = "";
    checkUrl.value = null;
    try {
        const result = await api.checkUpdate();
        if (compareVersions(result.latest, result.current) > 0) {
            checkState.value = "update";
            checkText.value = t("about.updateAvailable", {
                latest: result.latest,
            });
            checkUrl.value = result.download_url;
        } else {
            checkState.value = "uptodate";
            checkText.value = t("about.upToDate", {
                current: result.current,
                latest: result.latest,
            });
        }
    } catch (e) {
        checkState.value = "error";
        checkText.value = t("about.checkFailed", { error: String(e) });
    }
}

async function openDownloadUrl(url: string) {
    try {
        await api.openBrowser(url);
    } catch (e) {
        checkState.value = "error";
        checkText.value = t("about.checkFailed", { error: String(e) });
    }
}

// 每次打开时刷新应用信息并重置检查结果/错误/折叠状态
watch(open, (v) => {
    if (v) {
        loadInfo();
        checkState.value = "idle";
        checkText.value = "";
        checkUrl.value = null;
        reportError.value = null;
        depsOpen.value = false;
    }
});
</script>

<template>
    <Dialog v-model:open="open">
        <DialogContent class="max-w-lg">
            <DialogHeader>
                <DialogTitle>{{ t("about.title") }}</DialogTitle>
                <DialogDescription>{{ t("about.subtitle") }}</DialogDescription>
            </DialogHeader>

            <div class="flex flex-col gap-4 px-6">
                <!-- 应用信息（后端 get_app_info） -->
                <dl class="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1.5 text-sm">
                    <template v-if="info">
                        <dt class="text-muted-foreground">
                            {{ t("about.version") }}
                        </dt>
                        <dd>{{ info.version }}</dd>
                        <dt class="text-muted-foreground">
                            {{ t("about.buildDate") }}
                        </dt>
                        <dd>{{ info.build_date }}</dd>
                        <dt class="text-muted-foreground">
                            {{ t("about.os") }}
                        </dt>
                        <dd>{{ info.os }}</dd>
                        <dt class="text-muted-foreground">
                            {{ t("about.rustVersion") }}
                        </dt>
                        <dd>{{ info.rust_version }}</dd>
                        <dt class="text-muted-foreground">
                            {{ t("about.tauriVersion") }}
                        </dt>
                        <dd>{{ info.tauri_version }}</dd>
                    </template>
                    <p v-else-if="infoError" class="text-xs text-destructive">
                        {{ infoError }}
                    </p>
                    <p v-else class="text-xs text-muted-foreground">
                        {{ t("common.loading") }}
                    </p>
                </dl>

                <Separator />

                <!-- 开发者 / 许可证 / 法律声明 -->
                <div class="space-y-2 text-sm">
                    <p>
                        <span class="text-muted-foreground"
                            >{{ t("about.developer") }}：</span
                        >
                        {{ t("about.developerValue") }}
                    </p>
                    <p>
                        <span class="text-muted-foreground"
                            >{{ t("about.license") }}：</span
                        >
                        {{ t("about.licenseValue") }}
                    </p>
                    <p class="text-xs leading-relaxed text-muted-foreground">
                        <span class="font-medium text-foreground"
                            >{{ t("about.legal") }}：</span
                        >
                        {{ t("about.legalBody") }}
                    </p>
                </div>

                <Separator />

                <!-- 调试面板开关（开启后主导航显示"调试面板"入口，默认关闭） -->
                <div class="flex items-center justify-between gap-4">
                    <div>
                        <Label for="cfg-debug-enabled">
                            {{ t("about.debugEnabled") }}
                        </Label>
                        <p class="mt-0.5 text-xs text-muted-foreground">
                            {{ t("about.debugEnabledHint") }}
                        </p>
                    </div>
                    <Switch
                        id="cfg-debug-enabled"
                        :checked="debugStore.enabled"
                        @update:checked="debugStore.setEnabled"
                    />
                </div>

                <Separator />

                <!-- 开源依赖列表（可折叠） -->
                <Collapsible v-model:open="depsOpen">
                    <CollapsibleTrigger as-child>
                        <Button
                            variant="outline"
                            size="sm"
                            class="w-full justify-between"
                        >
                            {{ t("about.dependencies") }}
                            <span aria-hidden="true"></span>
                        </Button>
                    </CollapsibleTrigger>
                    <CollapsibleContent
                        class="mt-2 max-h-48 overflow-y-auto rounded-md border border-border/60"
                    >
                        <p
                            class="border-b border-border/60 px-3 py-1.5 text-[10px] text-muted-foreground"
                        >
                            {{ t("about.depSource") }}
                        </p>
                        <table class="w-full text-xs">
                            <thead class="sticky top-0 bg-muted/80">
                                <tr class="text-left text-muted-foreground">
                                    <th class="px-3 py-1.5 font-medium">
                                        {{ t("about.depName") }}
                                    </th>
                                    <th class="px-3 py-1.5 font-medium">
                                        {{ t("about.depVersion") }}
                                    </th>
                                    <th class="px-3 py-1.5 font-medium">
                                        {{ t("about.depLicense") }}
                                    </th>
                                </tr>
                            </thead>
                            <tbody>
                                <tr
                                    v-for="d in DEPENDENCIES"
                                    :key="d.name"
                                    class="border-t border-border/40"
                                >
                                    <td class="px-3 py-1.5">{{ d.name }}</td>
                                    <td
                                        class="px-3 py-1.5 text-muted-foreground"
                                    >
                                        {{ d.version }}
                                    </td>
                                    <td
                                        class="px-3 py-1.5 text-muted-foreground"
                                    >
                                        {{ d.license }}
                                    </td>
                                </tr>
                            </tbody>
                        </table>
                    </CollapsibleContent>
                </Collapsible>

                <!-- 检查更新（规格 §2.1：GitHub Releases API；结果 aria-live 播报） -->
                <div
                    class="flex flex-wrap items-center gap-2"
                    role="status"
                    aria-live="polite"
                >
                    <Button
                        variant="outline"
                        size="sm"
                        :disabled="checkState === 'checking'"
                        @click="checkUpdate"
                    >
                        <LoaderCircle
                            v-if="checkState === 'checking'"
                            class="size-4 animate-spin"
                            aria-hidden="true"
                        />
                        {{ t("about.checkUpdate") }}
                    </Button>
                    <span
                        v-if="checkState === 'update'"
                        class="text-sm font-medium text-primary"
                    >
                        {{ checkText }}
                    </span>
                    <span
                        v-else-if="checkState === 'uptodate'"
                        class="text-sm text-muted-foreground"
                    >
                        {{ checkText }}
                    </span>
                    <span
                        v-else-if="checkState === 'error'"
                        class="text-xs text-destructive"
                    >
                        {{ checkText }}
                    </span>
                    <Button
                        v-if="checkState === 'update' && checkUrl"
                        size="sm"
                        class="ml-auto"
                        @click="openDownloadUrl(checkUrl)"
                    >
                        <Download class="size-6" aria-hidden="true" />
                        {{ t("about.downloadUpdate") }}
                    </Button>
                </div>

                <Separator />

                <!-- 报告问题（规格 §2.2：GitHub Issues 预填系统信息） -->
                <div class="flex items-center justify-between gap-4">
                    <div>
                        <p class="text-sm font-medium">
                            {{ t("help.reportIssueTitle") }}
                        </p>
                        <p class="mt-0.5 text-xs text-muted-foreground">
                            {{ t("help.reportIssueHint") }}
                        </p>
                    </div>
                    <Button variant="outline" size="sm" @click="reportIssue">
                        <Bug class="size-6" aria-hidden="true" />
                        {{ t("help.reportIssue") }}
                    </Button>
                </div>
                <p
                    v-if="reportError"
                    class="text-xs text-destructive"
                    role="alert"
                >
                    {{ reportError }}
                </p>
            </div>

            <DialogFooter>
                <Button variant="outline" @click="open = false">
                    {{ t("about.close") }}
                </Button>
            </DialogFooter>
        </DialogContent>
    </Dialog>
</template>
