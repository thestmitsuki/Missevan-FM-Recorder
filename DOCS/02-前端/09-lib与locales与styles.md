# 09 · lib / locales / styles —— 工具库、国际化与样式

> 文件：`src/lib/*`、`src/locales/*`、`src/styles/*`

## 1. lib —— 纯函数工具（零业务依赖，可单测）

### anchorTags.ts —— 固定 5 标签

```ts
const TAG_PAIRS = [["live.tagMusic","音乐"],["live.tagSinging","唱歌"],["live.tagDaily","日常"],["live.tagASMR","ASMR"],["live.tagChat","杂谈"]];
export const ANCHOR_TAGS: readonly string[];        // i18n 键（渲染用 t()）
export const ANCHOR_TAG_VALUES: readonly string[];  // 落盘中文规范值（匹配/持久化）
export function isPresetTag(value: string): boolean;
```

- 两数组由同一 `TAG_PAIRS` 派生（下标一一对应，不漂移）；
- 落盘用**中文规范值**：与 UI 语言无关（同一选择任何语言下写同一值），天然兼容历史中文标签数据；
- 用户需求约束：标签只能从固定 5 个中选择，禁止自由输入（原 8 个推荐标签裁撤为 5 个：互动/游戏/其他被移除）。

### debounce.ts —— 去抖

- 手写零依赖：`debounce(fn, delay)` → `{ cancel, flush }`；
- 语义与 lodash 核心一致（连续调用只执行最后一次）；
- 用于搜索/筛选输入高频场景；组件卸载时 `cancel()`（防卸载后写 store）。

### virtualList.ts —— 固定行高虚拟列表

- `computeVisibleRange(items, scrollTop, viewportHeight, overscan=6)`：二分查找首条可见 → 顺序扫描末条 → 闭区间 + overscan；
- 前置条件：`items` 按 `top` 严格递增（扁平化时逐项累加行高）；
- 用于文件列表等大列表（DOM 数量与数据总量解耦）。

### utils.ts

- `cn(...inputs)`：`clsx` + `tailwind-merge` 类名合并（ui 组件变体组合标准工具）。

## 2. locales —— 国际化

- `index.ts`：`createI18n({ legacy:false, locale: readLocale(), fallbackLocale:"zh-CN" })`；`AppLocale = "zh-CN" | "en"`；偏好存 localStorage `locale`。
- `zh-CN.ts` / `en.ts`：约 35KB 各一份，键按页面/域组织（nav.* / live.* / files.* / settings.* / debug.* / wizard.* / notification.* / common.* / errors.* 等）。
- 语言切换：设置页「常规」→ 即时生效（i18n.global.locale）并持久化；主播标签渲染用 `ANCHOR_TAGS` i18n 键。

## 3. styles —— 样式体系

| 文件 | 内容 |
| --- | --- |
| `index.css` | Tailwind 4 入口（`@import "tailwindcss"` + 自定义 utilities） |
| `variables.css` | 主题 CSS 变量（--background / --foreground / --primary / --ring / --radius / 密度变量 --density-mult 等；亮暗两套） |
| `shadcn.css` | shadcn 组件样式层（ui 组件依赖的变量/动效） |
| `transitions.css` | 过渡动画（fade/slide 等，tw-animate-css 补充） |

- 主题切换：`themeStore` 切换 `document.documentElement.classList`（dark）；外观偏好（强调色/字号/密度）由 `appearanceStore` 以 inline style 覆盖 CSS 变量（优先级高于样式文件）。

## 4. 已知陷阱

- **标签规范值变更 = 历史数据失配**：`ANCHOR_TAG_VALUES` 是落盘值，改名会与新主播/筛选/持久化不匹配（除非做迁移）。新增标签需同步：TAG_PAIRS、设置页预选、后端校验（如有）、i18n 文案。
- `virtualList.ts` 依赖固定行高：视觉改版（行高/间距）必须同步计算。
- i18n 键缺失时 vue-i18n 回退 zh-CN：新增文案**必须**双语同步（zh-CN + en），CI 无 i18n 校验，靠人工。
- 样式变量是全局契约：`variables.css` 的变量被 shadcn.css / 组件 / appearanceStore 三方消费，改名需全局搜索（styles + components + stores）。
