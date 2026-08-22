<script setup lang="ts">
/**
 * 文件管理分类（规格 7.3）：
 * 自动清理（开关 + 保留天数 + 保留总量上限 GB）、磁盘空间保护（阈值 GB）。
 * 字段：auto_cleanup_enabled, retention_days, max_total_gb, disk_space_limit_gb。
 *（cleanup_time「每日定时清理时间」已废弃：自动清理改为每次录制结束时触发，
 *  见后端 monitor.rs cleanup_on_recording_end；字段保留仅为旧配置兼容，不再展示）
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
import { Switch } from "@/components/ui/switch";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

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
</script>

<template>
    <div class="space-y-6">
        <!-- ── 自动清理 ── -->
        <Card class="gap-0 rounded-lg p-4 shadow-none">
            <CardContent class="p-0">
                <div class="mb-4 flex items-center justify-between gap-4">
                    <div>
                        <Label for="cfg-cleanup-enable">{{ t("settings.files.cleanupEnable") }}</Label>
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
                </div>
            </CardContent>
        </Card>

        <!-- ── 磁盘空间保护 ── -->
        <Card class="gap-0 rounded-lg p-4 shadow-none">
            <CardHeader class="mb-3 gap-1 p-0">
                <CardTitle class="text-sm font-semibold">{{ t("settings.files.diskTitle") }}</CardTitle>
                <p class="text-xs text-muted-foreground">{{ t("settings.files.diskHint") }}</p>
            </CardHeader>
            <CardContent class="p-0">
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
            </CardContent>
        </Card>
    </div>
</template>
