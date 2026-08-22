<script setup lang="ts">
// Linux 无 emoji 字体（不引入字体依赖）：icon prop 均为 emoji 字符，
// 按平台渲染——Windows 显示 emoji，Linux 不渲染（避免方框/空白）
import { isWindowsPlatform } from "@/services/platform";
import {
    Empty,
    EmptyContent,
    EmptyDescription,
    EmptyTitle,
} from "@/components/ui/empty";
import { Button } from "@/components/ui/button";

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
    <Empty class="min-h-72 w-full">
        <EmptyContent>
            <span
                v-if="icon && isWindowsPlatform()"
                class="text-5xl leading-none"
                aria-hidden="true"
            >
                {{ icon }}
            </span>
            <EmptyTitle>{{ title }}</EmptyTitle>
            <EmptyDescription v-if="description">
                {{ description }}
            </EmptyDescription>
            <Button v-if="actionLabel" @click="$emit('action')">
                {{ actionLabel }}
            </Button>
        </EmptyContent>
    </Empty>
</template>
