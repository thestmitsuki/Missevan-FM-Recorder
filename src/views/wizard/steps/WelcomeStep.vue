<script setup lang="ts">
/**
 * 向导第一页：欢迎页（规格「引导菜单」第一页）
 *
 * 主标题 + 可滚动简介/免责声明容器 + 底部「不同意」（退出应用）/「同意」（下一页）。
 * 右上角预留「导入旧配置」占位按钮（规格未来扩展）。
 * 视觉：品牌渐变徽标 + 光环、标题渐变文字、卡片式介绍容器（ui/card）。
 */
import { useI18n } from "vue-i18n";
import { Headphones } from "@lucide/vue";
import { Button } from "@/components/ui/button";
import {
    Card,
    CardContent,
    CardHeader,
    CardTitle,
} from "@/components/ui/card";
import { ScrollArea } from "@/components/ui/scroll-area";

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

    <div class="flex flex-1 flex-col items-center justify-center py-4 text-center">
      <!-- 品牌徽标：渐变底 + 光环 -->
      <div class="relative">
        <div
          class="absolute -inset-4 rounded-full bg-primary/15 blur-2xl"
          aria-hidden="true"
        />
        <div
          class="relative flex size-24 items-center justify-center rounded-2xl bg-gradient-to-br from-primary via-primary/80 to-accent text-primary-foreground shadow-lg"
        >
          <Headphones class="size-12" aria-hidden="true" />
        </div>
      </div>

      <h1
        class="mt-7 bg-gradient-to-r from-foreground via-foreground to-primary bg-clip-text text-3xl font-bold tracking-tight text-transparent"
      >
        {{ t("wizard.welcomeTitle") }}
      </h1>

      <!-- 可滚动简介 + 免责声明（卡片式） -->
      <Card class="mt-8 w-full max-w-md gap-0 rounded-2xl bg-muted/30 shadow-none">
        <CardHeader class="gap-0.5 px-5 pt-4 pb-2 text-left">
          <CardTitle class="text-sm font-semibold">
            {{ t("wizard.welcomeIntroAria") }}
          </CardTitle>
        </CardHeader>
        <CardContent class="px-5 pb-4 text-left text-sm leading-relaxed text-muted-foreground">
          <ScrollArea class="max-h-44 pr-3" type="auto">
            <p>{{ t("wizard.welcomeIntro") }}</p>
            <p class="mt-3 border-t border-border/60 pt-3 text-foreground/80">
              {{ t("wizard.welcomeDisclaimer") }}
            </p>
          </ScrollArea>
        </CardContent>
      </Card>

      <!-- 底部按钮区 -->
      <div class="mt-9 flex items-center gap-4">
        <Button
          variant="outline"
          size="lg"
          class="min-w-32"
          @click="$emit('disagree')"
        >
          {{ t("wizard.disagree") }}
        </Button>
        <Button size="lg" class="min-w-32 shadow-md" @click="$emit('agree')">
          {{ t("wizard.agree") }}
        </Button>
      </div>
    </div>
  </div>
</template>
