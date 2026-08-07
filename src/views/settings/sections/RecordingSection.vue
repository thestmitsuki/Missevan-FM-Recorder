<script setup lang="ts">
/**
 * 录制分类（规格 7.2）：
 * 基本设置（输出目录/格式/比特率/仅音频/分段）、文件名模板（变量+实时预览）、
 * 并发录制限制、录制前延迟、录制后动作（无/打开文件夹/自定义命令）。
 * 字段：output_dir, record_format, bitrate_kbps, audio_only, segment_seconds,
 * filename_template, max_concurrent_recordings, pre_record_delay_secs,
 * post_record_action, post_record_command。
 */
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import type { SectionErrors, SettingsForm } from "../validation";
import { useNumberField } from "../useNumberField";
import { toRef } from "vue";
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
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Switch } from "@/components/ui/switch";

const props = defineProps<{
    config: SettingsForm;
    errors: SectionErrors;
}>();

const emit = defineEmits<{ browseOutputDir: [] }>();

const { t } = useI18n();

// ── 数字字段（文本 ↔ number 桥接；模板中需解构为顶层 ref 才能自动解包） ──
const { text: segmentSecondsText, invalid: segmentSecondsInvalid } = useNumberField(
    toRef(props.config, "segment_seconds"),
);
const { text: preRecordDelayText, invalid: preRecordDelayInvalid } = useNumberField(
    toRef(props.config, "pre_record_delay_secs"),
);
const { text: maxConcurrentText, invalid: maxConcurrentInvalid } = useNumberField(
    toRef(props.config, "max_concurrent_recordings"),
);

// 音频比特率（kbps）：新增 320 高码率档（前端优化任务）
const bitrateOptions = [64, 128, 192, 256, 320];

// ── 分段开关：segment_seconds > 0 视为启用 ──
const segmentEnabled = computed({
    get: () => props.config.segment_seconds > 0,
    set: (on: boolean) => {
        props.config.segment_seconds = on ? 60 : 0;
    },
});

// ── 文件名模板变量 ──
interface TemplateVar {
    token: string;
    labelKey: string;
    sample: string;
    /** 示例值走 i18n（其余示例为数字/日期，语言无关） */
    sampleKey?: string;
}

// ref 绑定 Input 组件实例（script setup 无 defineExpose）——按 { $el } 形态访问原生 input
const templateInput = ref<{ $el?: HTMLInputElement } | null>(null);
/** 模板输入框是否聚焦：无焦点时变量按钮回退为追加到行尾（未聚焦的 input
 *  selectionStart 恒为 0，直接插入会把变量塞到最前面） */
const templateFocused = ref(false);
const templateVars: TemplateVar[] = [
    { token: "{anchor_name}", labelKey: "settings.recording.tplVarAnchorName", sample: "sample", sampleKey: "settings.recording.tplSampleAnchorName" },
    { token: "{room_id}", labelKey: "settings.recording.tplVarRoomId", sample: "123456" },
    { token: "{date}", labelKey: "settings.recording.tplVarDate", sample: "2026-08-01" },
    { token: "{time}", labelKey: "settings.recording.tplVarTime", sample: "12-30-00" },
    { token: "{index}", labelKey: "settings.recording.tplVarIndex", sample: "001" },
    { token: "{ext}", labelKey: "settings.recording.tplVarExt", sample: "m4a" },
];

function insertVariable(token: string) {
    // ref 绑定的是 Input 组件实例（script setup 无 defineExpose）——用 $el 取
    // 其单根元素（原生 <input>）才能访问 selectionStart/focus/setSelectionRange
    const el = templateInput.value?.$el as HTMLInputElement | undefined;
    const current = props.config.filename_template;
    // 无输入框 / 输入框未聚焦（无真实光标）→ 回退追加到行尾
    if (!el || !templateFocused.value) {
        props.config.filename_template = current + token;
        return;
    }
    const start = el.selectionStart ?? current.length;
    const end = el.selectionEnd ?? current.length;
    props.config.filename_template =
        current.slice(0, start) + token + current.slice(end);
    // 光标移到插入内容之后（下一帧，等 v-model 值渲染进 DOM 后再设选区）
    requestAnimationFrame(() => {
        el.focus();
        const pos = start + token.length;
        el.setSelectionRange(pos, pos);
    });
}

/** 模板实时预览（规格：实时预览当前模板生成的示例文件名） */
const templatePreview = computed(() => {
    let out = props.config.filename_template || "";
    for (const v of templateVars) {
        out = out.split(v.token).join(v.sampleKey ? t(v.sampleKey) : v.sample);
    }
    return out;
});

const postActions = [
    { value: "none", labelKey: "settings.recording.postNone" },
    { value: "open_folder", labelKey: "settings.recording.postOpenFolder" },
    { value: "command", labelKey: "settings.recording.postCommand" },
];
</script>

<template>
    <div class="space-y-6">
        <!-- ── 基本设置 ── -->
        <div class="rounded-lg border p-4">
            <h3 class="mb-4 text-sm font-semibold">{{ t("settings.recording.basicTitle") }}</h3>
            <div class="space-y-4">
                <!-- 输出目录（必填） -->
                <div class="space-y-1.5">
                    <Label for="cfg-output-dir">{{ t("settings.recording.outputDir") }}</Label>
                    <div class="flex gap-2">
                        <Input
                            id="cfg-output-dir"
                            v-model="config.output_dir"
                            :class="errors.output_dir ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                            :aria-invalid="!!errors.output_dir"
                            :placeholder="t('settings.recording.outputDirPlaceholder')"
                        />
                        <Button variant="outline" @click="emit('browseOutputDir')">
                            {{ t("settings.recording.browse") }}
                        </Button>
                    </div>
                    <p v-if="errors.output_dir" class="text-xs text-destructive">
                        {{ errors.output_dir }}
                    </p>
                </div>

                <!-- 录制格式 -->
                <div class="space-y-2">
                    <Label>{{ t("settings.recording.recordFormat") }}</Label>
                    <RadioGroup v-model="config.record_format" class="flex gap-5">
                        <div class="flex items-center gap-2">
                            <RadioGroupItem id="cfg-fmt-m4a" value="m4a" class="size-4" />
                            <Label for="cfg-fmt-m4a">{{ t("settings.recording.formatM4A") }}</Label>
                        </div>
                        <div class="flex items-center gap-2">
                            <RadioGroupItem id="cfg-fmt-mp3" value="mp3" class="size-4" />
                            <Label for="cfg-fmt-mp3">{{ t("settings.recording.formatMP3") }}</Label>
                        </div>
                    </RadioGroup>
                </div>

                <!-- 音频比特率（shadcn Select，选项 64/128/192/256/320） -->
                <div class="space-y-1.5">
                    <Label for="cfg-bitrate">{{ t("settings.recording.bitrate") }}</Label>
                    <Select v-model="config.bitrate_kbps">
                        <SelectTrigger id="cfg-bitrate" class="w-40">
                            <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                            <SelectItem
                                v-for="b in bitrateOptions"
                                :key="b"
                                :value="b"
                                >{{ b }}kbps</SelectItem
                            >
                        </SelectContent>
                    </Select>
                    <p v-if="errors.bitrate_kbps" class="text-xs text-destructive">
                        {{ errors.bitrate_kbps }}
                    </p>
                </div>

                <!-- 仅录制音频流 -->
                <div class="flex items-center justify-between gap-4">
                    <div>
                        <Label for="cfg-audio-only">{{ t("settings.recording.audioOnly") }}</Label>
                        <p class="mt-0.5 text-xs text-muted-foreground">
                            {{ t("settings.recording.audioOnlyHint") }}
                        </p>
                    </div>
                    <Switch id="cfg-audio-only" v-model:checked="config.audio_only" />
                </div>

                <!-- 分段录制 -->
                <div class="flex items-center justify-between gap-4">
                    <div>
                        <Label for="cfg-segment-enable">{{ t("settings.recording.segmentEnable") }}</Label>
                        <p class="mt-0.5 text-xs text-muted-foreground">
                            {{ t("settings.recording.segmentHint") }}
                        </p>
                    </div>
                    <Switch id="cfg-segment-enable" v-model="segmentEnabled" />
                </div>
                <div class="space-y-1.5">
                    <Label for="cfg-segment-secs">{{ t("settings.recording.segmentSeconds") }}</Label>
                    <Input
                        id="cfg-segment-secs"
                        v-model="segmentSecondsText"
                        inputmode="numeric"
                        :class="errors.segment_seconds || segmentSecondsInvalid ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                        :aria-invalid="!!errors.segment_seconds || segmentSecondsInvalid"
                        :disabled="!segmentEnabled"
                    />
                    <p v-if="errors.segment_seconds" class="text-xs text-destructive">
                        {{ errors.segment_seconds }}
                    </p>
                </div>
            </div>
        </div>

        <!-- ── 文件名模板 ── -->
        <div class="rounded-lg border p-4">
            <h3 class="mb-1 text-sm font-semibold">
                {{ t("settings.recording.templateTitle") }}
            </h3>
            <p class="mb-3 text-xs text-muted-foreground">{{ t("settings.recording.templateDesc") }}</p>
            <div class="space-y-1.5">
                <Input
                    ref="templateInput"
                    v-model="config.filename_template"
                    @focus="templateFocused = true"
                    @blur="templateFocused = false"
                    :class="errors.filename_template ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                    :aria-invalid="!!errors.filename_template"
                    :placeholder="t('settings.recording.templatePlaceholder')"
                />
                <div class="flex flex-wrap items-center gap-1.5">
                    <span class="text-xs text-muted-foreground">{{ t("settings.recording.tplVars") }}:</span>
                    <Button
                        v-for="v in templateVars"
                        :key="v.token"
                        type="button"
                        size="xs"
                        variant="outline"
                        @click="insertVariable(v.token)"
                    >
                        {{ v.token }}
                    </Button>
                </div>
                <p v-if="errors.filename_template" class="text-xs text-destructive">
                    {{ errors.filename_template }}
                </p>
                <div
                    v-if="templatePreview"
                    class="rounded-md bg-muted/60 px-3 py-2 text-xs text-muted-foreground"
                >
                    <span class="mr-2 font-medium">{{ t("settings.recording.tplPreviewLabel") }}:</span>
                    <span class="break-all font-mono text-foreground">{{ templatePreview }}</span>
                </div>
            </div>
        </div>

        <!-- ── 并发录制限制 ── -->
        <div class="rounded-lg border p-4">
            <h3 class="mb-1 text-sm font-semibold">{{ t("settings.recording.concurrencyTitle") }}</h3>
            <p class="mb-3 text-xs text-muted-foreground">
                {{ t("settings.recording.concurrencyHint") }}
            </p>
            <div class="space-y-1.5">
                <Label for="cfg-max-concurrent">{{ t("settings.recording.maxConcurrent") }}</Label>
                <Input
                    id="cfg-max-concurrent"
                    v-model="maxConcurrentText"
                    inputmode="numeric"
                    class="w-40"
                    :class="errors.max_concurrent_recordings || maxConcurrentInvalid ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                    :aria-invalid="!!errors.max_concurrent_recordings || maxConcurrentInvalid"
                />
                <p v-if="errors.max_concurrent_recordings" class="text-xs text-destructive">
                    {{ errors.max_concurrent_recordings }}
                </p>
            </div>
        </div>

        <!-- ── 录制前延迟 ── -->
        <div class="rounded-lg border p-4">
            <h3 class="mb-1 text-sm font-semibold">{{ t("settings.recording.preDelayTitle") }}</h3>
            <p class="mb-3 text-xs text-muted-foreground">{{ t("settings.recording.preDelayHint") }}</p>
            <div class="space-y-1.5">
                <Label for="cfg-pre-delay">{{ t("settings.recording.preRecordDelay") }}</Label>
                <Input
                    id="cfg-pre-delay"
                    v-model="preRecordDelayText"
                    inputmode="numeric"
                    class="w-40"
                    :class="errors.pre_record_delay_secs || preRecordDelayInvalid ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                    :aria-invalid="!!errors.pre_record_delay_secs || preRecordDelayInvalid"
                />
                <p v-if="errors.pre_record_delay_secs" class="text-xs text-destructive">
                    {{ errors.pre_record_delay_secs }}
                </p>
            </div>
        </div>

        <!-- ── 录制后动作 ── -->
        <div class="rounded-lg border p-4">
            <h3 class="mb-3 text-sm font-semibold">
                {{ t("settings.recording.postActionTitle") }}
            </h3>
            <RadioGroup v-model="config.post_record_action" class="mb-4 flex flex-col gap-2">
                <div v-for="a in postActions" :key="a.value" class="flex items-center gap-2">
                    <RadioGroupItem
                        :id="`cfg-post-${a.value}`"
                        :value="a.value"
                        class="size-4"
                    />
                    <Label :for="`cfg-post-${a.value}`">{{ t(a.labelKey) }}</Label>
                </div>
            </RadioGroup>
            <div v-if="config.post_record_action === 'command'" class="space-y-1.5">
                <Label for="cfg-post-command">{{ t("settings.recording.postCommandLabel") }}</Label>
                <Input
                    id="cfg-post-command"
                    v-model="config.post_record_command"
                    :class="errors.post_record_command ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                    :aria-invalid="!!errors.post_record_command"
                    :placeholder="t('settings.recording.postCommandPlaceholder')"
                />
                <div class="space-y-1 rounded-md bg-muted/60 px-3 py-2 text-xs text-muted-foreground">
                    <p class="font-medium text-foreground">{{ t("settings.recording.postCommandHintTitle") }}</p>
                    <ul class="space-y-0.5">
                        <li>
                            <code class="rounded bg-background/80 px-1 font-mono">{file}</code>
                            <span class="ml-1">{{ t("settings.recording.postCommandVarFile") }}</span>
                        </li>
                        <li>
                            <code class="rounded bg-background/80 px-1 font-mono">{output_dir}</code>
                            <span class="ml-1">{{ t("settings.recording.postCommandVarOutputDir") }}</span>
                        </li>
                        <li>
                            <code class="rounded bg-background/80 px-1 font-mono">{anchor_name}</code>
                            <span class="ml-1">{{ t("settings.recording.postCommandVarAnchor") }}</span>
                        </li>
                        <li>
                            <code class="rounded bg-background/80 px-1 font-mono">{room_id}</code>
                            <span class="ml-1">{{ t("settings.recording.postCommandVarRoom") }}</span>
                        </li>
                    </ul>
                    <p>{{ t("settings.recording.postCommandHintShell") }}</p>
                    <p class="font-mono text-foreground">{{ t("settings.recording.postCommandHintExample") }}</p>
                </div>
                <p v-if="errors.post_record_command" class="text-xs text-destructive">
                    {{ errors.post_record_command }}
                </p>
            </div>
        </div>
    </div>
</template>
