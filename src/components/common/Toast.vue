<template>
    <!-- Task 20 a11y：动态通知区域 aria-live（屏幕阅读器播报，规格 §7.1）
         基于 ui/sonner（vue-sonner）：Toaster 挂载于应用根，toast() 触发。
         注意：vue-sonner 的 Toaster 只渲染「订阅之后」publish 的 toast，
         因此 toast() 必须延迟到 Toaster 挂载完成后再调用（nextTick 兜底）。 -->
    <Toaster
        position="bottom-center"
        :duration="duration || 3000"
        :close-button="false"
        :rich-colors="false"
    />
</template>

<script setup lang="ts">
import { nextTick, watch } from "vue";
import { toast } from "vue-sonner";
import { Toaster } from "@/components/ui/sonner";

const props = defineProps<{
    message: string;
    /** 通知序号：同文案连续通知也强制刷新（App.vue 每次通知自增） */
    nonce?: number;
    duration?: number; // 毫秒
}>();

/** 固定 id：新通知替换旧通知，与旧版单条 Toast 语义一致（不堆叠） */
const TOAST_ID = "app-toast";

function showToast() {
    if (!props.message) {
        // 消息被清空（App.vue 收到清空通知）时立即收回已显示的 toast，
        // 避免残留到 duration 结束（审查 L2：清空不立即消失）。
        nextTick(() => toast.dismiss());
        return;
    }
    // 延迟到当前渲染冲刷结束：确保 Toaster 已挂载并订阅 store，
    // 否则 toast() 的 publish 落在零订阅者上会直接丢失（通知不显示的根因）。
    nextTick(() => {
        toast(props.message, {
            id: TOAST_ID,
            duration: props.duration || 3000,
        });
    });
}

// 监听 message + nonce：nonce 变化保证「相同文案的连续通知」也会重新弹出
watch(() => [props.message, props.nonce], showToast, { immediate: true });
</script>
