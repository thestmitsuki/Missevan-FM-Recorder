<script setup lang="ts">
/**
 * 文件管理分类（规格 7.3）：
 * 自动清理（开关 + 保留天数 + 保留总量上限 GB + 清理时间）、磁盘空间保护（阈值 GB）。
 * 字段：auto_cleanup_enabled, retention_days, max_total_gb, cleanup_time,
 * disk_space_limit_gb。
 *
 * 语义化默认值（Task 3 对齐提示 ③）：max_total_gb=0 表示不限总大小（合法，非错误）；
 * disk_space_limit_gb 为磁盘保护阈值，后端 is_valid 要求 > 0。
 */
import { toRef } from "vue";
import { useI18n } from "vue-i18n";
import type { SectionErrors, SettingsForm } from "../validation";
import { useNumberField } from "../useNumberField";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import NotEffectiveBadge from "@/components/common/NotEffectiveBadge.vue";

const props = defineProps<{
    config: SettingsForm;
    errors: SectionErrors;
}>();

const { t } = useI18n();

const { text: retentionText, invalid: retentionInvalid } = useNumberField(
    toRef(props.config, "retention_days"),
);
const { text: maxTotalText, invalid: maxTotalInvalid } = useNumberField(
    toRef(props.config, "max_total_gb"),
);
const { text: diskLimitText, invalid: diskLimitInvalid } = useNumberField(
    toRef(props.config, "disk_space_limit_gb"),
);

/** 清理时间预设（"HH:MM"） */
const CLEANUP_PRESETS = [
    "00:00",
    "03:00",
    "06:00",
    "09:00",
    "12:00",
    "18:00",
    "21:00",
];
const hasPreset = (v: string) => CLEANUP_PRESETS.includes(v.trim());

function onCleanupPreset(value: unknown) {
    if (typeof value === "string" && value !== "custom") {
        props.config.cleanup_time = value;
    }
}
</script>

<template>
    <div class="space-y-6">
        <!-- ── 自动清理 ── -->
        <div class="rounded-lg border p-4">
            <div class="mb-4 flex items-center justify-between gap-4">
                <div>
                    <Label for="cfg-cleanup-enable">{{ t("settings.files.cleanupEnable") }}<NotEffectiveBadge /></Label>
                    <p class="mt-0.5 text-xs text-muted-foreground">
                        {{ t("settings.files.cleanupEnableHint") }}
                    </p>
                </div>
                <Switch id="cfg-cleanup-enable" v-model:checked="config.auto_cleanup_enabled" />
            </div>

            <div
                class="space-y-4"
                :class="config.auto_cleanup_enabled ? '' : 'pointer-events-none opacity-50'"
                :aria-disabled="!config.auto_cleanup_enabled"
            >
                <div class="space-y-1.5">
                    <Label for="cfg-retention-days">{{ t("settings.files.retentionDays") }}</Label>
                    <Input
                        id="cfg-retention-days"
                        v-model="retentionText"
                        inputmode="numeric"
                        class="w-40"
                        :class="errors.retention_days || retentionInvalid ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                        :aria-invalid="!!errors.retention_days || retentionInvalid"
                    />
                    <p v-if="errors.retention_days" class="text-xs text-destructive">
                        {{ errors.retention_days }}
                    </p>
                </div>

                <div class="space-y-1.5">
                    <Label for="cfg-max-total">{{ t("settings.files.maxTotalGb") }}</Label>
                    <Input
                        id="cfg-max-total"
                        v-model="maxTotalText"
                        inputmode="numeric"
                        class="w-40"
                        :class="errors.max_total_gb || maxTotalInvalid ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                        :aria-invalid="!!errors.max_total_gb || maxTotalInvalid"
                    />
                    <p class="text-xs text-muted-foreground">{{ t("settings.files.maxTotalGbHint") }}</p>
                    <p v-if="errors.max_total_gb" class="text-xs text-destructive">
                        {{ errors.max_total_gb }}
                    </p>
                </div>

                <div class="space-y-1.5">
                    <Label for="cfg-cleanup-time">{{ t("settings.files.cleanupTime") }}<NotEffectiveBadge /></Label>
                    <div class="flex flex-wrap items-center gap-3">
                        <Select
                            :model-value="hasPreset(config.cleanup_time) ? config.cleanup_time : 'custom'"
                            @update:model-value="(v: unknown) => onCleanupPreset(v)"
                        >
                            <SelectTrigger class="w-44">
                                <SelectValue />
                            </SelectTrigger>
                            <SelectContent>
                                <SelectItem
                                    v-for="p in CLEANUP_PRESETS"
                                    :key="p"
                                    :value="p"
                                    >{{ p }} {{ t("settings.files.cleanupTimeDaily") }}</SelectItem
                                >
                                <SelectItem value="custom">{{
                                    t("settings.files.cleanupTimeCustom")
                                }}</SelectItem>
                            </SelectContent>
                        </Select>
                        <Input
                            v-if="!hasPreset(config.cleanup_time)"
                            id="cfg-cleanup-time"
                            v-model="config.cleanup_time"
                            type="time"
                            class="w-40"
                            :class="errors.cleanup_time ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                            :aria-invalid="!!errors.cleanup_time"
                        />
                    </div>
                    <p v-if="errors.cleanup_time" class="text-xs text-destructive">
                        {{ errors.cleanup_time }}
                    </p>
                </div>
            </div>
        </div>

        <!-- ── 磁盘空间保护 ── -->
        <div class="rounded-lg border p-4">
            <h3 class="mb-1 text-sm font-semibold">{{ t("settings.files.diskTitle") }}</h3>
            <p class="mb-3 text-xs text-muted-foreground">{{ t("settings.files.diskHint") }}</p>
            <div class="space-y-1.5">
                <Label for="cfg-disk-limit">{{ t("settings.files.diskSpaceLimitGb") }}</Label>
                <Input
                    id="cfg-disk-limit"
                    v-model="diskLimitText"
                    inputmode="numeric"
                    class="w-40"
                    :class="errors.disk_space_limit_gb || diskLimitInvalid ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                    :aria-invalid="!!errors.disk_space_limit_gb || diskLimitInvalid"
                />
                <p v-if="errors.disk_space_limit_gb" class="text-xs text-destructive">
                    {{ errors.disk_space_limit_gb }}
                </p>
            </div>
        </div>
    </div>
</template>
