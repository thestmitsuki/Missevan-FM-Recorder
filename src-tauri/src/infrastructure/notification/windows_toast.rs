//! Windows 原生 toast 通知（组 C/3「通知不要使用 POWERSHELL 而是应用注册通知」）。
//!
//! ## 背景（源码级证据）
//!
//! tauri-plugin-notification 2.3.3 的 Windows 实现（`desktop.rs::show`）在
//! 开发模式（可执行文件位于 `target\debug` / `target\release`）下**不设置**
//! `System.AppUserModel.ID`，随后 notify-rust 4.18.0（`src/windows.rs:76-79`）
//! 回退到 `Toast::POWERSHELL_APP_ID`
//! （`{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe`，
//! 见 tauri-winrt-notification 0.7.3 `src/lib.rs:326-331` 的常量与注释
//! “the toast will erroneously report its origin as powershell”）——因此
//! 未安装运行时通知以 PowerShell 身份显示（图标/名称均为 PowerShell，
//! 点击通知甚至会拉起 powershell.exe 窗口）。
//!
//! ## 本模块方案
//!
//! 绕开插件/notify-rust，直接用 `tauri-winrt-notification`（已在依赖树中，
//! 版本 0.7.3）调用 WinRT `ToastNotificationManager::CreateToastNotifierWithId`，
//! AUMID 恒为本应用 `com.missevan-recorder.app`（与 tauri.conf.json
//! `identifier` 一致），并在启动时通过 [`ensure_aumid_registered`]
//! 注册“开始菜单”快捷方式（Windows 对非打包应用 toast 的 AUMID 注册要求）：
//!
//! - 已安装：安装器已生成同名快捷方式且带本 AUMID → 跳过；
//! - 开发/未安装：在用户“开始菜单”创建指向当前可执行文件的快捷方式并写入
//!   `System.AppUserModel.ID` 属性。
//!
//! 提示音：`Sound::Default` 在 toast XML 中省略 `<audio>` 元素 → 播放系统
//! 默认提示音；`None` → `<audio silent="true"/>` → 静音（notify_sound 关闭）。

use std::path::Path;

use tauri_winrt_notification::{Sound, Toast};
use windows::core::{Interface, PWSTR};
use windows::Win32::{
    Storage::EnhancedStorage::PKEY_AppUserModel_ID,
    System::Com::StructuredStorage::{PropVariantClear, PROPVARIANT},
    System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemAlloc, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STGM_READ},
    System::Variant::VT_LPWSTR,
    UI::Shell::PropertiesSystem::IPropertyStore,
    UI::Shell::{IShellLinkW, ShellLink},
};

/// 本应用的 AppUserModelID（与 tauri.conf.json `identifier` 一致）。
pub const AUMID: &str = "com.missevan-recorder.app";

/// “开始菜单”快捷方式文件名（与安装器（NSIS/MSI）生成的快捷方式同名，
/// 便于识别“已由安装器注册”从而跳过）。
const SHORTCUT_NAME: &str = "missevan-recorder.lnk";

/// 发送 Windows toast 通知（本应用身份，绝不使用 PowerShell AUMID）。
///
/// - `sound = true` → `Sound::Default`：toast XML 省略 `<audio>` 元素，
///   播放系统默认提示音；
/// - `sound = false` → `None` → `<audio silent="true"/>`，静音。
///
/// AUMID 未注册时 `CreateToastNotifierWithId` 返回
/// `0x803E0105`（AppId not registered）——启动时已由
/// [`ensure_aumid_registered`] 处理注册，此失败仅属异常路径。
pub fn show_toast(title: &str, body: &str, sound: bool) -> Result<(), String> {
    let audio = if sound { Some(Sound::Default) } else { None };
    Toast::new(AUMID)
        .title(title)
        .text1(body)
        .sound(audio)
        .show()
        .map_err(|e| e.to_string())
}

/// 确保本应用 AUMID 已注册（toast 可显示）。
///
/// 返回 `Ok(true)` 表示本次新建了注册，`Ok(false)` 表示已存在。
pub fn ensure_aumid_registered() -> Result<bool, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("获取当前可执行文件路径失败: {e}"))?;
    let exe_dir = exe
        .parent()
        .ok_or_else(|| "无法获取可执行文件所在目录".to_string())?;

    // 1. 用户/机器“开始菜单”已存在带本 AUMID 的快捷方式（安装器注册）→ 跳过
    for dir in start_menu_programs_dirs() {
        let lnk = dir.join(SHORTCUT_NAME);
        if lnk.is_file() {
            match shortcut_aumid(&lnk) {
                Ok(Some(id)) if id == AUMID => return Ok(false),
                Ok(_) => tracing::debug!(
                    "开始菜单快捷方式 {} 的 AUMID 不匹配，将重新注册",
                    lnk.display()
                ),
                Err(e) => tracing::debug!(
                    "读取快捷方式 AUMID 失败（{}），将重新注册: {}",
                    lnk.display(),
                    e
                ),
            }
        }
    }

    // 2. 未注册 → 在用户“开始菜单”目录创建带 AUMID 的快捷方式
    let user_programs = user_start_menu_programs_dir()
        .ok_or_else(|| "无法定位用户“开始菜单”目录（%APPDATA% 缺失）".to_string())?;
    create_aumid_shortcut(&user_programs.join(SHORTCUT_NAME), &exe, exe_dir)
        .map_err(|e| format!("创建通知注册快捷方式失败: {e}"))?;
    tracing::info!("已注册通知 AUMID（{}）——toast 将以本应用身份显示", AUMID);
    Ok(true)
}

/// 候选“开始菜单”Programs 目录：用户目录（%APPDATA%）+ 机器目录（%PROGRAMDATA%）。
fn start_menu_programs_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(d) = user_start_menu_programs_dir() {
        dirs.push(d);
    }
    if let Some(d) = machine_start_menu_programs_dir() {
        dirs.push(d);
    }
    dirs
}

fn user_start_menu_programs_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("APPDATA").map(|a| {
        std::path::PathBuf::from(a).join("Microsoft\\Windows\\Start Menu\\Programs")
    })
}

fn machine_start_menu_programs_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("PROGRAMDATA").map(|p| {
        std::path::PathBuf::from(p).join("Microsoft\\Windows\\Start Menu\\Programs")
    })
}

/// 读取 .lnk 快捷方式的 `System.AppUserModel.ID` 属性（无属性返回 `None`）。
fn shortcut_aumid(lnk: &Path) -> windows::core::Result<Option<String>> {
    // 调用方线程可能未初始化 COM（如测试线程）——重复初始化返回 S_FALSE，无害
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let shell_link: IShellLinkW =
        unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)? };
    let lnk_wide = wide(&lnk.to_string_lossy());
    unsafe {
        shell_link
            .cast::<IPersistFile>()?
            .Load(PWSTR(lnk_wide.as_ptr() as *mut u16), STGM_READ)?;
    }
    let props: IPropertyStore = shell_link.cast()?;
    let mut pv: PROPVARIANT = unsafe { props.GetValue(&PKEY_AppUserModel_ID)? };
    let value = unsafe {
        if pv.Anonymous.Anonymous.vt.0 == VT_LPWSTR.0 {
            pv.Anonymous.Anonymous.Anonymous.pwszVal.to_string().ok()
        } else {
            None
        }
    };
    // GetValue 返回的字符串由 CoTaskMem 分配，必须 PropVariantClear 释放
    let _ = unsafe { PropVariantClear(&mut pv) };
    Ok(value)
}

/// 创建带 `System.AppUserModel.ID` 属性的“开始菜单”快捷方式
/// （IShellLinkW → IPropertyStore → IPersistFile，Windows 官方
/// “非打包应用 toast AUMID 注册”文档推荐的实现方式）。
fn create_aumid_shortcut(
    lnk_path: &Path,
    exe: &Path,
    exe_dir: &Path,
) -> windows::core::Result<()> {
    // 主线程可能已被 tao 以 STA 初始化 COM——重复初始化返回 S_FALSE，无害
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };

    let shell_link: IShellLinkW =
        unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)? };
    let exe_wide = wide(&exe.to_string_lossy());
    let dir_wide = wide(&exe_dir.to_string_lossy());
    unsafe {
        shell_link.SetPath(PWSTR(exe_wide.as_ptr() as *mut u16))?;
        shell_link.SetWorkingDirectory(PWSTR(dir_wide.as_ptr() as *mut u16))?;
        shell_link.SetIconLocation(PWSTR(exe_wide.as_ptr() as *mut u16), 0)?;
        let desc_wide = wide("missevan-recorder 通知注册快捷方式");
        shell_link.SetDescription(PWSTR(desc_wide.as_ptr() as *mut u16))?;
    }

    // 写入 System.AppUserModel.ID（toast 身份注册的关键属性）
    let props: IPropertyStore = shell_link.cast()?;
    // ⚠️ 实测（本机 shell32 ShellLink 属性存储）：IPropertyStore::SetValue 对
    // VT_LPWSTR 不复制字符串，而是保留指针并在属性替换/对象销毁时用
    // CoTaskMemFree 释放——若传入 Rust Vec 缓冲，CoTaskMemFree 会对 Rust 堆
    // 指针执行释放 → 堆损坏（0xC0000374，测试中进程退出时崩溃）。
    // 因此字符串必须由 CoTaskMemAlloc 分配，且所有权移交给属性存储
    //（与微软官方示例 InitPropVariantFromString 的分配方式一致；注册为
    // 一次性操作，即使属性存储是“复制”语义也仅产生一次性少量泄漏）。
    let aumid_utf16: Vec<u16> = AUMID.encode_utf16().collect();
    let aumid_buf = unsafe { CoTaskMemAlloc((aumid_utf16.len() + 1) * 2) };
    if aumid_buf.is_null() {
        return Err(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_OUTOFMEMORY,
        ));
    }
    let aumid_buf = aumid_buf as *mut u16;
    let mut pv = PROPVARIANT::default();
    unsafe {
        // ManuallyDrop 包装的 union 字段写入需显式解引用
        std::ptr::copy_nonoverlapping(aumid_utf16.as_ptr(), aumid_buf, aumid_utf16.len());
        aumid_buf.add(aumid_utf16.len()).write(0);
        (*pv.Anonymous.Anonymous).vt = VT_LPWSTR;
        (*pv.Anonymous.Anonymous).Anonymous.pwszVal = PWSTR(aumid_buf);
        props.SetValue(&PKEY_AppUserModel_ID, &pv)?;
        props.Commit()?;
    }

    // 保存 .lnk 文件
    let persist: IPersistFile = shell_link.cast()?;
    let lnk_wide = wide(&lnk_path.to_string_lossy());
    unsafe { persist.Save(PWSTR(lnk_wide.as_ptr() as *mut u16), true) }?;
    Ok(())
}

/// 编码为 UTF-16 并以 NUL 结尾（PCWSTR 输入用）。
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_encodes_nul_terminated_utf16() {
        let v = wide("com.missevan-recorder.app");
        assert_eq!(
            v,
            vec![
                0x0063, 0x006f, 0x006d, 0x002e, 0x006d, 0x0069, 0x0073, 0x0073, 0x0065, 0x0076,
                0x0061, 0x006e, 0x002d, 0x0072, 0x0065, 0x0063, 0x006f, 0x0072, 0x0064, 0x0065,
                0x0072, 0x002e, 0x0061, 0x0070, 0x0070, 0x0000
            ]
        );
        assert_eq!(*v.last().unwrap(), 0);
    }

    #[test]
    fn wide_handles_unicode() {
        let v = wide("猫耳");
        assert_eq!(v, vec![0x732b, 0x8033, 0x0000]);
    }

    /// 手动验证（默认忽略，不进入常规测试）：
    /// `cargo test --lib windows_toast -- --ignored --nocapture`
    /// 会注册 AUMID 并在用户桌面弹出本应用身份的 toast——用于实测
    /// 「通知不再以 PowerShell 身份出现」+ 系统默认提示音。
    #[test]
    #[ignore = "弹出真实 toast，需人工观察"]
    fn manual_aumid_registration_and_toast() {
        manual_registration_only();
        // 弹出真实 toast（带系统默认提示音）
        show_toast("Missevan 猫耳录制器", "通知测试：本应用身份 toast + 系统提示音", true)
            .expect("toast 发送失败");
        println!("toast 已发送（AUMID={AUMID}），请观察通知中心中的应用名称与图标");
    }

    /// 只做 AUMID 注册与校验（不弹 toast，用于定位问题）
    #[test]
    #[ignore = "手动排查用"]
    fn manual_registration_only() {
        match ensure_aumid_registered() {
            Ok(created) => println!("ensure_aumid_registered -> created={created}"),
            Err(e) => panic!("AUMID 注册失败: {e}"),
        }
        // 校验快捷方式确实带本应用 AUMID
        let user_programs = user_start_menu_programs_dir()
            .expect("%APPDATA% 缺失");
        let lnk = user_programs.join(SHORTCUT_NAME);
        assert!(lnk.is_file(), "快捷方式未创建: {}", lnk.display());
        let aumid = shortcut_aumid(&lnk)
            .unwrap_or_else(|e| panic!("读取快捷方式 AUMID 失败: {e}"));
        assert_eq!(aumid.as_deref(), Some(AUMID), "快捷方式 AUMID 应为 {AUMID}");
        println!("快捷方式 {} 已注册 AUMID={AUMID}", lnk.display());
    }
}
