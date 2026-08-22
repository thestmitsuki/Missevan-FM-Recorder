<script setup lang="ts">
/**
 * 向导第四页：完成页（规格「引导菜单」第四页）
 *
 * 绿色对勾 + 「一切就绪，开始录制吧！」+ 「进入应用」按钮。
 * 视觉：成功徽标光环 + 轻微弹入动画、标题渐变、告警区卡片化。
 *
 * 写入时机（修复子代理 B 核心）：点击「进入应用」时**先全量落盘、再完成向导**：
 * 1. save_config：stagedToConfigPatch（全量 staged，含 autostart / trayMinimize→
 *    close_behavior / FFmpeg 下载路径，wizard_completed=true 与后端 finish_wizard
 *    语义对齐）合并进现有配置 → 这是配置文件的**唯一写入点**；
 * 2. set_autostart：注册表仅值变化时调用（失败仅提示该项，不阻断，与设置页 I3 一致）；
 * 3. finish_wizard：关向导窗、显主窗、刷新文件缓存、触发立即检测。
 *
 * 保证：中途任意步骤退出 → 无配置文件 → 下次启动仍进向导；
 * 完成 → 配置落盘（含全部暂存值）→ 下次启动进主页面。
 *
 * 注（M7 审查跟进）：`finish_wizard` 会先销毁向导窗口，命令响应无法送达
 * webview——`await` 之后的代码（旧的 `wizardStore.complete()` 写 localStorage）
 * 实际上永不执行，属死代码，已移除；向导完成标记由 save_config 落盘维护
 * （wizard_completed=true）。
 */
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { CircleCheck, RotateCw } from "@lucide/vue";

import { api } from "@/services/api";
import { isLinuxPlatform } from "@/services/platform";
import { useConfigStore } from "@/stores/configStore";
import { useWizardStore } from "@/stores/wizardStore";
import { stagedToConfigPatch } from "../stagedToConfig";

import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";

const { t } = useI18n();
const configStore = useConfigStore();
const wizardStore = useWizardStore();

const finishing = ref(false);
const error = ref<string | null>(null);
/** 开机自启（注册表）单独失败警告：配置已保存，仅提示该项，不阻断进入应用（与设置页 I3 一致） */
const autostartWarn = ref<string | null>(null);

async function enterApp() {
  if (finishing.value) return;
  finishing.value = true;
  error.value = null;
  autostartWarn.value = null;
  try {
    // 1. 唯一落盘点：全量 staged → save_config（含 autostart / trayMinimize→
    //    close_behavior / FFmpeg 下载路径 / wizard_completed=true）
    const previousAutostart = configStore.config.autostart;
    const merged = {
      ...configStore.config,
      ...stagedToConfigPatch(wizardStore.staged, isLinuxPlatform(), {
        wizardCompleted: true,
      }),
    };
    configStore.updateConfig(merged);
    await configStore.saveConfig();

    // 2. 开机自启（注册表）：仅在值变化时调用（H1）；失败仅提示该项，不阻断（I3）
    if (wizardStore.staged.autostart !== previousAutostart) {
      try {
        await api.setAutostart(wizardStore.staged.autostart);
      } catch (e) {
        autostartWarn.value = t("wizard.autostartFailed", { error: String(e) });
      }
    }

    // 3. 完成向导（关向导窗、显主窗、刷新文件缓存、触发立即检测）
    await api.finishWizard();
    // finish_wizard 已关闭向导窗口，此组件随后销毁；配置（wizard_completed=true）
    // 已在第 1 步落盘，无需再写任何完成标记
  } catch (e) {
    error.value = String(e);
  } finally {
    finishing.value = false;
  }
}
</script>

<template>
  <div class="flex flex-1 flex-col items-center justify-center py-4 text-center">
    <!-- 成功徽标：光环 + 弹入动画 -->
    <div class="relative">
      <div
        class="absolute -inset-5 rounded-full bg-emerald-500/15 blur-2xl"
        aria-hidden="true"
      />
      <div
        class="animate-pop-in relative flex size-28 items-center justify-center rounded-full bg-gradient-to-br from-emerald-400 to-emerald-600 text-white shadow-lg"
      >
        <CircleCheck class="size-14" aria-hidden="true" />
      </div>
    </div>

    <h2
      class="mt-7 bg-gradient-to-r from-foreground via-foreground to-emerald-600 bg-clip-text text-3xl font-bold tracking-tight text-transparent"
    >
      {{ t("wizard.completeTitle") }}
    </h2>

    <div
      v-if="error"
      class="mt-7 flex w-full max-w-md items-center justify-between gap-3 rounded-xl border border-destructive/40 bg-destructive/10 px-4 py-3"
      role="alert"
    >
      <span class="text-left text-xs text-destructive">
        {{ t("wizard.finishFailed", { error }) }}
      </span>
      <Button variant="outline" size="sm" :disabled="finishing" @click="enterApp">
        <RotateCw class="size-3.5" />
        {{ t("wizard.saveRetry") }}
      </Button>
    </div>

    <!-- 开机自启（注册表）设置失败：配置已保存，仅提示该项，不阻断进入应用（I3） -->
    <div
      v-if="autostartWarn"
      class="mt-7 flex w-full max-w-md items-center justify-between gap-3 rounded-xl border border-amber-500/40 bg-amber-500/10 px-4 py-3"
      role="alert"
    >
      <span class="text-left text-xs text-amber-600 dark:text-amber-400">
        {{ autostartWarn }}
      </span>
    </div>

    <Button
      size="lg"
      class="mt-10 min-w-44 shadow-md"
      :disabled="finishing"
      @click="enterApp"
    >
      <Spinner v-if="finishing" class="size-4" />
      <CircleCheck v-else class="size-4" />
      {{ t("wizard.enterApp") }}
    </Button>
  </div>
</template>

<style scoped>
@keyframes pop-in {
  0% {
    opacity: 0;
    transform: scale(0.85);
  }
  60% {
    transform: scale(1.04);
  }
  100% {
    opacity: 1;
    transform: scale(1);
  }
}
.animate-pop-in {
  animation: pop-in 0.45s cubic-bezier(0.22, 1, 0.36, 1) both;
}
</style>
