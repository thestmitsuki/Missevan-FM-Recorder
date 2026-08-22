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
import { isLinuxPlatform } from "@/services/platform";
import { setLocale, type AppLocale } from "@/locales";
import { useWizardStore } from "@/stores/wizardStore";
import { useThemeStore, type ThemeMode } from "@/stores/themeStore";

import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
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

// Linux 未集成系统托盘（后端决策 #2）：托盘最小化选项禁用并显示提示
const trayDisabled = isLinuxPlatform();

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
  // L2 审查跟进：数字输入补上限校验（此前仅正则 `^\d+$`，输入 20 位大数时
  // JS Number 转浮点 → Tauri 序列化 u64 失败 → save_config 报后端技术错误）。
  // 上限与设置页 validation.ts inRange 对齐：分段间隔 0..86400、磁盘阈值
  // 1..100000；`Number.isSafeInteger` 拦截超出 JS 安全整数范围的输入
  //（正则只保证"纯数字"，不保证可安全解析）。
  const segment = Number(segmentStr.value.trim());
  if (
    !/^\d+$/.test(segmentStr.value.trim()) ||
    !Number.isSafeInteger(segment) ||
    segment > 86400
  ) {
    next.segment = t("wizard.errSegmentInvalid");
  }
  const threshold = Number(thresholdStr.value.trim());
  if (
    !/^[1-9]\d*$/.test(thresholdStr.value.trim()) ||
    !Number.isSafeInteger(threshold) ||
    threshold > 100000
  ) {
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
        <RadioGroup
          :model-value="staged.language"
          class="mt-2 flex gap-5"
          @update:model-value="(v: unknown) => selectLanguage(v as AppLocale)"
        >
          <div
            v-for="opt in languageOptions"
            :key="opt.value"
            class="flex items-center gap-2"
          >
            <RadioGroupItem :id="`wizard-lang-${opt.value}`" :value="opt.value" />
            <Label :for="`wizard-lang-${opt.value}`">{{ t(opt.labelKey) }}</Label>
          </div>
        </RadioGroup>
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
      <Card class="flex-row items-center justify-between gap-0 rounded-lg px-4 py-3 shadow-none">
        <Label for="wizard-autostart">{{ t("wizard.autostart") }}</Label>
        <Switch id="wizard-autostart" v-model:checked="staged.autostart" />
      </Card>

      <!-- 7. 关闭窗口时最小化到系统托盘（Linux 未集成托盘：禁用 + 提示） -->
      <Card class="gap-0 rounded-lg px-4 py-3 shadow-none">
        <div class="flex items-center justify-between">
          <Label for="wizard-tray">{{ t("wizard.trayMinimize") }}</Label>
          <Switch
            id="wizard-tray"
            v-model:checked="staged.trayMinimize"
            :disabled="trayDisabled"
          />
        </div>
        <p
          v-if="trayDisabled"
          id="wizard-tray-hint"
          class="mt-1.5 text-xs text-muted-foreground"
        >
          {{ t("wizard.trayUnavailable") }}
        </p>
      </Card>

      <!-- 8. 主题颜色 -->
      <div>
        <Label>{{ t("wizard.theme") }}</Label>
        <RadioGroup
          :model-value="staged.theme"
          class="mt-2 flex gap-5"
          @update:model-value="(v: unknown) => selectTheme(v as ThemeMode)"
        >
          <div
            v-for="opt in themeOptions"
            :key="opt.value"
            class="flex items-center gap-2"
          >
            <RadioGroupItem :id="`wizard-theme-${opt.value}`" :value="opt.value" />
            <Label :for="`wizard-theme-${opt.value}`">{{ t(opt.labelKey) }}</Label>
          </div>
        </RadioGroup>
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
