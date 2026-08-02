<script setup lang="ts">
/**
 * 主内容容器：占满 TopBar 以下全部空间（flex 拉伸），不滚动、不加留白——
 * 滚动与留白由各页面自行管理（页面根节点 = 「竖向操作栏 + 滚动内容区」）。
 * 这样操作栏高度 = 主内容区容器全高（flex 拉伸），贴左侧无间隙。
 */
</script>

<template>
    <div class="page-container flex min-h-0 min-w-0 flex-1 overflow-hidden">
        <slot />
    </div>
</template>
