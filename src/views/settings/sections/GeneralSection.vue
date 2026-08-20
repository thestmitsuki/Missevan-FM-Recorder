<script setup lang="ts">
/**
 * 通用分类（规格 7.1）：
 * 语言（暂存于表单 config.locale，保存更改后生效）/ 开机自动启动 /
 * 关闭主窗口时（托盘|退出）/ 检查更新。
 * 写入 GlobalConfig（autostart/close_behavior/check_updates）。
 *
 * 托盘图标（show_tray）独立开关已移除（修复子代理 B）：托盘可见性由
 * close_behavior 派生——「最小化到托盘」自动显示图标、「直接退出」隐藏；
 * 后端保留 show_tray 字段仅为兼容旧配置读取。
 */
import { useI18n } from "vue-i18n";
import { isLinuxPlatform } from "@/services/platform";
import type { AppLocale } from "@/locales";
import type { SectionErrors, SettingsForm } from "../validation";
import { Switch } from "@/components/ui/switch";
import { Label } from "@/components/ui/label";
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";

const props = defineProps<{
    config: SettingsForm;
    errors: SectionErrors;
}>();

const { t } = useI18n();

// Linux 未集成系统托盘（后端决策 #2）：托盘相关选项禁用并显示提示
const trayDisabled = isLinuxPlatform();

function onLocaleChange(value: unknown) {
    if (value === "zh-CN" || value === "en") {
        props.config.locale = value as AppLocale;
    }
}
</script>

<template>
    <div class="space-y-6">
        <!-- 语言：暂存于表单，保存更改后生效 -->
        <div class="rounded-lg border p-4">
            <h3 class="mb-3 text-sm font-semibold">{{ t("settings.general.language") }}</h3>
            <p class="mb-2 text-xs text-muted-foreground">
                {{ t("settings.general.languageHint") }}
            </p>
            <Select
                :model-value="config.locale"
                @update:model-value="onLocaleChange"
            >
                <SelectTrigger class="w-52">
                    <SelectValue />
                </SelectTrigger>
                <SelectContent>
                    <SelectItem value="zh-CN">{{ t("settings.general.languageZh") }}</SelectItem>
                    <SelectItem value="en">{{ t("settings.general.languageEn") }}</SelectItem>
                </SelectContent>
            </Select>
        </div>

        <!-- 开机自动启动 -->
        <div class="flex items-center justify-between gap-4 rounded-lg border p-4">
            <div>
                <Label for="cfg-autostart">{{ t("settings.general.autostart") }}</Label>
                <p class="mt-0.5 text-xs text-muted-foreground">
                    {{ t("settings.general.autostartHint") }}
                </p>
            </div>
            <Switch id="cfg-autostart" v-model:checked="config.autostart" />
        </div>

        <!-- 关闭主窗口时（Linux 托盘不可用：tray 选项禁用 + 提示） -->
        <div class="rounded-lg border p-4">
            <h3 class="mb-3 text-sm font-semibold">{{ t("settings.general.closeBehavior") }}</h3>
            <RadioGroup v-model="config.close_behavior" class="flex flex-col gap-2">
                <div class="flex items-center gap-2">
                    <RadioGroupItem
                        id="cfg-close-tray"
                        value="tray"
                        class="size-4"
                        :disabled="trayDisabled"
                    />
                    <Label
                        for="cfg-close-tray"
                        :class="trayDisabled ? 'text-muted-foreground' : ''"
                    >
                        {{ t("settings.general.closeTray") }}
                    </Label>
                </div>
                <div class="flex items-center gap-2">
                    <RadioGroupItem id="cfg-close-exit" value="exit" class="size-4" />
                    <Label for="cfg-close-exit">{{ t("settings.general.closeExit") }}</Label>
                </div>
            </RadioGroup>
            <p v-if="trayDisabled" class="mt-1.5 text-xs text-muted-foreground">
                {{ t("settings.general.trayUnavailable") }}
            </p>
        </div>

        <!-- 检查更新 -->
        <div class="flex items-center justify-between gap-4 rounded-lg border p-4">
            <div>
                <Label for="cfg-check-updates">{{ t("settings.general.checkUpdates") }}</Label>
                <p class="mt-0.5 text-xs text-muted-foreground">
                    {{ t("settings.general.checkUpdatesHint") }}
                </p>
            </div>
            <Switch id="cfg-check-updates" v-model:checked="config.check_updates" />
        </div>
    </div>
</template>
