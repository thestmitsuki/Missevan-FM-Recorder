<script setup lang="ts">
import type { CheckboxRootEmits, CheckboxRootProps } from "reka-ui"
import { computed, type HTMLAttributes } from "vue"
import { Check } from "@lucide/vue"
import { reactiveOmit } from "@vueuse/core"
import { CheckboxIndicator, CheckboxRoot } from "reka-ui"
import { cn } from "@/lib/utils"

/**
 * reka-ui 2.x 兼容层：CheckboxRoot 只认 `modelValue`/`update:modelValue`（radix-vue 1.x
 * 的 `checked` API 已废弃）。项目内旧用法写 `v-model:checked` / `:checked` +
 * `@update:checked`（设置页通知/外观分类等），此处声明 `checked` prop 并在两种语义间
 * 互转（`indeterminate` 直接透传，reka-ui 原生支持 modelValue === "indeterminate"）：
 * - 传入 `checked` → 桥接为 `modelValue`，其 `update:modelValue` 回发 `update:checked`；
 * - 未传 `checked` → 原样透传 modelValue 语义。
 */
const props = defineProps<
  CheckboxRootProps & { class?: HTMLAttributes["class"]; checked?: boolean | "indeterminate" }
>()

const emits = defineEmits<CheckboxRootEmits & { "update:checked": [value: boolean] }>()

const delegatedProps = reactiveOmit(props, "class", "checked", "modelValue")

const bridgedValue = computed<unknown>({
  get: () => (props.checked !== undefined ? props.checked : props.modelValue),
  set: (v: unknown) => {
    if (props.checked !== undefined) emits("update:checked", v as boolean)
    else emits("update:modelValue", v as boolean)
  },
})

/** reka-ui `update:modelValue` → 桥接回写（走 computed setter 分发到正确的 emit） */
function onRekaUpdate(value: unknown) {
  bridgedValue.value = value
}
</script>

<template>
  <CheckboxRoot
    v-slot="slotProps"
    data-slot="checkbox"
    v-bind="delegatedProps"
    :model-value="bridgedValue"
    @update:model-value="onRekaUpdate"
    :class="
      cn('peer border-input data-[state=checked]:bg-primary data-[state=checked]:text-primary-foreground data-[state=checked]:border-primary focus-visible:border-ring focus-visible:ring-ring/50 aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 aria-invalid:border-destructive size-4 shrink-0 rounded-[4px] border shadow-xs transition-shadow outline-none focus-visible:ring-3 disabled:cursor-not-allowed disabled:opacity-50',
         props.class)"
  >
    <CheckboxIndicator
      data-slot="checkbox-indicator"
      class="grid place-content-center text-current transition-none"
    >
      <slot v-bind="slotProps">
        <Check class="size-3.5" />
      </slot>
    </CheckboxIndicator>
  </CheckboxRoot>
</template>
