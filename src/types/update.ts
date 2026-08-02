/**
 * 关于窗口 / 检查更新类型（与后端 `src-tauri/src/api/update_cmds.rs` 对齐，Task 20）
 */

/** 检查更新结果（规格 §2.1：最新版本 + 下载链接） */
export interface UpdateInfo {
  /** 最新版本（tag_name 剥离前导 v，如 "1.2.3"） */
  latest: string;
  /** 当前版本（CARGO_PKG_VERSION） */
  current: string;
  /** 下载链接：优先 .exe/.msi/.zip 资产 → 首个资产 → 发布页 html_url */
  download_url: string | null;
}

/** 关于窗口静态信息（应用名 / 版本 / 构建日期 / OS / Rust / Tauri） */
export interface AppInfo {
  name: string;
  version: string;
  /** 构建日期（可执行文件修改时间近似，YYYY-MM-DD HH:MM:SS） */
  build_date: string;
  os: string;
  rust_version: string;
  tauri_version: string;
}
