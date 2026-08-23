/**
 * 语义化版本比较（原 AboutDialog 内部实现，抽出供 AboutDialog 与
 * updateStore 共用——纯函数、无副作用）。
 *
 * 语义：a > b → 1，a < b → -1，相等 → 0。
 * - "1.2" 与 "1.2.0" 视为相等（逐段数字比较，缺失段按 0）；
 * - 预发布后缀（如 "-beta.1"）被忽略——`parseInt("1-beta.1")` 取前导数字 1，
 *   与后端 `update_cmds::compare_versions` 行为一致。
 */
export function compareVersions(a: string, b: string): number {
    const pa = a.split(".").map((x) => parseInt(x, 10) || 0);
    const pb = b.split(".").map((x) => parseInt(x, 10) || 0);
    for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
        const diff = (pa[i] ?? 0) - (pb[i] ?? 0);
        if (diff !== 0) return diff > 0 ? 1 : -1;
    }
    return 0;
}
