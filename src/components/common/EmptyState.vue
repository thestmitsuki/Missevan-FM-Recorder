<script setup lang="ts">
// Linux 无 emoji 字体（不引入字体依赖）：icon prop 均为 emoji 字符，
// 按平台渲染——Windows 显示 emoji，Linux 不渲染（避免方框/空白）
import { isWindowsPlatform } from "@/services/platform";

defineProps<{
    icon?: string;
    title: string;
    description?: string;
    actionLabel?: string;
}>();

defineEmits<{
    action: [];
}>();
</script>

<template>
    <div class="empty-state-wrapper">
        <div class="empty-state-card">
            <div class="empty-state-content">
                <span v-if="icon && isWindowsPlatform()" class="empty-icon">{{ icon }}</span>
                <h2 class="empty-title">{{ title }}</h2>
                <p v-if="description" class="empty-description">
                    {{ description }}
                </p>
                <button
                    v-if="actionLabel"
                    class="empty-action"
                    @click="$emit('action')"
                >
                    {{ actionLabel }}
                </button>
            </div>
        </div>
    </div>
</template>

<style scoped>
.empty-state-wrapper {
    display: flex;
    justify-content: center;
    align-items: center;
    min-height: 300px;
    padding: 24px;
}

.empty-state-card {
    max-width: 480px;
    width: 100%;
    padding: 48px 32px 40px;
    text-align: center;
    border-radius: 28px;
}

.empty-state-content {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 16px;
}

.empty-icon {
    font-size: 48px;
    line-height: 1;
    color: var(--md-sys-color-primary, #6750a4);
}

.empty-title {
    margin: 0;
    font-size: 24px;
    font-weight: 400;
    line-height: 1.25;
    color: var(--md-sys-color-on-surface, #1c1b1f);
}

.empty-description {
    margin: 0 0 8px;
    font-size: 16px;
    font-weight: 400;
    line-height: 1.5;
    color: var(--md-sys-color-on-surface-variant, #49454f);
}

.empty-action {
    padding: 8px 24px;
    border: none;
    border-radius: 20px;
    font-size: 14px;
    font-weight: 500;
    color: #fff;
    background: var(--md-sys-color-primary, #6750a4);
    cursor: pointer;
}

.empty-action:hover {
    filter: brightness(1.05);
}
</style>
