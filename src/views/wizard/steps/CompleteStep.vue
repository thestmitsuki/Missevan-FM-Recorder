<script setup lang="ts">
/**
 * 向导第四页：完成页（规格「引导菜单」第四页）
 *
 * 绿色对勾 + 「一切就绪，开始录制吧！」+ 「进入应用」按钮。
 * 点击后调用 finish_wizard（关闭向导窗、显示主窗、刷新文件缓存、触发立即检测）；
 * 失败时显示错误消息并允许重试。
 */
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { CircleCheck, LoaderCircle, RotateCw } from "@lucide/vue";

import { api } from "@/services/api";
import { useWizardStore } from "@/stores/wizardStore";

import { Button } from "@/components/ui/button";

const { t } = useI18n();
const wizardStore = useWizardStore();

const finishing = ref(false);
const error = ref<string | null>(null);

async function enterApp() {
  if (finishing.value) return;
  finishing.value = true;
  error.value = null;
  try {
    await api.finishWizard();
    wizardStore.complete(); // 持久化 wizard_completed（localStorage）
    // finish_wizard 已关闭向导窗口，此组件随后销毁
  } catch (e) {
    error.value = String(e);
  } finally {
    finishing.value = false;
  }
}
</script>

<template>
  <div class="flex flex-1 flex-col items-center justify-center text-center">
    <div
      class="flex size-24 items-center justify-center rounded-full bg-emerald-500/10"
    >
      <CircleCheck class="size-14 text-emerald-500" aria-hidden="true" />
    </div>

    <h2 class="mt-6 text-2xl font-semibold tracking-tight">
      {{ t("wizard.completeTitle") }}
    </h2>

    <div
      v-if="error"
      class="mt-6 flex w-full max-w-md items-center justify-between gap-3 rounded-lg border border-destructive/40 bg-destructive/10 px-4 py-3"
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

    <Button size="lg" class="mt-8 min-w-40" :disabled="finishing" @click="enterApp">
      <LoaderCircle v-if="finishing" class="size-4 animate-spin" />
      {{ t("wizard.enterApp") }}
    </Button>
  </div>
</template>
