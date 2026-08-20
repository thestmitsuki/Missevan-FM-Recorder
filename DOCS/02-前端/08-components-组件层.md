# 08 · components —— 组件层

> 文件：`src/components/{common,layout,ui}/**`、`src/layouts/AppLayout.vue`

## 1. 分层

| 目录 | 角色 | 内容 |
| --- | --- | --- |
| `ui/` | shadcn-vue 风格基础组件库 | 由 reka-ui（Radix Vue）原语 + `class-variance-authority` 变体 + Tailwind 构成；**无业务逻辑**，可跨项目复用 |
| `common/` | 业务通用组件 | 依赖 store / i18n / 业务类型的通用 UI |
| `layout/` | 布局组件 | PageContainer / TopBar |
| `layouts/AppLayout.vue` | 主窗整体布局 | NavRail + TopBar + 内容区（router-view） |

## 2. ui/ 组件清单（shadcn 风格）

button / input / label / textarea / select / switch / checkbox / badge / card / dialog / alert-dialog / sheet / dropdown-menu / tabs / tooltip / table / skeleton / separator / scroll-area / slider / radio-group / popover / progress / avatar / command 等（`components/ui/<name>/` 目录，每组件一个文件，含 cva 变体）。

- 风格约定：`cn()`（`lib/utils.ts`）合并类名；主题由 CSS 变量驱动（`styles/variables.css` + `shadcn.css`），暗色通过 `.dark` 类切换。
- 与 reka-ui 的关系：ui 组件封装 reka-ui 原语（如 AlertDialogContent 包 `reka-ui` 的 `AlertDialogContent`），对外暴露 Vue 组件 + 插槽。

## 3. common/ 业务组件

| 组件 | 职责 |
| --- | --- |
| `ConfirmDialog.vue` | 确认对话框（危险操作统一入口：删除主播/文件/清空等） |
| `EmptyState.vue` | 空状态占位（图标 + 文案 + 可选动作） |
| `ErrorBoundary.vue` | 渲染错误捕获（子组件异常 → 降级提示，不白屏） |
| `NavRail.vue` | 竖排操作栏（直播页左侧；导出 NavRailItem 类型） |
| `SideNav.vue` | 侧边导航（AppLayout 左侧主导航） |
| `StatusBadge.vue` | 状态徽标（直播中/录制中/离线等） |
| `NotEffectiveBadge.vue` | 「未启用检测」提示徽标（enable_check=false） |
| `Toast.vue` | Toast 容器（vue-sonner 封装，顶部/底部，自动消失 + 手动关闭） |

## 4. layout/ 与 AppLayout

- `PageContainer.vue`：页面内容容器（内边距 / 最大宽度 / 滚动区）。
- `TopBar.vue`：顶栏（标题 / 全局操作：打开输出目录、通知铃铛、主题切换、设置入口）。
- `AppLayout.vue`：SideNav（主导航：直播/文件/设置 + 更多→调试）+ TopBar + `<router-view>`；错误边界包裹；全局监听（onTrayOpenLivePage 跳转直播页、通知 store 初始化）。

## 5. 使用约定

- 页面优先组合 `ui/` 与 `common/` 组件，**禁止**在页面内复制 shadcn 样式块；
- 新增可复用基础组件放 `ui/`（带 cva 变体与 Tailwind），业务组件放 `common/`；
- 图标统一 `@lucide/vue`（按需导入），文案一律 i18n。

## 6. 已知陷阱

- `ui/` 组件基于 reka-ui：升级 reka-ui 主版本可能破坏原语 API（组件内 import 来自 `reka-ui`），升级后需全量回归弹层/焦点/键盘交互。
- `ErrorBoundary` 只捕获**渲染期**错误（onErrorCaptured），事件回调/异步错误不经过它——异步错误走 notificationStore / Toast。
- Toast 由 vue-sonner 驱动：**后端通知（app:notification）与前端 Toast 是两条链**（前者进通知中心，后者本地触发），不要混用。
- shadcn.css 的 CSS 变量被 appearanceStore 的 inline style 覆盖：改主题变量时注意「自定义强调色」与默认色板的优先级（自定义色优先）。
