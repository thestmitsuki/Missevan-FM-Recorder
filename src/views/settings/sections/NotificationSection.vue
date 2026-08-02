<script setup lang="ts">
/**
 * 通知分类（规格 7.5）：
 * 全局通知开关、事件通知选择（录制开始/结束/出错、开播/下播、磁盘警告、更新可用）、
 * 通知方式（系统原生通知；应用内横幅跟随全局开关）、声音提示。
 * 字段：notifications_enabled + notify_* 全量（与 GlobalConfig 逐字对齐）。
 */
import { useI18n } from "vue-i18n";
import type { SectionErrors, SettingsForm } from "../validation";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";

const props = defineProps<{
    config: SettingsForm;
    errors: SectionErrors;
}>();

const { t } = useI18n();
</script>

<template>
    <div class="space-y-6">
        <!-- 全局通知开关 -->
        <div class="flex items-center justify-between gap-4 rounded-lg border p-4">
            <div>
                <Label for="cfg-notif-global">{{ t("settings.notification.global") }}</Label>
                <p class="mt-0.5 text-xs text-muted-foreground">
                    {{ t("settings.notification.globalHint") }}
                </p>
            </div>
            <Switch id="cfg-notif-global" v-model:checked="config.notifications_enabled" />
        </div>

        <div
            class="space-y-6"
            :class="config.notifications_enabled ? '' : 'pointer-events-none opacity-50'"
            :aria-disabled="!config.notifications_enabled"
        >
            <!-- 事件通知选择 -->
            <div class="rounded-lg border p-4">
                <h3 class="mb-3 text-sm font-semibold">{{ t("settings.notification.eventsTitle") }}</h3>
                <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
                    <div class="flex items-center gap-2">
                        <Checkbox id="cfg-notify-rec-start" v-model:checked="config.notify_recording_start" class="size-4" />
                        <Label for="cfg-notify-rec-start">{{ t("settings.notification.recStart") }}</Label>
                    </div>
                    <div class="flex items-center gap-2">
                        <Checkbox id="cfg-notify-rec-end" v-model:checked="config.notify_recording_end" class="size-4" />
                        <Label for="cfg-notify-rec-end">{{ t("settings.notification.recEnd") }}</Label>
                    </div>
                    <div class="flex items-center gap-2">
                        <Checkbox id="cfg-notify-rec-error" v-model:checked="config.notify_recording_error" class="size-4" />
                        <Label for="cfg-notify-rec-error">{{ t("settings.notification.recError") }}</Label>
                    </div>
                    <div class="flex items-center gap-2">
                        <Checkbox id="cfg-notify-live-start" v-model:checked="config.notify_live_start" class="size-4" />
                        <Label for="cfg-notify-live-start">{{ t("settings.notification.liveStart") }}</Label>
                    </div>
                    <div class="flex items-center gap-2">
                        <Checkbox id="cfg-notify-live-end" v-model:checked="config.notify_live_end" class="size-4" />
                        <Label for="cfg-notify-live-end">{{ t("settings.notification.liveEnd") }}</Label>
                    </div>
                    <div class="flex items-center gap-2">
                        <Checkbox id="cfg-notify-disk" v-model:checked="config.notify_disk_warning" class="size-4" />
                        <Label for="cfg-notify-disk">{{ t("settings.notification.diskWarning") }}</Label>
                    </div>
                    <div class="flex items-center gap-2">
                        <Checkbox id="cfg-notify-update" v-model:checked="config.notify_update" class="size-4" />
                        <Label for="cfg-notify-update">{{ t("settings.notification.updateAvailable") }}</Label>
                    </div>
                </div>
            </div>

            <!-- 通知方式 -->
            <div class="rounded-lg border p-4">
                <h3 class="mb-3 text-sm font-semibold">{{ t("settings.notification.channelTitle") }}</h3>
                <div class="space-y-3">
                    <div class="flex items-center gap-2">
                        <Checkbox id="cfg-notify-system" v-model:checked="config.notify_system" class="size-4" />
                        <Label for="cfg-notify-system">{{ t("settings.notification.channelSystem") }}</Label>
                    </div>
                    <div class="flex items-center gap-2 opacity-80">
                        <Checkbox id="cfg-notify-inapp" checked disabled class="size-4" />
                        <div>
                            <Label>{{ t("settings.notification.channelInApp") }}</Label>
                            <p class="text-xs text-muted-foreground">
                                {{ t("settings.notification.channelInAppHint") }}
                            </p>
                        </div>
                    </div>
                </div>
            </div>

            <!-- 声音提示 -->
            <div class="flex items-center justify-between gap-4 rounded-lg border p-4">
                <div>
                    <Label for="cfg-notify-sound">{{ t("settings.notification.sound") }}</Label>
                    <p class="mt-0.5 text-xs text-muted-foreground">
                        {{ t("settings.notification.soundHint") }}
                    </p>
                </div>
                <Switch id="cfg-notify-sound" v-model:checked="config.notify_sound" />
            </div>
        </div>
    </div>
</template>
