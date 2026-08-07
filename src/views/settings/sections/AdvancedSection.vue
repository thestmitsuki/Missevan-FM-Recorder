<script setup lang="ts">
/**
 * 高级分类（规格 7.8）：
 * 日志级别、直播检测并发限制、检测间隔、随机抖动范围、FFmpeg/ffprobe
 * 自定义路径（输入+浏览）、导出/导入配置、重置所有设置。
 *
 * - 随机抖动范围：Task 20 迁移——由 localStorage 改为 GlobalConfig.detector_jitter_secs
 *   （Task 14 后端已接线 detector/loop.rs，0 = 不抖动）。
 * - 导出（Task 20 收尾接 export_config）：后端生成 JSON（含主播列表，
 *   proxy_password/cookie 敏感字段置空）+ 复制到剪贴板；
 * - 导入：文件选择器读取 JSON → 后端 import_config（替换/合并，父级处理）；
 * - 重置：后端 reset_config（删配置 + 重启应用，父级确认）。
 */
import { ref, toRef } from "vue";
import { useI18n } from "vue-i18n";
import { Download, Upload } from "@lucide/vue";
import { api } from "@/services/api";
import type { SectionErrors, SettingsForm } from "../validation";
import { useNumberField } from "../useNumberField";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import {
    Dialog,
    DialogContent,
    DialogDescription,
    DialogFooter,
    DialogHeader,
    DialogTitle,
} from "@/components/ui/dialog";

const props = defineProps<{
    config: SettingsForm;
    errors: SectionErrors;
}>();

const emit = defineEmits<{
    /** 浏览 FFmpeg/ffprobe 可执行文件（父级打开系统文件对话框） */
    browse: [kind: "ffmpeg" | "ffprobe"];
    /** 导入配置文件（父级打开文件选择器读取 JSON） */
    importConfig: [];
    /** 重置所有设置（父级确认对话框 → 后端 reset_config：删配置 + 重启） */
    resetConfig: [];
}>();

const { t } = useI18n();


const { text: detectorText, invalid: detectorInvalid } = useNumberField(
    toRef(props.config, "detector_concurrency"),
);
const { text: intervalText, invalid: intervalInvalid } = useNumberField(
    toRef(props.config, "check_interval_secs"),
);
// Task 20：随机抖动迁移至 GlobalConfig.detector_jitter_secs（0 = 不抖动）
const { text: jitterText, invalid: jitterInvalid } = useNumberField(
    toRef(props.config, "detector_jitter_secs"),
);

const logLevels = ["error", "warn", "info", "debug", "trace"] as const;
const logLevelLabel = (lv: (typeof logLevels)[number]) =>
    t(`settings.advanced.logLevel.${lv}`);

// ── 导出预览对话框 ──
const exportOpen = ref(false);
const exportCopied = ref(false);
const exportJson = ref("");

async function openExport() {
    // Task 20 收尾：改调后端 export_config（含主播列表；proxy_password/cookie 已置空）。
    // 剪贴板预览流程保留——后端返回 JSON 后同样走预览对话框 + 复制
    try {
        exportJson.value = await api.exportConfig();
        exportCopied.value = false;
        exportOpen.value = true;
    } catch {
        // 后端不可用时降级为客户端快照（仅全局字段，proxy_password 置空）
        const safe: Record<string, unknown> = { ...props.config, proxy_password: "" };
        exportJson.value = JSON.stringify(safe, null, 2);
        exportCopied.value = false;
        exportOpen.value = true;
    }
}

async function copyExport() {
    try {
        await navigator.clipboard.writeText(exportJson.value);
        exportCopied.value = true;
    } catch {
        exportCopied.value = false;
    }
}

</script>

<template>
    <div class="space-y-6">
        <!-- ── 日志级别 ── -->
        <div class="rounded-lg border p-4">
            <h3 class="mb-1 text-sm font-semibold">
                {{ t("settings.advanced.logLevelTitle") }}
            </h3>
            <p class="mb-3 text-xs text-muted-foreground">{{ t("settings.advanced.logLevelHint") }}</p>
            <Select v-model="config.log_level">
                <SelectTrigger class="w-44">
                    <SelectValue />
                </SelectTrigger>
                <SelectContent>
                    <SelectItem v-for="lv in logLevels" :key="lv" :value="lv">
                        {{ logLevelLabel(lv) }}
                    </SelectItem>
                </SelectContent>
            </Select>
        </div>

        <!-- ── 直播检测 ── -->
        <div class="rounded-lg border p-4">
            <h3 class="mb-4 text-sm font-semibold">{{ t("settings.advanced.detectionTitle") }}</h3>
            <div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
                <div class="space-y-1.5">
                    <Label for="cfg-detector-concurrency">{{ t("settings.advanced.detectorConcurrency") }}</Label>
                    <Input
                        id="cfg-detector-concurrency"
                        v-model="detectorText"
                        inputmode="numeric"
                        :class="errors.detector_concurrency || detectorInvalid ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                        :aria-invalid="!!errors.detector_concurrency || detectorInvalid"
                    />
                    <p v-if="errors.detector_concurrency" class="text-xs text-destructive">
                        {{ errors.detector_concurrency }}
                    </p>
                </div>
                <div class="space-y-1.5">
                    <Label for="cfg-check-interval">{{ t("settings.advanced.checkInterval") }}</Label>
                    <Input
                        id="cfg-check-interval"
                        v-model="intervalText"
                        inputmode="numeric"
                        :class="errors.check_interval_secs || intervalInvalid ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                        :aria-invalid="!!errors.check_interval_secs || intervalInvalid"
                    />
                    <p v-if="errors.check_interval_secs" class="text-xs text-destructive">
                        {{ errors.check_interval_secs }}
                    </p>
                </div>
            </div>
            <div class="mt-4 space-y-1.5">
                <Label for="cfg-jitter">{{ t("settings.advanced.jitter") }}</Label>
                <Input
                    id="cfg-jitter"
                    v-model="jitterText"
                    inputmode="numeric"
                    class="w-40"
                    :class="errors.detector_jitter_secs || jitterInvalid ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                    :aria-invalid="!!errors.detector_jitter_secs || jitterInvalid"
                />
                <p v-if="errors.detector_jitter_secs" class="text-xs text-destructive">
                    {{ errors.detector_jitter_secs }}
                </p>
                <p v-else class="text-xs text-muted-foreground">{{ t("settings.advanced.jitterHint") }}</p>
            </div>
        </div>

        <!-- ── FFmpeg / ffprobe 路径 ── -->
        <div class="rounded-lg border p-4">
            <h3 class="mb-4 text-sm font-semibold">{{ t("settings.advanced.pathsTitle") }}</h3>
            <div class="space-y-4">
                <div class="space-y-1.5">
                    <Label for="cfg-ffmpeg-path">{{ t("settings.advanced.ffmpegPath") }}</Label>
                    <div class="flex gap-2">
                        <Input
                            id="cfg-ffmpeg-path"
                            v-model="config.ffmpeg_path"
                            :class="errors.ffmpeg_path ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                            :aria-invalid="!!errors.ffmpeg_path"
                            :placeholder="t('settings.advanced.ffmpegPathPlaceholder')"
                        />
                        <Button variant="outline" @click="emit('browse', 'ffmpeg')">
                            {{ t("settings.advanced.browse") }}
                        </Button>
                    </div>
                    <p v-if="errors.ffmpeg_path" class="text-xs text-destructive">
                        {{ errors.ffmpeg_path }}
                    </p>
                </div>
                <div class="space-y-1.5">
                    <Label for="cfg-ffprobe-path">{{ t("settings.advanced.ffprobePath") }}</Label>
                    <div class="flex gap-2">
                        <Input
                            id="cfg-ffprobe-path"
                            v-model="config.ffprobe_path"
                            :class="errors.ffprobe_path ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                            :aria-invalid="!!errors.ffprobe_path"
                            :placeholder="t('settings.advanced.ffprobePathPlaceholder')"
                        />
                        <Button variant="outline" @click="emit('browse', 'ffprobe')">
                            {{ t("settings.advanced.browse") }}
                        </Button>
                    </div>
                    <p v-if="errors.ffprobe_path" class="text-xs text-destructive">
                        {{ errors.ffprobe_path }}
                    </p>
                </div>
            </div>
        </div>

        <!-- ── 配置操作 ── -->
        <div class="rounded-lg border p-4">
            <h3 class="mb-4 text-sm font-semibold">{{ t("settings.advanced.actionsTitle") }}</h3>
            <div class="flex flex-wrap gap-2">
                <Button variant="outline" @click="openExport">
                    <Download />
                    {{ t("settings.advanced.exportBtn") }}
                </Button>
                <Button variant="outline" @click="emit('importConfig')">
                    <Upload />
                    {{ t("settings.advanced.importBtn") }}
                </Button>
                <Button variant="destructive" @click="emit('resetConfig')">
                    {{ t("settings.advanced.resetBtn") }}
                </Button>
            </div>
            <p class="mt-3 text-xs text-muted-foreground">{{ t("settings.advanced.actionsHint") }}</p>
        </div>

        <!-- ── 导出预览对话框 ── -->
        <Dialog v-model:open="exportOpen">
            <DialogContent class="max-w-lg">
                <DialogHeader>
                    <DialogTitle>{{ t("settings.advanced.exportDialogTitle") }}</DialogTitle>
                    <DialogDescription>
                        {{ t("settings.advanced.exportDialogDesc") }}
                    </DialogDescription>
                </DialogHeader>
                <pre
                    class="max-h-64 overflow-auto rounded-md bg-muted/60 p-3 text-xs text-muted-foreground whitespace-pre-wrap break-all"
                >{{ exportJson }}</pre>
                <DialogFooter>
                    <Button variant="outline" size="sm" @click="copyExport">
                        {{ exportCopied ? t("settings.advanced.copied") : t("settings.advanced.copy") }}
                    </Button>
                    <Button size="sm" @click="exportOpen = false">
                        {{ t("common.close") }}
                    </Button>
                </DialogFooter>
            </DialogContent>
        </Dialog>
    </div>
</template>
