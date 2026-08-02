/**
 * 固定标签常量（用户需求：标签只能从固定的五个中选择，禁止自由输入）。
 *
 * 最终清单（从原 8 个推荐标签中选定 5 个）：
 *   音乐 / 唱歌 / 日常 / ASMR / 杂谈
 * 裁撤：互动（与杂谈重叠）、游戏（窄众）、其他（兜底桶会稀释固定标签语义）。
 *
 * 数据结构说明：
 * - ANCHOR_TAGS       —— i18n 键数组（渲染标签名时 t(key) 翻译，zh-CN/en 同步维护）
 * - ANCHOR_TAG_VALUES —— 落盘规范值（中文文本）。anchor.tags / tagFilter /
 *   localStorage 持久化均使用该值：与 UI 语言无关（同一选择在任何界面语言下
 *   都写入同一值），且天然兼容历史中文标签数据（旧版「推荐标签快速选择」落盘
 *   的就是中文文本）。
 * - 两数组由同一 TAG_PAIRS 派生，键值按下标一一对应、不会漂移。
 */

const TAG_PAIRS = [
  ["live.tagMusic", "音乐"],
  ["live.tagSinging", "唱歌"],
  ["live.tagDaily", "日常"],
  ["live.tagASMR", "ASMR"],
  ["live.tagChat", "杂谈"],
] as const;

/** 固定标签 i18n 键（渲染时 t()；与 ANCHOR_TAG_VALUES 按下标一一对应） */
export const ANCHOR_TAGS: readonly string[] = TAG_PAIRS.map(([key]) => key);

/** 固定标签落盘规范值（用于匹配 anchor.tags / tagFilter 与持久化） */
export const ANCHOR_TAG_VALUES: readonly string[] = TAG_PAIRS.map(
  ([, value]) => value,
);

/** 判断某标签文本是否属于固定 5 标签（设置面板预选/保存过滤用） */
export function isPresetTag(value: string): boolean {
  return ANCHOR_TAG_VALUES.includes(value);
}
