<script lang="ts">
/**
 * 快捷键分类（规格 7.7）——禁用态占位（H2：当前版本快捷键功能不实现）。
 *
 * 后端尚未注册全局快捷键（无 global-shortcut 接线），本分类仅作占位：
 * - 顶部展示「快捷键功能当前版本暂未启用」说明（disabledNotice 徽标 + 文案）；
 * - 列表仍展示三条快捷键的默认键位（DEFAULT_SHORTCUTS / 表单已有值），仅作参考；
 * - 编辑交互（重新绑定 / 清除 / 恢复默认 / 清除全部 / 按键捕获）全部移除；
 * - 设置页保存不再把快捷键写入落盘配置（normalizeConfig 剔除 shortcuts，
 *   SettingsView 恢复默认也不再触碰 form.shortcuts），旧配置字段保留向后兼容。
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
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
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
        <Card class="gap-0 rounded-lg p-4 shadow-none">
            <CardHeader class="mb-1 gap-0 p-0">
                <div class="flex items-center gap-2">
                    <CardTitle class="text-sm font-semibold">{{ t("settings.shortcuts.title") }}</CardTitle>
                    <!-- 占位标注（与调试页性能监控「实验性」徽标同风格） -->
                    <Badge class="bg-amber-500/15 text-amber-600 dark:text-amber-400">
                        {{ t("settings.shortcuts.placeholder") }}
                    </Badge>
                </div>
            </CardHeader>
            <CardContent class="p-0">
                <p class="mb-4 text-xs text-muted-foreground">{{ t("settings.shortcuts.desc") }}</p>

                <!-- 禁用态说明：当前版本快捷键功能未启用（H2 占位） -->
                <div
                    class="mb-4 flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2.5"
                    role="note"
                >
                    <span class="text-xs leading-relaxed text-amber-700 dark:text-amber-400">
                        {{ t("settings.shortcuts.disabledNotice") }}
                    </span>
                </div>

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
            </CardContent>
        </Card>
    </div>
</template>
