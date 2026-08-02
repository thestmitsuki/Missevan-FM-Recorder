<script lang="ts">
/**
 * 快捷键分类（规格 7.7）——占位展示（需求：快捷键不实现实际绑定）。
 *
 * 后端尚未注册全局快捷键（无 global-shortcut 接线），因此本分类仅作占位：
 * - 列表仍展示三条快捷键的默认键位（DEFAULT_SHORTCUTS / 表单已有值）；
 * - 顶部标注「暂未生效」徽标（与调试页性能监控占位样式一致）；
 * - 编辑交互（重新绑定 / 清除 / 恢复默认 / 清除全部 / 按键捕获）全部移除。
 *
 * 后续版本接入 tauri-plugin-global-shortcut 时，恢复「重新绑定」交互即可。
 */

export type ShortcutId = "toggle_window" | "toggle_recording" | "open_output_dir";

/** 默认绑定（SettingsView「恢复默认值」/「全部恢复默认」复用；<script setup> 不允许值导出，故放普通块） */
export const DEFAULT_SHORTCUTS: Record<ShortcutId, string> = {
    toggle_window: "Ctrl+Shift+M",
    toggle_recording: "Ctrl+R",
    open_output_dir: "Ctrl+Shift+O",
};

export const SHORTCUT_IDS: ShortcutId[] = [
    "toggle_window",
    "toggle_recording",
    "open_output_dir",
];
</script>

<script setup lang="ts">
import { useI18n } from "vue-i18n";
import type { SectionErrors, SettingsForm } from "../validation";
import { Badge } from "@/components/ui/badge";
import { Kbd, KbdGroup } from "@/components/ui/kbd";
import {
    Table,
    TableBody,
    TableCell,
    TableHead,
    TableHeader,
    TableRow,
} from "@/components/ui/table";

const props = defineProps<{
    config: SettingsForm;
    errors: SectionErrors;
}>();

const { t } = useI18n();

/** 展示键位：表单已绑定的值优先，未设置则显示默认键位（占位展示） */
const comboOf = (id: ShortcutId) => props.config.shortcuts?.[id] || DEFAULT_SHORTCUTS[id];

const functionLabel = (id: ShortcutId) => t(`settings.shortcuts.${id}`);

function comboParts(combo: string): string[] {
    return combo.split("+").filter(Boolean);
}
</script>

<template>
    <div class="space-y-6">
        <div class="rounded-lg border p-4">
            <div class="mb-1 flex items-center gap-2">
                <h3 class="text-sm font-semibold">{{ t("settings.shortcuts.title") }}</h3>
                <!-- 占位标注（与调试页性能监控「实验性」徽标同风格） -->
                <Badge class="bg-amber-500/15 text-amber-600 dark:text-amber-400">
                    {{ t("settings.shortcuts.placeholder") }}
                </Badge>
            </div>
            <p class="mb-4 text-xs text-muted-foreground">{{ t("settings.shortcuts.desc") }}</p>

            <div class="rounded-md border">
                <Table>
                    <TableHeader>
                        <TableRow class="hover:bg-transparent">
                            <TableHead>{{ t("settings.shortcuts.function") }}</TableHead>
                            <TableHead>{{ t("settings.shortcuts.combo") }}</TableHead>
                        </TableRow>
                    </TableHeader>
                    <TableBody>
                        <TableRow v-for="id in SHORTCUT_IDS" :key="id">
                            <TableCell class="font-medium">{{ functionLabel(id) }}</TableCell>
                            <TableCell>
                                <KbdGroup>
                                    <Kbd v-for="(k, i) in comboParts(comboOf(id))" :key="i">{{ k }}</Kbd>
                                </KbdGroup>
                            </TableCell>
                        </TableRow>
                    </TableBody>
                </Table>
            </div>

            <p class="mt-4 text-xs text-muted-foreground">{{ t("settings.shortcuts.note") }}</p>
        </div>
    </div>
</template>
