<template>
    <Transition name="fade">
        <!-- Task 20 a11y：动态通知区域 aria-live（屏幕阅读器播报，规格 §7.1） -->
        <div v-if="visible" class="toast" role="status" aria-live="polite">
            {{ message }}
        </div>
    </Transition>
</template>

<script setup lang="ts">
import { ref, watch } from "vue";

const props = defineProps<{
    message: string;
    duration?: number; // 毫秒
}>();

const visible = ref(false);

watch(
    () => props.message,
    (newMsg) => {
        if (newMsg) {
            visible.value = true;
            setTimeout(() => {
                visible.value = false;
            }, props.duration || 3000);
        } else {
            visible.value = false; // 空消息直接隐藏
        }
    },
    { immediate: true },
);
</script>

<style scoped>
.toast {
    position: fixed;
    bottom: 30px;
    left: 50%;
    transform: translateX(-50%);
    background: #333;
    color: #fff;
    padding: 12px 24px;
    border-radius: 8px;
    font-size: 14px;
    z-index: 9999;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
    max-width: 80%;
}
.fade-enter-active,
.fade-leave-active {
    transition: opacity 0.3s ease;
}
.fade-enter-from,
.fade-leave-to {
    opacity: 0;
}
</style>
