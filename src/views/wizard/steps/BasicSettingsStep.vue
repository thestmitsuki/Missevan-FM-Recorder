<script setup lang="ts">
/**
 * 向导第二页：基本设置（规格「引导菜单」第二页）
 *
 * 8 项表单：语言 / 输出路径 / 录制格式 / 分段间隔 / 磁盘阈值 / 开机启动 /
 * 托盘最小化 / 主题。校验失败红字显示在对应输入框下方；校验通过后仅暂存
 * 内存（wizardStore.staged），第三页全部检查通过后才写入全局配置。
 */
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { storeToRefs } from "pinia";
import { FolderOpen } from "@lucide/vue";

import { api } from "@/services/api";
import { setLocale, type AppLocale } from "@/locales";
import { useWizardStore } from "@/stores/wizardStore";
import { useThemeStore, type ThemeMode } from "@/stores/themeStore";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Switch } from "@/components/ui/switch";

const { t } = useI18n();
const wizardStore = useWizardStore();
const themeStore = useThemeStore();
const { staged } = storeToRefs(wizardStore);

const emit = defineEmits<{
  previous: [];
  next: [];
}>();

// ── 表单校验错误（红字显示在输入框下方） ──
const errors = ref<{ outputDir?: string; segment?: string; threshold?: string }>({});

// 数字字段以字符串承载，提交时按规格校验（非负整数 / 正整数）
const segmentStr = ref(String(staged.value.segmentSeconds));
const thresholdStr = ref(String(staged.value.diskThresholdGb));
const browsing = ref(false);

const languageOptions = [
  { value: "zh-CN" as AppLocale, labelKey: "wizard.languageZh" },
  { value: "en" as AppLocale, labelKey: "wizard.languageEn" },
];

const formatOptions = [
  { value: "m4a", labelKey: "wizard.formatM4A" },
  { value: "mp3", labelKey: "wizard.formatMP3" },
];

const themeOptions: { value: ThemeMode; labelKey: string }[] = [
  { value: "light", labelKey: "wizard.themeLight" },
  { value: "dark", labelKey: "wizard.themeDark" },
  { value: "system", labelKey: "wizard.themeSystem" },
];

function selectLanguage(locale: AppLocale) {
  wizardStore.setStaged({ language: locale });
  setLocale(locale); // 即时生效并持久化（localStorage['locale']）
}

function selectTheme(mode: ThemeMode) {
  wizardStore.setStaged({ theme: mode });
  themeStore.setMode(mode);
}

async function browseOutputDir() {
  browsing.value = true;
  try {
    const dir = await api.pickOutputDir();
    if (dir) {
      wizardStore.setStaged({ outputDir: dir });
    }
  } catch {
    // 非 Tauri 环境或用户取消：忽略
  } finally {
    browsing.value = false;
  }
}

function validate(): boolean {
  const next: typeof errors.value = {};

  if (!staged.value.outputDir.trim()) {
    next.outputDir = t("wizard.errOutputDirRequired");
  }
  if (!/^\d+$/.test(segmentStr.value.trim())) {
    next.segment = t("wizard.errSegmentInvalid");
  }
  if (!/^[1-9]\d*$/.test(thresholdStr.value.trim())) {
    next.threshold = t("wizard.errThresholdInvalid");
  }

  errors.value = next;
  return Object.keys(next).length === 0;
}

function handleNext() {
  if (!validate()) return;
  // 校验通过：暂存内存（不写盘），进入第三页
  wizardStore.setStaged({
    segmentSeconds: Number(segmentStr.value),
    diskThresholdGb: Number(thresholdStr.value),
  });
  emit("next");
}
</script>

<template>
  <div class="flex flex-1 flex-col">
    <h2 class="text-xl font-semibold tracking-tight">
      {{ t("wizard.basicTitle") }}
    </h2>
    <p class="mt-1 text-sm text-muted-foreground">
      {{ t("wizard.basicDesc") }}
    </p>

    <div class="mt-6 flex-1 space-y-5">
      <!-- 1. 语言 -->
      <div>
        <Label>{{ t("wizard.language") }}</Label>
        <div class="mt-2 inline-flex rounded-md border bg-muted/40 p-0.5">
          <button
            v-for="opt in languageOptions"
            :key="opt.value"
            type="button"
            class="rounded px-3 py-1.5 text-sm transition-colors"
            :class="
              staged.language === opt.value
                ? 'bg-primary text-primary-foreground shadow-sm'
                : 'text-muted-foreground hover:text-foreground'
            "
            @click="selectLanguage(opt.value)"
          >
            {{ t(opt.labelKey) }}
          </button>
        </div>
      </div>

      <!-- 2. 音频输出路径 -->
      <div>
        <Label for="wizard-output-dir">{{ t("wizard.outputDir") }}</Label>
        <div class="mt-2 flex gap-2">
          <Input
            id="wizard-output-dir"
            v-model="staged.outputDir"
            class="flex-1"
            :aria-invalid="!!errors.outputDir"
            aria-describedby="wizard-output-dir-error"
          />
          <Button
            variant="outline"
            :disabled="browsing"
            @click="browseOutputDir"
          >
            <FolderOpen class="size-4" />
            {{ t("wizard.browse") }}
          </Button>
        </div>
        <p
          v-if="errors.outputDir"
          id="wizard-output-dir-error"
          class="mt-1 text-xs font-medium text-destructive"
          role="alert"
        >
          {{ errors.outputDir }}
        </p>
      </div>

      <!-- 3. 录制格式 -->
      <div>
        <Label>{{ t("wizard.format") }}</Label>
        <RadioGroup v-model="staged.recordFormat" class="mt-2 flex gap-5">
          <div
            v-for="fmt in formatOptions"
            :key="fmt.value"
            class="flex items-center gap-2"
          >
            <RadioGroupItem :id="`wizard-fmt-${fmt.value}`" :value="fmt.value" />
            <Label :for="`wizard-fmt-${fmt.value}`">{{ t(fmt.labelKey) }}</Label>
          </div>
        </RadioGroup>
      </div>

      <!-- 4. 音频分段间隔 -->
      <div>
        <Label for="wizard-segment">{{ t("wizard.segment") }}</Label>
        <Input
          id="wizard-segment"
          v-model="segmentStr"
          class="mt-2 max-w-48"
          inputmode="numeric"
          :aria-invalid="!!errors.segment"
          aria-describedby="wizard-segment-hint wizard-segment-error"
        />
        <p id="wizard-segment-hint" class="mt-1 text-xs text-muted-foreground">
          {{ t("wizard.segmentHint") }}
        </p>
        <p
          v-if="errors.segment"
          id="wizard-segment-error"
          class="mt-1 text-xs font-medium text-destructive"
          role="alert"
        >
          {{ errors.segment }}
        </p>
      </div>

      <!-- 5. 磁盘空间阈值 -->
      <div>
        <Label for="wizard-threshold">{{ t("wizard.diskThreshold") }}</Label>
        <Input
          id="wizard-threshold"
          v-model="thresholdStr"
          class="mt-2 max-w-48"
          inputmode="numeric"
          :aria-invalid="!!errors.threshold"
          aria-describedby="wizard-threshold-hint wizard-threshold-error"
        />
        <p id="wizard-threshold-hint" class="mt-1 text-xs text-muted-foreground">
          {{ t("wizard.diskThresholdHint") }}
        </p>
        <p
          v-if="errors.threshold"
          id="wizard-threshold-error"
          class="mt-1 text-xs font-medium text-destructive"
          role="alert"
        >
          {{ errors.threshold }}
        </p>
      </div>

      <!-- 6. 开机启动 -->
      <div class="flex items-center justify-between rounded-lg border px-4 py-3">
        <Label for="wizard-autostart">{{ t("wizard.autostart") }}</Label>
        <Switch id="wizard-autostart" v-model="staged.autostart" />
      </div>

      <!-- 7. 关闭窗口时最小化到系统托盘 -->
      <div class="flex items-center justify-between rounded-lg border px-4 py-3">
        <Label for="wizard-tray">{{ t("wizard.trayMinimize") }}</Label>
        <Switch id="wizard-tray" v-model="staged.trayMinimize" />
      </div>

      <!-- 8. 主题颜色 -->
      <div>
        <Label>{{ t("wizard.theme") }}</Label>
        <div class="mt-2 inline-flex rounded-md border bg-muted/40 p-0.5">
          <button
            v-for="opt in themeOptions"
            :key="opt.value"
            type="button"
            class="rounded px-3 py-1.5 text-sm transition-colors"
            :class="
              staged.theme === opt.value
                ? 'bg-primary text-primary-foreground shadow-sm'
                : 'text-muted-foreground hover:text-foreground'
            "
            @click="selectTheme(opt.value)"
          >
            {{ t(opt.labelKey) }}
          </button>
        </div>
      </div>
    </div>

    <!-- 底部按钮 -->
    <div class="mt-8 flex items-center justify-between">
      <Button variant="outline" @click="emit('previous')">
        {{ t("wizard.previous") }}
      </Button>
      <Button class="min-w-28" @click="handleNext">
        {{ t("wizard.next") }}
      </Button>
    </div>
  </div>
</template>
