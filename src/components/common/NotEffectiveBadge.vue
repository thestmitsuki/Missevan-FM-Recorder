<script setup lang="ts">
/**
 * 「暂未生效」徽标：设置页中后端尚未运行时接线的字段提示（Task 20 收尾 I-2.4）。
 * 文案统一走 i18n 键 settings.notEffective（zh/en 同步）。
 */
import { useI18n } from "vue-i18n";

const { t } = useI18n();
</script>

<template>
    <span
        class="ml-1.5 inline-block rounded bg-muted px-1.5 py-0.5 align-middle text-[10px] font-normal text-muted-foreground"
        :title="t('settings.notEffective')"
    >
        {{ t("settings.notEffective") }}
    </span>
</template>
