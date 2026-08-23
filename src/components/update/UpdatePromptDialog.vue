<script setup lang="ts">
/**
 * 启动自动检查更新的全局弹窗（挂在 App.vue 根组件，AlertDialog Teleport 到
 * body——网络慢时用户已切到任意路由页面，检查完成后仍能正常弹出）。
 *
 * 显示条件由 updateStore 控制（check_updates 开启且 notify_update 开启且
 * 检测到新版本）；「稍后再说」关闭，本会话不再重复弹出。
 */
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
import { useUpdateStore } from "@/stores/updateStore";

const { t } = useI18n();
const updateStore = useUpdateStore();
</script>

<template>
    <AlertDialog
        :open="updateStore.promptOpen"
        @update:open="(v) => { if (!v) updateStore.dismiss(); }"
    >
        <AlertDialogContent>
            <AlertDialogHeader>
                <AlertDialogTitle>{{ t("update.promptTitle") }}</AlertDialogTitle>
                <AlertDialogDescription>
                    {{ t("update.promptBody", { version: updateStore.latest }) }}
                </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
                <AlertDialogCancel @click="updateStore.dismiss()">
                    {{ t("update.later") }}
                </AlertDialogCancel>
                <AlertDialogAction @click="updateStore.openDownload()">
                    {{ t("update.download") }}
                </AlertDialogAction>
            </AlertDialogFooter>
        </AlertDialogContent>
    </AlertDialog>
</template>
