//! 后端 i18n：TOML 键值表驱动 + 语言状态 + `tr!` 宏。
//!
//! - 翻译表：`src-tauri/i18n.toml`（编译期 `include_str!` 嵌入，启动时解析一次）
//! - 语言来源：前端 i18n（`localStorage["locale"]`，D5 决策），经 `set_locale`
//!   命令（`config_cmds::set_locale`）同步到后端；默认中文。
//! - 查表：`lookup(key)` / `lookup_fmt(key, args)` / `lookup_plural(key, count, args)`
//! - 调用宏：`tr!("config.save_ok")` 或 `tr!("recorder.timeout", name = x)`
//!   （`#[macro_export]`，调用点 `use crate::tr;`）
//! - 回退：语言字段缺失 → zh；key 缺失 → 返回 key 本身 + warn

use std::collections::HashMap;
use std::sync::OnceLock;

use toml::Value;

/// 编译期嵌入翻译表（相对本文件：src/infrastructure/i18n.rs → ../../i18n.toml）
const I18N_TOML: &str = include_str!("../../i18n.toml");

/// 英文文本：普通字符串或复数（one/other）。
#[derive(Debug, Clone, PartialEq)]
enum EnText {
    Plain(String),
    Plural { one: String, other: String },
}

/// 单个翻译条目（zh 必填；en 缺省回退 zh）。
#[derive(Debug, Clone)]
struct Entry {
    zh: String,
    en: EnText,
}

/// 全表：`namespace.key` → 条目（进程内只解析一次）。
static TABLE: OnceLock<HashMap<String, Entry>> = OnceLock::new();

fn table() -> &'static HashMap<String, Entry> {
    TABLE.get_or_init(|| {
        // 解析失败不 panic：降级为空表（lookup 回退 key 本身 + warn），
        // 避免启动即崩（panic hook 自身也调 tr!，双重 panic 会直接 abort）。
        let value: Value = match toml::from_str(I18N_TOML) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(
                    "i18n.toml parse failed: {e}; translations fall back to key passthrough"
                );
                return HashMap::new();
            }
        };
        let Some(root) = value.as_table() else {
            tracing::error!(
                "i18n.toml root is not a table; translations fall back to key passthrough"
            );
            return HashMap::new();
        };
        let mut map = HashMap::new();
        for (ns, ns_value) in root {
            let Some(entries) = ns_value.as_table() else {
                continue;
            };
            for (key, entry_value) in entries {
                let full_key = format!("{}.{}", ns, key);
                let Some(entry) = entry_value.as_table() else {
                    tracing::warn!("i18n entry {} malformed (expected a table)", full_key);
                    continue;
                };
                let zh = entry
                    .get("zh")
                    .and_then(Value::as_str)
                    .unwrap_or(key)
                    .to_string();
                let en = match entry.get("en") {
                    Some(Value::String(s)) => EnText::Plain(s.clone()),
                    Some(Value::Table(t)) => EnText::Plural {
                        one: t
                            .get("one")
                            .and_then(Value::as_str)
                            .unwrap_or(&zh)
                            .to_string(),
                        other: t
                            .get("other")
                            .and_then(Value::as_str)
                            .unwrap_or(&zh)
                            .to_string(),
                    },
                    _ => EnText::Plain(zh.clone()),
                };
                map.insert(full_key, Entry { zh, en });
            }
        }
        map
    })
}

// ── 语言状态 ──────────────────────────────────────────────────────────────

use std::sync::atomic::{AtomicU8, Ordering};

const LANG_ZH: u8 = 0;
const LANG_EN: u8 = 1;

static CURRENT_LANG: AtomicU8 = AtomicU8::new(LANG_ZH);

/// 同步前端语言（"zh-CN" / "en"，大小写不敏感前缀匹配：en* → 英文，其余 → 中文）。
pub fn set_language(locale: &str) {
    let lang = if locale.to_ascii_lowercase().starts_with("en") {
        LANG_EN
    } else {
        LANG_ZH
    };
    CURRENT_LANG.store(lang, Ordering::Relaxed);
}

/// 当前是否为英文语言。
#[inline]
pub fn is_en() -> bool {
    CURRENT_LANG.load(Ordering::Relaxed) == LANG_EN
}

// ── 查表 ──────────────────────────────────────────────────────────────────

/// 取当前语言下的文本（en 复数条目无 count 时取 other 分支）。
fn resolve(key: &str) -> &str {
    match table().get(key) {
        Some(e) => {
            if is_en() {
                match &e.en {
                    EnText::Plain(s) => s,
                    EnText::Plural { other, .. } => other,
                }
            } else {
                &e.zh
            }
        }
        None => {
            tracing::warn!("i18n key missing: {}", key);
            key
        }
    }
}

/// 无参数查表：返回当前语言文本（&'static str，来自静态表或 key 本身）。
pub fn lookup(key: &'static str) -> &'static str {
    resolve(key)
}

/// 带命名占位符查表：`{name}` → 传入值；返回新 String（非静态）。
/// 用法：`lookup_fmt("recorder.timeout", &[("name", &anchor_name)])`
pub fn lookup_fmt(key: &str, args: &[(&str, &str)]) -> String {
    let base = resolve(key);
    if args.is_empty() {
        return base.to_string();
    }
    let mut s = base.to_string();
    for (k, v) in args {
        s = s.replace(&format!("{{{}}}", k), v);
    }
    s
}

/// 复数查表：en 按 count 选 one/other（count==1 → one，其余 → other）；
/// zh 直接返回原文。count 同时以 {count} 占位符注入。
pub fn lookup_plural(key: &str, count: u64, args: &[(&str, &str)]) -> String {
    let text = match table().get(key) {
        Some(e) if is_en() => match &e.en {
            EnText::Plain(s) => s.clone(),
            EnText::Plural { one, other } => {
                if count == 1 {
                    one.clone()
                } else {
                    other.clone()
                }
            }
        },
        Some(e) => e.zh.clone(),
        None => {
            tracing::warn!("i18n key missing: {}", key);
            key.to_string()
        }
    };
    let mut s = text;
    s = s.replace("{count}", &count.to_string());
    for (k, v) in args {
        s = s.replace(&format!("{{{}}}", k), v);
    }
    s
}

// ── tr! 宏 ────────────────────────────────────────────────────────────────
// 调用点：`use crate::tr;` 后 `tr!("key")` / `tr!("key", name = value)`。
// value 需实现 Display（String/&str/u64/...）。

/// 后端 i18n 消息宏：`tr!("config.save_ok")` 或
/// `tr!("recorder.timeout", name = anchor_name)`（命名占位符替换）。
#[macro_export]
macro_rules! tr {
    ($key:literal) => {
        $crate::infrastructure::i18n::lookup($key)
    };
    ($key:literal, $($arg:ident = $val:expr),+ $(,)?) => {
        $crate::infrastructure::i18n::lookup_fmt($key, &[$( (stringify!($arg), &$val.to_string()) ),+])
    };
}

/// 复数消息宏：`tr_plural!("cleanup.files_removed", count, ...)`。
#[macro_export]
macro_rules! tr_plural {
    ($key:literal, $count:expr $(, $arg:ident = $val:expr)* $(,)?) => {
        $crate::infrastructure::i18n::lookup_plural($key, $count, &[$( (stringify!($arg), &$val.to_string()) ),*])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    // 语言状态是全局单例，依赖它的断言必须合并为单个测试顺序执行
    //（Rust 测试默认并行，拆分多个函数会互相覆盖语言导致竞态）。
    #[test]
    fn i18n_lookup_behavior() {
        set_language("zh-CN");
        assert_eq!(tr!("config.save_ok"), "配置保存成功");
        assert_eq!(tr_plural!("config.save_ok", 1), "配置保存成功");
        // 缺失 key → 返回 key 本身（故意缺失的回退测试，直调 lookup 避免
        // 被 all_used_keys_exist_in_table 误报）
        assert_eq!(lookup("no.such_key"), "no.such_key");
        assert_eq!(
            tr!("config.save_failed_body", err = "boom"),
            "保存配置文件时出错：boom"
        );

        set_language("en");
        assert_eq!(tr!("config.save_ok"), "Configuration saved successfully");
        assert_eq!(
            tr!("config.save_failed_body", err = "boom"),
            "Error saving configuration file: boom"
        );
        // 无真实复数条目时回退普通英文文本
        assert_eq!(
            tr_plural!("config.save_ok", 1),
            "Configuration saved successfully"
        );
        assert_eq!(
            tr_plural!("config.save_ok", 2),
            "Configuration saved successfully"
        );
        // 复位语言，避免污染其他并行测试（语言状态是全局单例，Rust 测试默认并行）
        set_language("zh-CN");
    }

    #[test]
    fn table_contains_namespaced_keys() {
        let t = table();
        assert!(t.contains_key("config.save_ok"));
        assert!(t.contains_key("config.save_failed_body"));
        // 条目 zh 与占位符完整
        assert!(t["config.save_failed_body"].zh.contains("{err}"));
    }

    /// 全量 key 校验：代码中所有 `tr!` / `tr_plural!` 字面量 key 必须存在于
    /// i18n.toml（M5：错拼 key 会在 UI 显示裸 key 字符串，字符串字面量无法
    /// 在编译期检查，此处兜底）。扫描 src 目录全部 .rs 文件。
    #[test]
    fn all_used_keys_exist_in_table() {
        let t = table();
        let mut missing: Vec<String> = Vec::new();
        let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![src_dir];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("read src dir") {
                let entry = entry.expect("dir entry");
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let content = std::fs::read_to_string(&path).expect("read rs file");
                    for key in extract_tr_keys(&content) {
                        if !t.contains_key(key) {
                            missing.push(format!("{}: {}", path.display(), key));
                        }
                    }
                }
            }
        }
        assert!(
            missing.is_empty(),
            "used i18n keys missing from i18n.toml:\n{}",
            missing.join("\n")
        );
    }

    /// 提取 `tr!("a.b")` / `tr_plural!("a.b", ...)` 中的 key 字面量。
    /// 按行扫描：跳过注释行（文档示例非真实调用）、路径样式（include_str! 等）。
    fn extract_tr_keys(content: &str) -> Vec<&str> {
        let mut keys = Vec::new();
        for line in content.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue; // 注释/文档示例
            }
            for marker in ["tr!(\"", "tr_plural!(\""] {
                let mut rest = line;
                while let Some(idx) = rest.find(marker) {
                    let after = &rest[idx + marker.len()..];
                    if let Some(end) = after.find('"') {
                        let key = &after[..end];
                        if !key.starts_with("./") && !key.starts_with("../") {
                            keys.push(key);
                        }
                    }
                    rest = &rest[idx + marker.len()..];
                }
            }
        }
        keys
    }
}
