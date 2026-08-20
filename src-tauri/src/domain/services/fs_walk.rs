//! 安全目录遍历工具（S1 修复）：递归收集目录树中的**普通文件**路径，
//! 全程**不跟随**符号链接 / Windows junction / 目录符号链接。
//!
//! 背景（docs/audit/01-静态暴力审查-功能漏洞.md S1）：清理服务
//! （cleanup.rs `scan_recording_files`）与文件缓存（file_cache.rs
//! `scan_output_dir`）原本各自实现一份「递归遍历输出目录」的逻辑。若遍历把
//! 目录类链接当作普通目录跟随，输出目录内的链接（用户误建 / 第三方软件生成）
//! 会指向**目录外**位置——自动清理可能据此删除目录外的音频文件（数据丢失），
//! 文件页会越权列出目录外文件。两处遍历统一收敛到本模块。
//!
//! 判定方式：条目类型一律以 `std::fs::symlink_metadata` 为准（**不跟随**
//! 链接）。Windows 上 junction（`mklink /J`）与目录符号链接（`mklink /D`）
//! 是 reparse point，`symlink_metadata` 得到的 `file_type().is_symlink()`
//! 为 `true`、`is_dir()` 为 `false`；Linux 上符号链接同理（`lstat` 语义）。
//! 因此链接项既不会作为目录进入，也不会作为文件产出。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// 深度优先安全遍历 `root`，返回其下所有**普通文件**的路径（顺序不定）。
///
/// 安全保证（S1）：
/// - 每条目经 `symlink_metadata`（不跟随链接）判定；符号链接 / Windows
///   junction / 目录符号链接一律跳过——既不进入其目录，也不产出其文件；
/// - `root` 本身**不**做链接校验：输出目录由用户配置，可能故意是链接 /
///   映射盘，风险点在**目录内**的链接项；
/// - `root` 不存在 → 空列表；`root` 无法读取（如指向普通文件）→ `Err`，
///   由调用方决定处理（cleanup 透传为 AppError，file_cache 退回空列表）；
/// - 子树读取失败 / 单条目元数据失败 → 跳过该子树/条目，不中断整次遍历。
pub(crate) fn safe_walk_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    let mut stack: Vec<PathBuf> = Vec::new();
    // 根目录读取失败向上传播（`?`）；子树读取失败仅跳过该子树
    for entry in fs::read_dir(root)?.flatten() {
        classify_entry(&entry.path(), &mut stack, &mut out);
    }
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            classify_entry(&entry.path(), &mut stack, &mut out);
        }
    }
    Ok(out)
}

/// 分类单个目录项：链接一律跳过；真实目录入栈待遍历；普通文件产出；
/// 元数据读取失败（条目被并发删除等）同样跳过。
fn classify_entry(path: &Path, stack: &mut Vec<PathBuf>, out: &mut Vec<PathBuf>) {
    // S1 核心：symlink_metadata 不跟随链接——junction/目录符号链接在
    // Windows 上同时带 DIRECTORY 与 REPARSE_POINT 属性，此处按
    // is_symlink() == true 处理，不再当普通目录
    let Ok(meta) = fs::symlink_metadata(path) else {
        return;
    };
    let ft = meta.file_type();
    if ft.is_symlink() {
        // 链接项（文件链接 / 目录链接 / junction）：不进入、不产出
        return;
    }
    if ft.is_dir() {
        stack.push(path.to_path_buf());
    } else if ft.is_file() {
        out.push(path.to_path_buf());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn test_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "missevan-test-fswalk-{}-{}-{}",
            tag,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn collects_files_recursively_but_not_dirs() {
        let root = test_root("plain");
        std::fs::create_dir_all(root.join("sub/deep")).unwrap();
        std::fs::write(root.join("a.m4a"), b"x").unwrap();
        std::fs::write(root.join("sub/b.m4a"), b"x").unwrap();
        std::fs::write(root.join("sub/deep/c.txt"), b"x").unwrap();

        let found = safe_walk_files(&root).unwrap();
        let mut names: Vec<String> = found
            .iter()
            .map(|p| {
                p.strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.m4a", "sub/b.m4a", "sub/deep/c.txt"]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_root_returns_empty() {
        let root = test_root("missing");
        assert!(safe_walk_files(&root).unwrap().is_empty());
    }

    #[test]
    fn root_that_is_a_file_returns_err() {
        let root = test_root("file-root");
        let f = root.join("not-a-dir");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&f, b"x").unwrap();
        assert!(safe_walk_files(&f).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn skips_symlinks_both_file_and_dir() {
        let root = test_root("link");
        let outside = test_root("link-outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("real.m4a"), b"x").unwrap();
        std::fs::write(outside.join("secret.m4a"), b"x").unwrap();

        // 目录链接指向 root 外
        #[cfg(unix)]
        let dir_link_ok = std::os::unix::fs::symlink(&outside, root.join("linked-dir")).is_ok();
        #[cfg(windows)]
        let dir_link_ok =
            std::os::windows::fs::symlink_dir(&outside, root.join("linked-dir")).is_ok();
        // 文件链接指向 root 外的文件
        #[cfg(unix)]
        let file_link_ok = std::os::unix::fs::symlink(
            &outside.join("secret.m4a"),
            root.join("linked-file.m4a"),
        )
        .is_ok();
        #[cfg(windows)]
        let file_link_ok = std::os::windows::fs::symlink_file(
            &outside.join("secret.m4a"),
            root.join("linked-file.m4a"),
        )
        .is_ok();

        let found = safe_walk_files(&root).unwrap();
        let names: Vec<String> = found
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec![root.join("real.m4a").to_string_lossy().into_owned()],
            "文件链接与目录链接指向的内容都不得进入遍历结果"
        );

        // 无权限（Windows 非开发者模式 / 非管理员）时链接创建失败，测试仅
        // 验证「无链接时的正常遍历」（该场景已由其他用例覆盖），不视为失败
        let _ = (dir_link_ok, file_link_ok);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// Windows 专用：junction（mklink /J，无需管理员/开发者模式）指向目录外，
    /// 遍历不得进入——S1 的 P0 场景（自动清理可能据此删除目录外音频文件）
    #[cfg(windows)]
    #[test]
    fn skips_windows_junction_created_via_mklink() {
        let root = test_root("junction");
        let outside = test_root("junction-outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("real.m4a"), b"x").unwrap();
        std::fs::write(outside.join("secret.m4a"), b"x").unwrap();
        let status = std::process::Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                root.join("linked").to_str().unwrap(),
                outside.to_str().unwrap(),
            ])
            .status();
        match status {
            Ok(s) if s.success() => {}
            _ => {
                // 极罕见：junction 创建失败（如目标已存在/权限），跳过本测试
                let _ = std::fs::remove_dir_all(&root);
                let _ = std::fs::remove_dir_all(&outside);
                return;
            }
        }
        // junction 应按符号链接处理：is_symlink() == true（实测 Rust 1.96 /
        // Windows：junction 的 symlink_metadata file_type 为 is_symlink=true）
        let meta = std::fs::symlink_metadata(root.join("linked")).unwrap();
        assert!(
            meta.file_type().is_symlink(),
            "junction 应被 symlink_metadata 识别为链接"
        );

        let found = safe_walk_files(&root).unwrap();
        let names: Vec<String> = found
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec![root.join("real.m4a").to_string_lossy().into_owned()],
            "junction 指向的目录外文件不得进入遍历结果"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }
}
