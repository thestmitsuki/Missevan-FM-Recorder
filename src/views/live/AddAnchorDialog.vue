<script setup lang="ts">
/**
 * 添加主播对话框（规格「添加主播」）
 *
 * URL 必填（校验 fm.missevan.com/live/<数字> 并提取房间号，无效显示红字错误）；
 * 自定义名称可选（留空后端自动获取）；检测开关默认开启；
 * 标签：只能从固定 5 个标签中选择（Checkbox 组多选，禁止自由输入；
 * 共享常量见 src/lib/anchorTags.ts，选择落盘为规范中文值）；
 * 保存调 add_anchor，成功后关闭并刷新列表。
 * 成功/失败通知由后端 dispatcher 推送（app:notification），本组件不重复造通知。
 */
import { ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { useAnchorStore } from "@/stores/anchorStore";
import { ANCHOR_TAGS, ANCHOR_TAG_VALUES } from "@/lib/anchorTags";
import { extractRoomId } from "@/services/liveUrl";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
    Dialog,
    DialogContent,
    DialogDescription,
    DialogFooter,
    DialogHeader,
    DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";

const open = defineModel<boolean>("open", { default: false });

const anchorStore = useAnchorStore();
const { t } = useI18n();

// ── 表单字段 ──
const url = ref("");
const name = ref("");
const enableCheck = ref(true);
/** 已勾选的固定标签（规范值，与 ANCHOR_TAG_VALUES 一致） */
const selectedTags = ref<string[]>([]);
const urlError = ref("");
const submitError = ref("");
const submitting = ref(false);

// 每次打开弹窗重置表单（默认开关为开）
watch(open, (isOpen) => {
    if (isOpen) {
        url.value = "";
        name.value = "";
        enableCheck.value = true;
        selectedTags.value = [];
        urlError.value = "";
        submitError.value = "";
    }
});

function validateUrl(): boolean {
    const value = url.value.trim();
    if (!value) {
        urlError.value = t("live.urlRequired");
        return false;
    }
    if (!extractRoomId(value)) {
        urlError.value = t("live.invalidUrl");
        return false;
    }
    urlError.value = "";
    return true;
}

/** Checkbox 勾选切换：从固定 5 标签中多选（规范值） */
function toggleTag(value: string, checked: boolean) {
    selectedTags.value = checked
        ? [...selectedTags.value, value]
        : selectedTags.value.filter((v) => v !== value);
}

async function handleSubmit() {
    if (!validateUrl()) return;
    submitError.value = "";
    submitting.value = true;
    try {
        const roomId = extractRoomId(url.value);
        if (!roomId) return;
        await anchorStore.addAnchor({
            id: crypto.randomUUID(),
            name: name.value.trim(),
            url: url.value.trim(),
            room_id: roomId,
            enable_check: enableCheck.value,
            tags: [...selectedTags.value],
            proxy: null,
            cookie: null,
        });
        open.value = false;
    } catch (e) {
        console.error("Failed to add anchor:", e);
        submitError.value = t("live.addFailed");
    } finally {
        submitting.value = false;
    }
}

function handleOpenChange(isOpen: boolean) {
    open.value = isOpen;
}
</script>

<template>
    <Dialog :open="open" @update:open="handleOpenChange">
        <DialogContent class="sm:max-w-[425px]">
            <DialogHeader>
                <DialogTitle>{{ t("live.addAnchor") }}</DialogTitle>
                <DialogDescription>
                    {{ t("live.noAnchorsDesc") }}
                </DialogDescription>
            </DialogHeader>

            <form class="flex flex-col gap-4" @submit.prevent="handleSubmit">
                <!-- 直播间 URL（必填） -->
                <div class="flex flex-col gap-1.5">
                    <Label for="add-url" class="text-sm font-medium">
                        {{ t("live.homepageUrl") }}
                    </Label>
                    <Input
                        id="add-url"
                        v-model="url"
                        type="url"
                        :placeholder="t('live.homepageUrlPlaceholder')"
                        :aria-invalid="!!urlError"
                        @blur="validateUrl"
                    />
                    <p
                        v-if="urlError"
                        class="text-xs text-destructive"
                        role="alert"
                    >
                        {{ urlError }}
                    </p>
                </div>

                <!-- 自定义名称（可选，留空自动获取） -->
                <div class="flex flex-col gap-1.5">
                    <Label for="add-name" class="text-sm font-medium">
                        {{ t("live.customName") }}
                    </Label>
                    <Input
                        id="add-name"
                        v-model="name"
                        :placeholder="t('live.customNamePlaceholder')"
                    />
                </div>

                <!-- 标签（固定 5 个多选，禁止自由输入） -->
                <div class="flex flex-col gap-1.5">
                    <Label class="text-sm font-medium">
                        {{ t("live.tags") }}
                    </Label>
                    <div class="flex flex-col gap-2">
                        <label
                            v-for="(key, i) in ANCHOR_TAGS"
                            :key="key"
                            class="flex cursor-pointer items-center gap-2 text-sm"
                        >
                            <Checkbox
                                :checked="
                                    selectedTags.includes(
                                        ANCHOR_TAG_VALUES[i],
                                    )
                                "
                                @update:checked="(v) =>
                                    toggleTag(ANCHOR_TAG_VALUES[i], v === true)"
                            />
                            <span>{{ t(key) }}</span>
                        </label>
                    </div>
                    <p class="text-xs text-muted-foreground">
                        {{ t("live.tagSelectHint") }}
                    </p>
                </div>

                <!-- 检测开关（默认开） -->
                <div
                    class="flex items-center justify-between rounded-lg border border-border/60 px-3 py-2.5"
                >
                    <Label class="text-sm font-medium" for="add-check">
                        {{ t("live.enableLiveCheck") }}
                    </Label>
                    <Switch id="add-check" v-model:checked="enableCheck" />
                </div>

                <p
                    v-if="submitError"
                    class="text-xs text-destructive"
                    role="alert"
                >
                    {{ submitError }}
                </p>

                <DialogFooter>
                    <Button
                        type="button"
                        variant="ghost"
                        :disabled="submitting"
                        @click="handleOpenChange(false)"
                    >
                        {{ t("live.cancel") }}
                    </Button>
                    <Button type="submit" :disabled="submitting">
                        {{ submitting ? t("live.adding") : t("live.add") }}
                    </Button>
                </DialogFooter>
            </form>
        </DialogContent>
    </Dialog>
</template>
