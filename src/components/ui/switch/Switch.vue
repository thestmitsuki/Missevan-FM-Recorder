<script setup lang="ts">
import type { SwitchRootEmits, SwitchRootProps } from "reka-ui"
import { computed, type HTMLAttributes } from "vue"
import { reactiveOmit } from "@vueuse/core"
import { SwitchRoot, SwitchThumb } from "reka-ui"
import { cn } from "@/lib/utils"

/**
 * reka-ui 2.x 兼容层：SwitchRoot 只认 `modelValue`/`update:modelValue`（radix-vue 1.x
 * 的 `checked` API 已废弃）。项目内旧用法（shadcn-vue 1.x 风格）写 `v-model:checked`
 * 或 `:checked` + `@update:checked`，此处声明 `checked` prop 并在两种语义间互转：
 * - 传入 `checked`（v-model:checked 编译产物）→ 桥接为 `modelValue` 交给 reka-ui，
 *   其 `update:modelValue` 回发为 `update:checked`；
 * - 未传 `checked`（v-model="..." 的 modelValue 语义）→ 原样透传 modelValue。
 */
const props = defineProps<
  SwitchRootProps & { class?: HTMLAttributes["class"]; checked?: boolean }
>()

const emits = defineEmits<SwitchRootEmits & { "update:checked": [value: boolean] }>()

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
  <SwitchRoot
    v-slot="slotProps"
    data-slot="switch"
    v-bind="delegatedProps"
    :model-value="bridgedValue"
    @update:model-value="onRekaUpdate"
    :class="cn(
      'peer data-[state=checked]:bg-primary data-[state=unchecked]:bg-input focus-visible:border-ring focus-visible:ring-ring/50 dark:data-[state=unchecked]:bg-input/80 inline-flex h-[1.15rem] w-8 shrink-0 items-center rounded-full border border-transparent shadow-xs transition-all outline-none focus-visible:ring-3 disabled:cursor-not-allowed disabled:opacity-50',
      props.class,
    )"
  >
    <SwitchThumb
      data-slot="switch-thumb"
      :class="cn('bg-background dark:data-[state=unchecked]:bg-foreground dark:data-[state=checked]:bg-primary-foreground pointer-events-none block size-4 rounded-full ring-0 transition-transform data-[state=checked]:translate-x-[calc(100%-2px)] data-[state=unchecked]:translate-x-0')"
    >
      <slot name="thumb" v-bind="slotProps" />
    </SwitchThumb>
  </SwitchRoot>
</template>
