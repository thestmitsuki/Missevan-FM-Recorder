<script setup lang="ts">
/**
 * 向导第一页：欢迎页（规格「引导菜单」第一页）
 *
 * 主标题 + 可滚动简介/免责声明容器 + 底部「不同意」（退出应用）/「同意」（下一页）。
 * 右上角预留「导入旧配置」占位按钮（规格未来扩展）。
 */
import { useI18n } from "vue-i18n";
import { Headphones } from "@lucide/vue";
import { Button } from "@/components/ui/button";

const { t } = useI18n();

defineEmits<{
  agree: [];
  disagree: [];
}>();
</script>

<template>
  <div class="flex flex-1 flex-col">
    <!-- 右上角：导入旧配置（规格未来扩展，暂置灰） -->
    <div class="flex justify-end">
      <Button variant="ghost" size="sm" disabled class="text-muted-foreground">
        {{ t("wizard.importOldConfig") }}
      </Button>
    </div>

    <div class="flex flex-1 flex-col items-center justify-center text-center">
      <div
        class="flex size-20 items-center justify-center rounded-full bg-primary/10 text-primary"
      >
        <Headphones class="size-10" />
      </div>

      <h1 class="mt-5 text-2xl font-semibold tracking-tight">
        {{ t("wizard.welcomeTitle") }}
      </h1>

      <!-- 可滚动简介 + 免责声明 -->
      <div
        class="mt-6 max-h-56 w-full max-w-md overflow-y-auto rounded-lg border bg-muted/40 px-5 py-4 text-left text-sm leading-relaxed text-muted-foreground"
        role="region"
        :aria-label="t('wizard.welcomeIntroAria')"
      >
        <p>{{ t("wizard.welcomeIntro") }}</p>
        <p class="mt-3 text-foreground/80">
          {{ t("wizard.welcomeDisclaimer") }}
        </p>
      </div>

      <!-- 底部按钮区 -->
      <div class="mt-8 flex items-center gap-4">
        <Button
          variant="outline"
          size="lg"
          class="min-w-32"
          @click="$emit('disagree')"
        >
          {{ t("wizard.disagree") }}
        </Button>
        <Button size="lg" class="min-w-32" @click="$emit('agree')">
          {{ t("wizard.agree") }}
        </Button>
      </div>
    </div>
  </div>
</template>
