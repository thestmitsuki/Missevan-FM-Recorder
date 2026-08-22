//! 关于窗口后端命令（规格 §2.1 关于窗口：检查更新 / 应用信息 / 打开浏览器，Task 20）
//!
//! - [`check_update`]：GET GitHub Releases API 最新版本，解析 `tag_name` 与下载链接，
//!   返回 `{ latest, current, download_url }`；仓库未发布（404）或网络失败时
//!   返回 `Err`——前端显示「检查更新失败」，应用不崩溃。
//! - [`get_app_info`]：关于对话框静态信息（应用名 / 版本 / 构建日期 / OS / Rust / Tauri）。
//!   构建日期取可执行文件修改时间（Windows 下即编译产物时间，近似构建时间）。
//! - [`open_browser`]：默认浏览器打开 URL（Windows `cmd /c start`；macOS `open`；
//!   Linux `xdg-open`；仅放行 http/https，防命令注入）。

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local};
use serde::Serialize;
use tauri::State;

use crate::domain::config::manager::ConfigManager;
use crate::domain::spider::MissevanClient;
use crate::infrastructure::error::types::AppError;
use crate::tr;

/// GitHub 发布仓库（Missevan-FM-Recorder）：发布版 tag 命名 `v{version}`。
/// 仓库尚未发布（404）或网络失败时 GitHub API 返回错误 → 前端「检查更新失败」，应用不崩溃。
const UPDATE_REPO_OWNER: &str = "thestmitsuki";
const UPDATE_REPO_NAME: &str = "Missevan-FM-Recorder";
/// GitHub API 请求超时（秒）
const UPDATE_TIMEOUT_SECS: u64 = 10;

/// 更新检查结果（规格 §2.1：最新版本 + 下载链接）
#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    /// 最新版本（tag_name 剥离前导 v，如 "1.2.3"）
    pub latest: String,
    /// 当前版本（CARGO_PKG_VERSION）
    pub current: String,
    /// 下载链接：按平台关键词匹配资产（win / appimage / arch）；无匹配资产时
    /// 回退发布页 html_url（用户自行选择，绝不下载错误平台的包）
    pub download_url: Option<String>,
}

/// 下载资产平台匹配（P1-9：按平台关键词匹配发布资产名，大小写不敏感）。
///
/// 发布资产命名约定：Windows 包名含 `win`（如 `missevan-recorder_1.2.0_win_x64.exe`）、
/// AppImage 包名含 `appimage`（如 `..._linux.AppImage`）、Arch 包名含 `arch`
/// （如 `..._arch_x86_64.pkg.tar.zst`）。按关键词而非后缀匹配，避免资产后缀
/// 变化（如 `.exe` 改 `.msi`）导致匹配失败后错误回退到首个资产。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetPlatform {
    /// Windows：包名含 `win`
    Windows,
    /// Linux（AppImage）：包名含 `appimage`
    AppImage,
    /// Linux（Arch 系发行版）：包名含 `arch`
    Arch,
}

impl AssetPlatform {
    /// 当前运行平台：Windows → `Windows`；Linux 按发行版区分 Arch（读
    /// `/etc/os-release`，覆盖 Arch / Manjaro / EndeavourOS 等 Arch 系）与
    /// AppImage（其余发行版）；macOS 暂按 Windows 规则兜底（资产通常同时发布）。
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            AssetPlatform::Windows
        } else if cfg!(target_os = "linux") && is_arch_linux() {
            AssetPlatform::Arch
        } else if cfg!(target_os = "linux") {
            AssetPlatform::AppImage
        } else {
            AssetPlatform::Windows
        }
    }

    /// 资产 URL（已小写）是否含本平台关键词
    fn matches(self, lower_url: &str) -> bool {
        match self {
            AssetPlatform::Windows => lower_url.contains("win"),
            AssetPlatform::AppImage => lower_url.contains("appimage"),
            AssetPlatform::Arch => lower_url.contains("arch"),
        }
    }
}

/// Linux 发行版检测：`/etc/os-release` 的 `ID=arch` 或 `ID_LIKE` 含 `arch`。
/// 仅 Linux 上调用（Windows/macOS 分支不读取该文件）。
fn is_arch_linux() -> bool {
    std::fs::read_to_string("/etc/os-release")
        .map(|content| {
            content.lines().any(|line| {
                line.starts_with("ID=arch")
                    || (line.starts_with("ID_LIKE=") && line.contains("arch"))
            })
        })
        .unwrap_or(false)
}

/// 从 GitHub `releases/latest` 响应体解析更新信息（纯函数，单测覆盖）。
///
/// 提取规则：
/// - `tag_name`："v1.2.3" → "1.2.3"（无前导 v 原样保留）；
/// - **tag 必须是语义化版本**（至少 `数字.数字.数字`，可带 `-预发布` 后缀）——
///   误把分支名（如 "main"）当作 tag 发布时，非版本 tag 返回 `None`
///   （check_update 报「检查更新失败」而非把 "main" 当版本展示）；
/// - 下载链接优先级：`assets[]` 中 `browser_download_url` 含 `platform`
///   关键词（win / appimage / arch）的首个资产 → `html_url`（发布页兜底）。
///   无匹配资产时**不**回退首个资产——避免下载到错误平台的包。
pub fn parse_release(
    json: &serde_json::Value,
    current: &str,
    platform: AssetPlatform,
) -> Option<UpdateInfo> {
    let tag = json.get("tag_name")?.as_str()?;
    let latest = tag.strip_prefix('v').unwrap_or(tag).to_string();
    // 语义化版本校验：X.Y.Z（X/Y/Z 均为数字；预发布后缀 "-beta.1" 等允许）
    let base = latest.split('-').next().unwrap_or(&latest);
    let parts: Vec<&str> = base.split('.').collect();
    let valid = parts.len() >= 3 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    if !valid {
        return None;
    }

    let download_url = json
        .get("assets")
        .and_then(|a| a.as_array())
        .and_then(|assets| {
            let urls: Vec<&str> = assets
                .iter()
                .filter_map(|a| a.get("browser_download_url")?.as_str())
                .collect();
            urls.iter()
                .copied()
                .find(|u| platform.matches(&u.to_lowercase()))
        })
        .map(|s| s.to_string())
        .or_else(|| json.get("html_url").and_then(|h| h.as_str()).map(|s| s.to_string()));

    Some(UpdateInfo {
        latest,
        current: current.to_string(),
        download_url,
    })
}

/// 检查更新（规格 §2.1）：GitHub Releases API 最新版本。
///
/// 网络失败 / HTTP 非 2xx（含仓库未发布 404）/ 响应缺版本字段 → `Err`，
/// 前端展示「检查更新失败」；不会 panic 或崩溃。当前版本取自 `CARGO_PKG_VERSION`。
#[tauri::command]
pub async fn check_update(
    config_manager: State<'_, Arc<ConfigManager>>,
) -> Result<UpdateInfo, AppError> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    // 「检查更新」开关（设置 > 通用）：关闭时手动检查也拒绝（与规格「检查更新」设置一致）；
    // 配置加载失败按默认（开启）处理，与旧逻辑一致
    let config = config_manager.load().unwrap_or_default();
    if !config.global.check_updates {
        return Err(AppError::config(tr!("update.check_disabled")));
    }

    let url = format!(
        "https://api.github.com/repos/{}/{}/releases/latest",
        UPDATE_REPO_OWNER, UPDATE_REPO_NAME
    );
    // G9 审查跟进：复用共享 HTTP client（`from_config` 配置指纹缓存，连接池/
    // TLS 会话跨调用复用，不再每次新建 reqwest Client）；GitHub 专属 UA 与
    // 10s 超时用「每请求 header/timeout 覆盖」实现，不改共享实例配置
    let client = MissevanClient::from_config(&config.global)?;
    let resp = client
        .reqwest_client()
        .get(&url)
        // GitHub API 要求 User-Agent（无 UA 返回 403）
        .header(
            "User-Agent",
            concat!("missevan-recorder/", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(UPDATE_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| AppError::internal(tr!("update.check_failed_network", err = e)))?;

    if !resp.status().is_success() {
        return Err(AppError::internal(tr!(
            "update.check_failed_http",
            status = resp.status()
        )));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::internal(tr!("update.check_failed_parse", err = e)))?;

    parse_release(&json, &current, AssetPlatform::current()).ok_or_else(|| {
        AppError::internal(tr!("update.check_failed_no_version"))
    })
}

/// 关于对话框静态信息（规格 §2.1：应用名称、版本号、构建日期）
#[derive(Debug, Clone, Serialize)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    /// 构建日期（可执行文件修改时间近似；`YYYY-MM-DD HH:MM:SS`）
    pub build_date: String,
    pub os: String,
    pub rust_version: String,
    pub tauri_version: String,
}

/// 获取关于窗口信息：应用名 / 版本 / 构建日期 / OS / Rust / Tauri 版本。
/// 构建日期无可执行文件元数据时退回 "unknown"（不报错）。
#[tauri::command]
pub fn get_app_info() -> AppInfo {
    let build_date = std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok())
        .map(|t| DateTime::<Local>::from(t).format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "unknown".to_string());

    AppInfo {
        name: tr!("update.app_name").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        build_date,
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        rust_version: env!("CARGO_PKG_RUST_VERSION").to_string(),
        tauri_version: tauri::VERSION.to_string(),
    }
}

/// URL 是否可安全交给系统浏览器打开（防命令注入）。
///
/// 仅放行 `http://` / `https://`，且不允许控制字符、空格、引号与 shell 元字符
/// `|`/`^`/`<>`（合法 URL 中这些字符均应百分号编码）。
///
/// 历史（Task 20）：曾因 `cmd /c start` 的命令行解释而把 `%` 与 `&` 一并拒绝
/// （cmd 中 `%` 是变量展开符、`&` 是命令分隔符）。Windows 侧已改经
/// `rundll32 url.dll,FileProtocolHandler` 直接传参（不经任何 shell，无 `%VAR%`
/// 展开 / `&` 分隔符解释），而「报告问题」URL 由 `encodeURIComponent` 编码后
/// 必然含 `%`（如 `%E9%97%AE`）与 `&`（title/body 查询参数分隔符）——守卫放行
/// 这两种字符；控制字符（`\r` `\n` `\t` 等）仍一律拒绝（trim 只去首尾空白）。
fn is_browsable_url(url: &str) -> bool {
    let u = url.trim();
    if !(u.starts_with("https://") || u.starts_with("http://")) {
        return false;
    }
    !u.chars().any(|c| {
        c.is_control() || matches!(c, ' ' | '"' | '|' | '^' | '<' | '>')
    })
}

/// 用默认浏览器打开 URL（规格 §2.1 下载链接 / §2.2 报告问题）。
///
/// Windows：`rundll32 url.dll,FileProtocolHandler "<url>"`——Windows 官方推荐的
/// 协议处理器打开方式（Tauri 无 opener 插件，std 命令最简，无新增依赖）。
/// URL 作为命令行参数直接交给 FileProtocolHandler，**不经 cmd**：
/// 1. 无 `%VAR%` 环境变量展开——`cmd /c start` 下 URL 编码序列（`%E9%97%AE`、
///    `%0A`）会被 cmd 破坏（实测 `%25OS%` → `5OS`、`%0` 被当作脚本名展开）；
/// 2. 无 8191 字符的 cmd 命令行上限（CreateProcess 上限 32767）——报告问题
///    URL（实测 ~1KB，含多行系统信息）与未来更长的正文均不受限；
/// 3. `&` 无需引号保护（无命令分隔符语义）。
/// URL 自加引号防空格分词（守卫已拒绝 URL 内的引号字符，无转义风险）。
/// macOS：`open`；Linux：`xdg-open`（均 argv 直传，无 shell）。spawn 后由后台
/// 线程 wait 回收（M4：xdg-open/open 是短命启动器，Linux 上不 wait 会留僵尸）。
#[tauri::command]
pub fn open_browser(url: String) -> Result<(), AppError> {
    let url = url.trim().to_string();
    if !is_browsable_url(&url) {
        return Err(AppError::config(tr!("update.url_not_allowed")));
    }
    #[cfg(target_os = "windows")]
    {
        // ⚠️ rundll32 FileProtocolHandler 会把引号字符原样并入 URL 传给
        // ShellExecute（实测带引号打开失败）——而 is_browsable_url 已拒绝
        // 空格（URL 编码后无字面空格），故直接裸传 URL，不加引号。
        let child = std::process::Command::new("rundll32")
            .arg("url.dll,FileProtocolHandler")
            .arg(&url)
            .spawn()
            .map_err(|e| AppError::internal(tr!("update.open_browser_failed", err = e)))?;
        crate::domain::tools::reap_in_background(child);
    }
    #[cfg(target_os = "macos")]
    {
        let child = std::process::Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|e| AppError::internal(tr!("update.open_browser_failed", err = e)))?;
        crate::domain::tools::reap_in_background(child);
    }
    #[cfg(target_os = "linux")]
    {
        let child = std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| AppError::internal(tr!("update.open_browser_failed", err = e)))?;
        crate::domain::tools::reap_in_background(child);
    }
    tracing::info!("{}", tr!("update.open_browser_ok", url = url));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn release_json(tag: &str) -> serde_json::Value {
        json!({
            "tag_name": tag,
            "html_url": "https://github.com/thestmitsuki/Missevan-FM-Recorder/releases/tag/v1.2.0",
            "assets": [
                { "name": "missevan-recorder_1.2.0_win_x64.exe", "browser_download_url": "https://github.com/.../missevan-recorder_1.2.0_win_x64.exe" },
                { "name": "missevan-recorder_1.2.0_win_x64_portable.exe", "browser_download_url": "https://github.com/.../missevan-recorder_1.2.0_win_x64_portable.exe" },
                { "name": "notes.md", "browser_download_url": "https://github.com/.../notes.md" }
            ]
        })
    }

    #[test]
    fn parse_release_strips_v_prefix_and_prefers_win_asset() {
        let info = parse_release(&release_json("v1.2.0"), "0.1.0", AssetPlatform::Windows).unwrap();
        assert_eq!(info.latest, "1.2.0");
        assert_eq!(info.current, "0.1.0");
        // Windows 关键词 `win`：两个 win 资产中取首个
        assert_eq!(
            info.download_url.as_deref(),
            Some("https://github.com/.../missevan-recorder_1.2.0_win_x64.exe")
        );
    }

    #[test]
    fn parse_release_tag_without_v_kept_as_is() {
        let info = parse_release(&release_json("1.2.0"), "0.1.0", AssetPlatform::Windows).unwrap();
        assert_eq!(info.latest, "1.2.0");
    }

    #[test]
    fn parse_release_no_assets_falls_back_to_html_url() {
        let json = json!({
            "tag_name": "v1.2.0",
            "html_url": "https://github.com/owner/repo/releases/tag/v1.2.0",
            "assets": []
        });
        let info = parse_release(&json, "0.1.0", AssetPlatform::Windows).unwrap();
        assert_eq!(info.latest, "1.2.0");
        assert_eq!(
            info.download_url.as_deref(),
            Some("https://github.com/owner/repo/releases/tag/v1.2.0")
        );
    }

    #[test]
    fn parse_release_missing_tag_yields_none() {
        assert!(parse_release(&json!({ "foo": 1 }), "0.1.0", AssetPlatform::Windows).is_none());
        assert!(parse_release(&json!({ "tag_name": null }), "0.1.0", AssetPlatform::Windows).is_none());
    }

    #[test]
    fn parse_release_rejects_non_semver_tag() {
        // 误把分支名当 tag 发布（如 "main"）——不得当作版本展示
        assert!(parse_release(&release_json("main"), "0.1.0", AssetPlatform::Windows).is_none());
        assert!(parse_release(&release_json("vmain"), "0.1.0", AssetPlatform::Windows).is_none());
        assert!(parse_release(&release_json("1.2"), "0.1.0", AssetPlatform::Windows).is_none());
        assert!(parse_release(&release_json("release-1"), "0.1.0", AssetPlatform::Windows).is_none());
        // 预发布后缀仍视为合法版本
        let info = parse_release(&release_json("v1.2.0-beta.1"), "0.1.0", AssetPlatform::Windows).unwrap();
        assert_eq!(info.latest, "1.2.0-beta.1");
    }

    /// AppImage 平台：跳过 win / arch 资产，按关键词 `appimage` 匹配
    #[test]
    fn parse_release_appimage_matches_by_keyword() {
        let json = json!({
            "tag_name": "v1.2.0",
            "html_url": "https://github.com/owner/repo/releases/tag/v1.2.0",
            "assets": [
                { "name": "missevan-recorder_1.2.0_win_x64.exe", "browser_download_url": "https://github.com/.../missevan-recorder_1.2.0_win_x64.exe" },
                { "name": "missevan-recorder_1.2.0_linux.AppImage", "browser_download_url": "https://github.com/.../missevan-recorder_1.2.0_linux.AppImage" },
                { "name": "missevan-recorder_1.2.0_arch_x86_64.pkg.tar.zst", "browser_download_url": "https://github.com/.../missevan-recorder_1.2.0_arch_x86_64.pkg.tar.zst" },
                { "name": "notes.md", "browser_download_url": "https://github.com/.../notes.md" }
            ]
        });
        let info = parse_release(&json, "0.1.0", AssetPlatform::AppImage).unwrap();
        assert_eq!(
            info.download_url.as_deref(),
            Some("https://github.com/.../missevan-recorder_1.2.0_linux.AppImage")
        );
    }

    /// Arch 平台：按关键词 `arch` 匹配 pkg.tar.zst 资产
    #[test]
    fn parse_release_arch_matches_by_keyword() {
        let json = json!({
            "tag_name": "v1.2.0",
            "html_url": "https://github.com/owner/repo/releases/tag/v1.2.0",
            "assets": [
                { "name": "missevan-recorder_1.2.0_win_x64.exe", "browser_download_url": "https://github.com/.../missevan-recorder_1.2.0_win_x64.exe" },
                { "name": "missevan-recorder_1.2.0_linux.AppImage", "browser_download_url": "https://github.com/.../missevan-recorder_1.2.0_linux.AppImage" },
                { "name": "missevan-recorder_1.2.0_arch_x86_64.pkg.tar.zst", "browser_download_url": "https://github.com/.../missevan-recorder_1.2.0_arch_x86_64.pkg.tar.zst" }
            ]
        });
        let info = parse_release(&json, "0.1.0", AssetPlatform::Arch).unwrap();
        assert_eq!(
            info.download_url.as_deref(),
            Some("https://github.com/.../missevan-recorder_1.2.0_arch_x86_64.pkg.tar.zst")
        );
    }

    /// 关键词匹配大小写不敏感（`LINUX.APPIMAGE` 大写也命中）
    #[test]
    fn parse_release_keyword_match_is_case_insensitive() {
        let json = json!({
            "tag_name": "v1.2.0",
            "html_url": "https://github.com/owner/repo/releases/tag/v1.2.0",
            "assets": [
                { "name": "x", "browser_download_url": "https://github.com/.../missevan-recorder_1.2.0_LINUX.APPIMAGE" }
            ]
        });
        let info = parse_release(&json, "0.1.0", AssetPlatform::AppImage).unwrap();
        assert_eq!(
            info.download_url.as_deref(),
            Some("https://github.com/.../missevan-recorder_1.2.0_LINUX.APPIMAGE")
        );
    }

    /// 无本平台关键词资产 → 回退发布页 html_url（**不**取首个资产）
    #[test]
    fn parse_release_no_platform_match_falls_back_to_html_url() {
        let json = json!({
            "tag_name": "v1.2.0",
            "html_url": "https://github.com/owner/repo/releases/tag/v1.2.0",
            "assets": [
                { "name": "a", "browser_download_url": "https://github.com/.../app.deb" },
                { "name": "b", "browser_download_url": "https://github.com/.../notes.md" }
            ]
        });
        // Windows 平台无 `win` 关键词资产 → 不下载错误平台包，回退发布页（用户自行选择）
        let info = parse_release(&json, "0.1.0", AssetPlatform::Windows).unwrap();
        assert_eq!(
            info.download_url.as_deref(),
            Some("https://github.com/owner/repo/releases/tag/v1.2.0")
        );
    }

    #[test]
    fn browsable_url_only_allows_http_https() {
        assert!(is_browsable_url("https://github.com/owner/repo/releases"));
        assert!(is_browsable_url("http://example.com"));
        assert!(!is_browsable_url("file:///C:/Windows/System32"));
        assert!(!is_browsable_url("javascript:alert(1)"));
        assert!(!is_browsable_url(""));
        // 命令注入尝试：空格 / 元字符即使夹在 https 前缀后也一律拒绝
        assert!(!is_browsable_url("https://a.com & calc.exe"));
        assert!(!is_browsable_url("https://a.com|calc.exe"));
        assert!(!is_browsable_url("https://a.com^echo"));
        assert!(!is_browsable_url("https://a.com<cmd"));
        assert!(!is_browsable_url("https://a.com>cmd"));
        assert!(!is_browsable_url("https://a.com/\"b\""));
        // 控制字符：trim 只去首尾空白，URL 内部 \r\n\t 一律拒绝
        assert!(!is_browsable_url("https://a.com/\r\ncalc.exe"));
        assert!(!is_browsable_url("https://a.com\tb"));
        assert!(!is_browsable_url("https://a.com\x07b"));
        // 合法发布页链接放行
        assert!(is_browsable_url("https://github.com/owner/repo/releases/download/v1.0.0/app-setup.exe"));
    }

    #[test]
    fn percent_encoded_and_query_urls_are_allowed() {
        // 「报告问题」URL：encodeURIComponent 编码（含 %XX 序列）+ title/body
        // 查询参数（含 & 分隔符）——Windows 经 rundll32 直传不经 cmd，无
        // %VAR% 展开风险，必须放行（此前被 `%`/`&` 守卫拦截 → 「无法打开浏览器」）
        assert!(is_browsable_url(
            "https://github.com/thestmitsuki/Missevan-FM-Recorder/issues/new?title=%5BBug%5D%200.1.0%20%E9%97%AE%E9%A2%98%E5%8F%8D%E9%A6%88&body=%0A**%E5%BA%94%E7%94%A8%E7%89%88%E6%9C%AC**%3A%200.1.0"
        ));
        // 百分号编码空格 / 编码后的中文 / 换行（%0A）均为合法 URL 字符
        assert!(is_browsable_url("https://a.com/x%20y"));
        assert!(is_browsable_url("https://a.com/?q=%E4%BD%A0%E5%A5%BD&t=a%0Ab"));
        // 查询参数中的 & 与空 title/body 也放行
        assert!(is_browsable_url("https://a.com/path?a=1&b=2"));
    }
}
