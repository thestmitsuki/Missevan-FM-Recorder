<script setup lang="ts">
/**
 * 外观分类（规格 7.6）：全部为「保存后生效」暂存态（D5 决策已迁移，I6）——
 * 主题（config.theme）、强调色/密度/字号/卡片显示项（config.appearance）只写暂存
 * 表单字段，SettingsView save() 成功时统一提交：
 * - theme → themeStore.setMode（持久化 theme_mode）
 * - appearance → appearanceStore.update（applyPrefs 改写 CSS 变量 + 持久化 appearance）
 * 设置页外（LiveView 卡片显示项、启动时主题初始化）仍直接消费 localStorage /
 * store，保存提交后即生效。
 */
import { useI18n } from "vue-i18n";
import type { AppearancePrefs, CardOptionKey, Density, FontSize } from "@/stores/appearanceStore";
import type { ThemeMode } from "@/stores/themeStore";
import type { SectionErrors, SettingsForm } from "../validation";
import { Checkbox } from "@/components/ui/checkbox";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

const props = defineProps<{
    config: SettingsForm;
    errors: SectionErrors;
}>();

const { t } = useI18n();

const ACCENT_SWATCHES = [
    "#2563eb", // 品牌蓝
    "#7c3aed", // 紫
    "#db2777", // 品红
    "#e11d48", // 玫红
    "#ea580c", // 橙
    "#16a34a", // 绿
    "#0d9488", // 青
    "#64748b", // 灰
];

const themeOptions: { value: ThemeMode; labelKey: string }[] = [
    { value: "light", labelKey: "settings.appearance.themeLight" },
    { value: "dark", labelKey: "settings.appearance.themeDark" },
    { value: "system", labelKey: "settings.appearance.themeSystem" },
];

const densityOptions: { value: Density; labelKey: string }[] = [
    { value: "compact", labelKey: "settings.appearance.densityCompact" },
    { value: "standard", labelKey: "settings.appearance.densityStandard" },
    { value: "comfortable", labelKey: "settings.appearance.densityComfortable" },
];

const fontSizeOptions: { value: FontSize; labelKey: string }[] = [
    { value: "small", labelKey: "settings.appearance.fontSizeSmall" },
    { value: "medium", labelKey: "settings.appearance.fontSizeMedium" },
    { value: "large", labelKey: "settings.appearance.fontSizeLarge" },
];

const cardOptions: { key: CardOptionKey; labelKey: string }[] = [
    { key: "cardShowAvatar", labelKey: "settings.appearance.cardAvatar" },
    { key: "cardShowTags", labelKey: "settings.appearance.cardTags" },
    { key: "cardShowRoomId", labelKey: "settings.appearance.cardRoomId" },
    { key: "cardShowStatusIcon", labelKey: "settings.appearance.cardStatusIcon" },
];

/** 就地更新暂存的外观偏好（不可变更新，触发表单 dirty） */
function setAppearance<K extends keyof AppearancePrefs>(key: K, value: AppearancePrefs[K]) {
    props.config.appearance = { ...props.config.appearance, [key]: value };
}
</script>

<template>
    <div class="space-y-6">
        <!-- 主题 -->
        <Card class="gap-0 rounded-lg p-4 shadow-none">
            <CardHeader class="mb-3 gap-0 p-0">
                <CardTitle class="text-sm font-semibold">{{ t("settings.appearance.theme") }}</CardTitle>
            </CardHeader>
            <CardContent class="p-0">
                <RadioGroup
                    :model-value="config.theme"
                    class="flex flex-col gap-2"
                    @update:model-value="(v: unknown) => (config.theme = v as ThemeMode)"
                >
                    <div v-for="o in themeOptions" :key="o.value" class="flex items-center gap-2">
                        <RadioGroupItem :id="`cfg-theme-${o.value}`" :value="o.value" class="size-4" />
                        <Label :for="`cfg-theme-${o.value}`">{{ t(o.labelKey) }}</Label>
                    </div>
                </RadioGroup>
            </CardContent>
        </Card>

        <!-- 强调色 -->
        <Card class="gap-0 rounded-lg p-4 shadow-none">
            <CardHeader class="mb-3 gap-0 p-0">
                <CardTitle class="text-sm font-semibold">{{ t("settings.appearance.accent") }}</CardTitle>
            </CardHeader>
            <CardContent class="p-0">
                <div class="flex flex-wrap items-center gap-2.5">
                    <Button
                        v-for="c in ACCENT_SWATCHES"
                        :key="c"
                        variant="ghost"
                        size="icon"
                        :aria-label="c"
                        class="size-8 cursor-pointer rounded-full border border-border p-0 transition-transform hover:scale-110"
                        :class="
                            config.appearance.accent === c
                                ? 'ring-2 ring-ring ring-offset-2'
                                : ''
                        "
                        :style="{ backgroundColor: c }"
                        @click="setAppearance('accent', c)"
                    />
                    <label
                        class="relative ml-1 inline-flex cursor-pointer items-center gap-2 rounded-md border border-dashed px-3 py-1.5 text-sm text-muted-foreground hover:text-foreground"
                    >
                        <input
                            type="color"
                            :value="config.appearance.accent"
                            class="size-5 cursor-pointer appearance-none border-0 bg-transparent p-0"
                            @input="(e) => setAppearance('accent', (e.target as HTMLInputElement).value)"
                        />
                        {{ t("settings.appearance.accentCustom") }}
                    </label>
                </div>
                <p class="mt-2 text-xs text-muted-foreground">{{ t("settings.appearance.accentHint") }}</p>
            </CardContent>
        </Card>

        <!-- 列表密度 -->
        <Card class="gap-0 rounded-lg p-4 shadow-none">
            <CardHeader class="mb-3 gap-0 p-0">
                <CardTitle class="text-sm font-semibold">{{ t("settings.appearance.density") }}</CardTitle>
            </CardHeader>
            <CardContent class="p-0">
                <RadioGroup
                    :model-value="config.appearance.density"
                    class="flex flex-col gap-2"
                    @update:model-value="(v: unknown) => setAppearance('density', v as Density)"
                >
                    <div v-for="o in densityOptions" :key="o.value" class="flex items-center gap-2">
                        <RadioGroupItem :id="`cfg-density-${o.value}`" :value="o.value" class="size-4" />
                        <Label :for="`cfg-density-${o.value}`">{{ t(o.labelKey) }}</Label>
                    </div>
                </RadioGroup>
            </CardContent>
        </Card>

        <!-- 字体大小 -->
        <Card class="gap-0 rounded-lg p-4 shadow-none">
            <CardHeader class="mb-3 gap-0 p-0">
                <CardTitle class="text-sm font-semibold">{{ t("settings.appearance.fontSize") }}</CardTitle>
            </CardHeader>
            <CardContent class="p-0">
                <RadioGroup
                    :model-value="config.appearance.fontSize"
                    class="flex flex-col gap-2"
                    @update:model-value="(v: unknown) => setAppearance('fontSize', v as FontSize)"
                >
                    <div v-for="o in fontSizeOptions" :key="o.value" class="flex items-center gap-2">
                        <RadioGroupItem :id="`cfg-font-${o.value}`" :value="o.value" class="size-4" />
                        <Label :for="`cfg-font-${o.value}`">{{ t(o.labelKey) }}</Label>
                    </div>
                </RadioGroup>
            </CardContent>
        </Card>

        <!-- 主播卡片显示选项（直播页生效） -->
        <Card class="gap-0 rounded-lg p-4 shadow-none">
            <CardHeader class="mb-3 gap-1 p-0">
                <CardTitle class="text-sm font-semibold">{{ t("settings.appearance.cardTitle") }}</CardTitle>
                <p class="text-xs text-muted-foreground">{{ t("settings.appearance.cardTitleHint") }}</p>
            </CardHeader>
            <CardContent class="p-0">
                <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
                    <div v-for="o in cardOptions" :key="o.key" class="flex items-center gap-2">
                        <Checkbox
                            :id="`cfg-card-${o.key}`"
                            :checked="config.appearance[o.key]"
                            class="size-4"
                            @update:checked="(v: boolean | 'indeterminate') => setAppearance(o.key, v === true)"
                        />
                        <Label :for="`cfg-card-${o.key}`">{{ t(o.labelKey) }}</Label>
                    </div>
                </div>
            </CardContent>
        </Card>
    </div>
</template>
