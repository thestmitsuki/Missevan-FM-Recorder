<script setup lang="ts">
/**
 * 设置向导（规格「引导菜单」第 1-4 节）
 *
 * 4 步流程：欢迎 → 基本设置 → 环境检查 → 完成。
 * 仅渲染于独立 wizard 窗口（App.vue 按窗口 label 分流，不含 AppLayout）。
 */
import { onMounted, onUnmounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { Check } from "@lucide/vue";

import { api } from "@/services/api";
import { isWizardWindow } from "@/services/window";
import { useConfigStore } from "@/stores/configStore";
import { useThemeStore } from "@/stores/themeStore";
import { useWizardStore } from "@/stores/wizardStore";
import { i18n } from "@/locales";

import { Button } from "@/components/ui/button";
import {
    AlertDialog,
    AlertDialogCancel,
    AlertDialogContent,
    AlertDialogDescription,
    AlertDialogFooter,
    AlertDialogHeader,
    AlertDialogTitle,
} from "@/components/ui/alert-dialog";

import WelcomeStep from "./steps/WelcomeStep.vue";
import BasicSettingsStep from "./steps/BasicSettingsStep.vue";
import EnvCheckStep from "./steps/EnvCheckStep.vue";
import CompleteStep from "./steps/CompleteStep.vue";

const { t } = useI18n();
const configStore = useConfigStore();
const themeStore = useThemeStore();
const wizardStore = useWizardStore();

// ── 步骤状态 ──
const currentStep = ref(1);

const stepList = [
    { key: "welcome", labelKey: "wizard.steps.welcome" },
    { key: "basic", labelKey: "wizard.steps.basic" },
    { key: "env", labelKey: "wizard.steps.env" },
    { key: "done", labelKey: "wizard.steps.done" },
] as const;

// ── 关闭确认（M4：窗口关闭按钮 → 确认对话框，是=exitApp，否=留当前页） ──
const closeConfirmOpen = ref(false);
let unlistenClose: UnlistenFn | null = null;

async function registerCloseGuard() {
    if (!isWizardWindow()) return; // 浏览器调试环境无 Tauri 窗口
    try {
        const win = getCurrentWebviewWindow();
        unlistenClose = await win.onCloseRequested((event) => {
            event.preventDefault();
            closeConfirmOpen.value = true;
        });
    } catch {
        // 非 Tauri 环境忽略
    }
}

function handleExitApp() {
    api.exitApp().catch(() => {
        closeConfirmOpen.value = false;
    });
}

// ── 暂存默认值：配置异步加载完成后补齐（只填空值，保留用户已编辑项） ──
function syncStagedFromConfig() {
    const locale = String(i18n.global.locale.value);
    wizardStore.initStaged(
        configStore.config,
        locale === "en" ? "en" : "zh-CN",
        themeStore.mode,
    );
}

onMounted(() => {
    registerCloseGuard();
    // immediate：配置可能已加载完成（loading 已为 false），先补一次默认值；
    // 未完成时等 fetchConfig 结束后再补（initStaged 幂等，只填空值）
    watch(
        () => configStore.loading,
        (loading) => {
            if (!loading) syncStagedFromConfig();
        },
        { immediate: true },
    );
});

onUnmounted(() => {
    unlistenClose?.();
});
</script>

<template>
    <div
        class="flex h-screen flex-col overflow-hidden bg-background text-foreground"
    >
        <!-- ── 顶部步骤条：已完成绿勾 / 当前蓝点 / 未完成灰点 + 第 X/4 步 ── -->
        <header class="shrink-0 border-b-0 bg-background/95 px-6 pt-4 pb-3">
            <div class="mb-2 text-right text-xs text-muted-foreground">
                {{ t("wizard.stepIndicator", { current: currentStep }) }}
            </div>
            <ol class="flex items-start">
                <li
                    v-for="(step, idx) in stepList"
                    :key="step.key"
                    class="flex flex-1 flex-col items-center"
                >
                    <div class="flex w-full items-center">
                        <div
                            class="h-0.5 flex-1"
                            :class="
                                idx === 0
                                    ? 'invisible'
                                    : currentStep > idx
                                      ? 'bg-emerald-500'
                                      : 'bg-border'
                            "
                        />
                        <div
                            class="flex size-7 items-center justify-center rounded-full border text-xs font-medium transition-colors"
                            :class="
                                currentStep > idx + 1
                                    ? 'border-emerald-500 bg-emerald-500 text-white'
                                    : currentStep === idx + 1
                                      ? 'border-primary bg-primary text-primary-foreground'
                                      : 'border-border bg-background text-muted-foreground'
                            "
                            role="presentation"
                        >
                            <Check
                                v-if="currentStep > idx + 1"
                                class="size-4"
                            />
                            <span v-else>{{ idx + 1 }}</span>
                        </div>
                        <div
                            class="h-0.5 flex-1"
                            :class="
                                idx === stepList.length - 1
                                    ? 'invisible'
                                    : currentStep > idx + 1
                                      ? 'bg-emerald-500'
                                      : 'bg-border'
                            "
                        />
                    </div>
                    <span
                        class="mt-1.5 text-xs"
                        :class="
                            currentStep === idx + 1
                                ? 'font-medium text-primary'
                                : 'text-muted-foreground'
                        "
                    >
                        {{ t(step.labelKey) }}
                    </span>
                </li>
            </ol>
        </header>

        <!-- ── 步骤内容 ── -->
        <main class="flex-1 overflow-y-auto">
            <Transition name="wizard-fade" mode="out-in">
                <div
                    :key="currentStep"
                    class="mx-auto flex min-h-full w-full max-w-lg flex-col px-6 py-6"
                >
                    <WelcomeStep
                        v-if="currentStep === 1"
                        @agree="currentStep = 2"
                        @disagree="handleExitApp"
                    />
                    <BasicSettingsStep
                        v-else-if="currentStep === 2"
                        @previous="currentStep = 1"
                        @next="currentStep = 3"
                    />
                    <EnvCheckStep
                        v-else-if="currentStep === 3"
                        @previous="currentStep = 2"
                        @next="currentStep = 4"
                        @change-output-dir="currentStep = 2"
                    />
                    <CompleteStep v-else />
                </div>
            </Transition>
        </main>

        <!-- ── 关闭确认对话框 ── -->
        <AlertDialog v-model:open="closeConfirmOpen">
            <AlertDialogContent class="max-w-sm">
                <AlertDialogHeader>
                    <AlertDialogTitle>{{
                        t("wizard.closeConfirmTitle")
                    }}</AlertDialogTitle>
                    <AlertDialogDescription>
                        {{ t("wizard.closeConfirmDesc") }}
                    </AlertDialogDescription>
                </AlertDialogHeader>
                <AlertDialogFooter>
                    <AlertDialogCancel>{{
                        t("wizard.closeConfirmNo")
                    }}</AlertDialogCancel>
                    <Button
                        variant="destructive"
                        class="bg-destructive text-white hover:bg-destructive/90"
                        @click="handleExitApp"
                    >
                        {{ t("wizard.closeConfirmYes") }}
                    </Button>
                </AlertDialogFooter>
            </AlertDialogContent>
        </AlertDialog>
    </div>
</template>

<style scoped>
.wizard-fade-enter-active,
.wizard-fade-leave-active {
    transition:
        opacity 0.18s ease,
        transform 0.18s ease;
}

.wizard-fade-enter-from {
    opacity: 0;
    transform: translateY(10px);
}

.wizard-fade-leave-to {
    opacity: 0;
    transform: translateY(-10px);
}
</style>
