<script setup lang="ts">
import type { PrimitiveProps } from "reka-ui"
import type { ButtonHTMLAttributes, HTMLAttributes } from "vue"
import type { ButtonVariants } from "./variants"
import { Primitive } from "reka-ui"
import { cn } from "@/lib/utils"
import { buttonVariants } from "./variants"

interface Props extends PrimitiveProps {
  variant?: ButtonVariants["variant"]
  size?: ButtonVariants["size"]
  /** 原生 button type：默认 "button"，避免表单内误提交；提交按钮显式传 "submit" */
  type?: ButtonHTMLAttributes["type"]
  class?: HTMLAttributes["class"]
}

const props = withDefaults(defineProps<Props>(), {
  as: "button",
  type: "button",
})
</script>

<template>
  <Primitive
    data-slot="button"
    :data-variant="variant"
    :data-size="size"
    :type="type"
    :as="as"
    :as-child="asChild"
    :class="cn(buttonVariants({ variant, size }), props.class)"
  >
    <slot />
  </Primitive>
</template>
