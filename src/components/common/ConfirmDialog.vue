<script setup lang="ts">
/**
 * 通用确认对话框（基于 ui/alert-dialog）
 *
 * 受控组件：open 由父组件控制。点击「确认」「取消」分别上抛
 * confirm / cancel；ESC 或点击遮罩关闭时上抛 cancel。
 * 纯展示组件，不做任何业务操作，由父组件在事件回调中处理并关闭。
 */
import { ref } from "vue";
import { useI18n } from "vue-i18n";

import {
    AlertDialog,
    AlertDialogAction,
    AlertDialogCancel,
    AlertDialogContent,
    AlertDialogDescription,
    AlertDialogFooter,
    AlertDialogHeader,
    AlertDialogTitle,
} from "@/components/ui/alert-dialog";

withDefaults(
    defineProps<{
        /** 是否打开（受控） */
        open: boolean;
        /** 标题 */
        title: string;
        /** 说明文案 */
        message: string;
        /** 确认按钮文案（默认取 i18n common.confirm） */
        confirmText?: string;
        /** 取消按钮文案（默认取 i18n common.cancel） */
        cancelText?: string;
        /** 确认按钮使用危险色 */
        destructive?: boolean;
    }>(),
    {
        confirmText: "",
        cancelText: "",
        destructive: false,
    },
);

const emit = defineEmits<{
    confirm: [];
    cancel: [];
}>();

const { t } = useI18n();

/**
 * 抑制重复事件：AlertDialogAction/Cancel 点击后会自动关闭对话框，
 * 触发 update:open(false)，此时不应再上抛一次 cancel。
 *
 * 时序说明（修复「删除需点两次」根因）：reka-ui 的 Action/Cancel 内部 onClick
 * （onOpenChange(false)）注册在业务 @click 处理器**之前**，且 useVModel 的 setter
 * 会**同步** emit update:open(false)——即单击「确认」时 handleOpenChange(false)
 * 先于 handleConfirm 执行，此时 acted 仍为 false，若直接 emit("cancel") 会误报
 * cancel（父组件清空删除目标 → confirm 执行落空，文件删不掉，用户需再点一轮）。
 * 修复：被动关闭的 cancel 上抛延迟到本次事件循环结束后（setTimeout 0），
 * 让同一 click 内的 confirm/cancel 处理器先置位 acted；若无任何按钮处理器
 * （ESC / 遮罩点击等真实被动关闭）才补发 cancel。
 */
const acted = ref(false);
/** 已安排待补发的 cancel（防止同一轮关闭中重复补发） */
let cancelPending = false;

function handleOpenChange(open: boolean) {
    if (open) {
        acted.value = false;
        cancelPending = false;
        return;
    }
    if (!acted.value && !cancelPending) {
        cancelPending = true;
        setTimeout(() => {
            cancelPending = false;
            // 确认/取消按钮的处理器已在本轮 click 内同步执行 → 不再补发 cancel
            if (!acted.value) emit("cancel"); // ESC / 遮罩点击等被动关闭
            acted.value = false;
        }, 0);
    } else {
        acted.value = false;
    }
}

function handleConfirm() {
    acted.value = true;
    emit("confirm");
}

function handleCancel() {
    acted.value = true;
    emit("cancel");
}
</script>

<template>
    <AlertDialog :open="open" @update:open="handleOpenChange">
        <AlertDialogContent class="max-w-sm">
            <AlertDialogHeader>
                <AlertDialogTitle>{{ title }}</AlertDialogTitle>
                <AlertDialogDescription v-if="message">
                    {{ message }}
                </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
                <AlertDialogCancel @click="handleCancel">
                    {{ cancelText || t("common.cancel") }}
                </AlertDialogCancel>
                <AlertDialogAction
                    :class="
                        destructive
                            ? 'bg-destructive text-white hover:bg-destructive/90'
                            : ''
                    "
                    @click="handleConfirm"
                >
                    {{ confirmText || t("common.confirm") }}
                </AlertDialogAction>
            </AlertDialogFooter>
        </AlertDialogContent>
    </AlertDialog>
</template>
