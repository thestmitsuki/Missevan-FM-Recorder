/**
 * 设置页分类 ID（独立类型模块）。
 *
 * 原定义位于 sections/index.ts，validation.ts 以 `import type` 反向引用
 * sections/index 形成类型级环（sections/* → validation → sections/index）。
 * 类型导入虽在编译期被擦除、无运行时风险，但为让模块图完全无环
 * （madge --circular 全绿 + 杜绝后续误改运行时导入的隐患），
 * 将 CategoryId 下沉到独立模块，双方均只依赖本模块。
 */
export type CategoryId =
  | "general"
  | "recording"
  | "files"
  | "network"
  | "notification"
  | "appearance"
  | "shortcuts"
  | "advanced";
