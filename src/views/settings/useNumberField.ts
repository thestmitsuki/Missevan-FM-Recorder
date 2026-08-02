/**
 * 数字输入字段桥接：GlobalConfig 数字字段（number）↔ Input 文本（string）。
 *
 * - 合法数字文本 → 同步回 model（reactive 表单）
 * - 空文本 → model 置 0（配合校验提示必填/范围错误）
 * - 非法文本（如 "abc"）→ 保持 model 不变，返回 invalid=true 供红边框展示
 * - model 外部变化（加载/恢复默认）→ 回写文本
 */
import { ref, watch, type Ref } from "vue";

export function useNumberField(model: Ref<number>) {
    const text = ref(String(model.value ?? ""));
    const invalid = ref(false);

    watch(
        model,
        (v) => {
            const s = String(v ?? "");
            if (s !== text.value) {
                text.value = s;
            }
        },
        { immediate: false },
    );

    watch(text, (raw) => {
        const t = raw.trim();
        if (t === "") {
            invalid.value = false;
            if (model.value !== 0) model.value = 0;
            return;
        }
        if (!/^-?\d+$/.test(t)) {
            invalid.value = true; // 非法字符：不写回 model，红边框提示
            return;
        }
        invalid.value = false;
        const n = Number(t);
        if (!Number.isSafeInteger(n)) {
            invalid.value = true;
            return;
        }
        model.value = n;
    });

    return { text, invalid };
}
