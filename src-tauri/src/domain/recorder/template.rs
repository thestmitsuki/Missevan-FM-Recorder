//! 文件名模板渲染（§11.1 `filename_template` 接线：不再硬编码输出路径）。
//!
//! 支持变量：
//! - `{anchor_name}` 主播名
//! - `{room_id}` 房间号
//! - `{date}` 日期（YYYY-MM-DD）
//! - `{time}` 时间（HH-MM-SS）
//! - `{index}` 录制序号（3 位补零，001 起；每主播单调递增，见 AppState.recording_seq）
//! - `{ext}` 录制格式扩展名（m4a / mp3）
//!
//! 安全与兼容：
//! - 渲染结果按 `/` 或 `\` 拆分为路径组件后逐组件 `sanitize_path_component`
//!   （Windows 非法字符/控制字符 → `_`，`..` 子串 → `_`），再以 `/` 拼接——
//!   模板无法通过 `..` 或绝对路径逃逸出输出目录；
//! - 模板为空 / 不含任何变量时回退默认模板
//!   `{anchor_name}/{date}_{time}_{anchor_name}_{index}.{ext}`；渲染后无有效路径
//!   组件（如模板仅含分隔符）同样回退默认；
//! - 旧输出（无模板时代的 `{主播名}-{房间号}/{主播名}_{时间戳}.{ext}`）不受影响：
//!   渲染只作用于新录制的输出路径，已存在文件/目录原样保留。
//!
//! 默认模板含 `{index}` 的原因（实装审查跟进，数据丢失风险）：默认模板若不含
//! 序号，两个同名主播（不同房间号）同时录制 → 同目录同秒同名文件 → ffmpeg `-y`
//! 互相覆盖。`{index}` 是**每主播**单调递增序号（见 AppState.recording_seq），
//! 消除同一主播重复录制的碰撞；跨主播同秒碰撞由 engine 的「目标文件已存在则
//! 自动追加序号」（deduplicate_output_path）兜底。

use chrono::{DateTime, Local};

use crate::domain::recorder::engine::sanitize_path_component;

/// 默认模板（与 GlobalConfig 默认值一致）
pub const DEFAULT_TEMPLATE: &str = "{anchor_name}/{date}_{time}_{anchor_name}_{index}.{ext}";

/// 模板渲染上下文
pub struct TemplateContext<'a> {
    pub anchor_name: &'a str,
    pub room_id: &'a str,
    pub now: DateTime<Local>,
    /// 录制序号（1 起；渲染为 3 位补零，上限 999）
    pub index: u32,
    /// 录制格式扩展名（m4a / mp3）
    pub ext: &'a str,
}

/// 渲染文件名模板 → 相对输出目录的路径（分隔符统一 `/`）。
///
/// 若模板为空白或渲染后无有效组件，回退默认模板（不会无限递归——
/// 默认模板含全部必需变量，渲染结果恒非空）。
pub fn render_filename_template(template: &str, ctx: &TemplateContext) -> String {
    let rendered = replace_variables(template, ctx);
    let components: Vec<String> = rendered
        .split(['/', '\\'])
        .filter(|c| !c.is_empty())
        .map(sanitize_path_component)
        .collect();
    if components.is_empty() {
        let default_rendered = replace_variables(DEFAULT_TEMPLATE, ctx);
        let default_components: Vec<String> = default_rendered
            .split(['/', '\\'])
            .filter(|c| !c.is_empty())
            .map(sanitize_path_component)
            .collect();
        return default_components.join("/");
    }
    components.join("/")
}

/// 变量替换（纯文本替换，不做消毒——消毒在组件拆分后统一进行）
fn replace_variables(template: &str, ctx: &TemplateContext) -> String {
    let tpl = if template.trim().is_empty() || !template.contains('{') {
        DEFAULT_TEMPLATE
    } else {
        template
    };
    let date = ctx.now.format("%Y-%m-%d").to_string();
    let time = ctx.now.format("%H-%M-%S").to_string();
    let index = format!("{:03}", ctx.index.min(999));
    tpl.replace("{anchor_name}", ctx.anchor_name)
        .replace("{room_id}", ctx.room_id)
        .replace("{date}", &date)
        .replace("{time}", &time)
        .replace("{index}", &index)
        .replace("{ext}", ctx.ext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ctx() -> TemplateContext<'static> {
        TemplateContext {
            anchor_name: "主播A",
            room_id: "123456",
            now: chrono::Local
                .with_ymd_and_hms(2026, 8, 7, 12, 30, 45)
                .unwrap(),
            index: 1,
            ext: "m4a",
        }
    }

    #[test]
    fn default_template_renders_subdir_and_variables() {
        let rendered = render_filename_template(DEFAULT_TEMPLATE, &ctx());
        assert_eq!(rendered, "主播A/2026-08-07_12-30-45_主播A_001.m4a");
    }

    #[test]
    fn default_template_contains_index_to_avoid_collisions() {
        // 实装审查跟进（数据丢失风险）：默认模板必须含 {index}——两个同名主播
        // 同时录制时同目录同秒文件名会互相覆盖（ffmpeg -y）。默认模板是
        // 回退路径（空模板/无变量模板）的最终保障。
        assert!(
            DEFAULT_TEMPLATE.contains("{index}"),
            "默认模板必须含 {{index}}: {}",
            DEFAULT_TEMPLATE
        );
        // 同一默认模板下序号不同 → 文件名不同（同秒重复录制不覆盖）
        let mut c1 = ctx();
        c1.index = 1;
        let mut c2 = ctx();
        c2.index = 2;
        let a = render_filename_template("", &c1);
        let b = render_filename_template("", &c2);
        assert_ne!(a, b, "不同序号必须渲染出不同文件名: {} vs {}", a, b);
    }

    #[test]
    fn render_full_path_directory_and_filename_parts() {
        // 渲染结果含完整相对路径：目录部分与音频文件名部分都来自模板
        let rendered = render_filename_template(
            "{anchor_name}-{room_id}/{index}_{anchor_name}.{ext}",
            &ctx(),
        );
        assert_eq!(rendered, "主播A-123456/001_主播A.m4a");
        let (dir, file) = rendered.rsplit_once('/').expect("模板含子目录");
        assert_eq!(dir, "主播A-123456", "目录部分来自模板");
        assert_eq!(file, "001_主播A.m4a", "音频文件名部分来自模板");
    }

    #[test]
    fn render_no_subdir_template_yields_single_component() {
        // 模板无子目录：渲染结果为单组件路径（无 '/'）——录制时不建多余目录
        let rendered = render_filename_template(
            "{date}_{time}_{anchor_name}_{index}.{ext}",
            &ctx(),
        );
        assert_eq!(rendered, "2026-08-07_12-30-45_主播A_001.m4a");
        assert!(
            !rendered.contains('/'),
            "无子目录模板不得产生路径分隔符: {}",
            rendered
        );
    }

    #[test]
    fn render_multi_level_subdir_preserves_hierarchy() {
        let rendered =
            render_filename_template("{anchor_name}/{date}/{time}_{index}.{ext}", &ctx());
        let (dir, file) = rendered.rsplit_once('/').expect("多级子目录");
        assert_eq!(dir, "主播A/2026-08-07");
        assert_eq!(file, "12-30-45_001.m4a");
    }

    #[test]
    fn old_default_template_without_index_renders() {
        // 旧默认模板（含 {anchor_name}/ 子目录、无 {index}）兼容：旧配置里的
        // 模板由 serde 原样保留（见 model.rs 测试），渲染不失败且目录/文件名
        // 部分各自正确
        let rendered = render_filename_template(
            "{anchor_name}/{date}_{time}_{anchor_name}.{ext}",
            &ctx(),
        );
        assert_eq!(rendered, "主播A/2026-08-07_12-30-45_主播A.m4a");
        let (dir, file) = rendered.rsplit_once('/').unwrap();
        assert_eq!(dir, "主播A");
        assert_eq!(file, "2026-08-07_12-30-45_主播A.m4a");
    }

    #[test]
    fn all_variables_render() {
        let rendered = render_filename_template(
            "{index}_{anchor_name}_{room_id}_{date}_{time}.{ext}",
            &ctx(),
        );
        assert_eq!(rendered, "001_主播A_123456_2026-08-07_12-30-45.m4a");
    }

    #[test]
    fn index_zero_pads_to_three_digits() {
        let mut c = ctx();
        c.index = 1;
        assert_eq!(
            render_filename_template("{index}", &c),
            "001",
            "序号 1 → 001"
        );
        c.index = 42;
        assert_eq!(render_filename_template("{index}", &c), "042");
        c.index = 999;
        assert_eq!(render_filename_template("{index}", &c), "999");
        c.index = 1234;
        assert_eq!(
            render_filename_template("{index}", &c),
            "999",
            "超出 999 截断到 999"
        );
    }

    #[test]
    fn empty_template_falls_back_to_default() {
        assert_eq!(
            render_filename_template("", &ctx()),
            "主播A/2026-08-07_12-30-45_主播A_001.m4a"
        );
        assert_eq!(
            render_filename_template("   ", &ctx()),
            "主播A/2026-08-07_12-30-45_主播A_001.m4a"
        );
        // 不含任何变量（`{` 都不存在）→ 回退默认
        assert_eq!(
            render_filename_template("literal-name", &ctx()),
            "主播A/2026-08-07_12-30-45_主播A_001.m4a"
        );
    }

    #[test]
    fn separator_only_template_falls_back_to_default() {
        // 仅含分隔符 → 拆分后无有效组件 → 回退默认
        assert_eq!(
            render_filename_template("/", &ctx()),
            "主播A/2026-08-07_12-30-45_主播A_001.m4a"
        );
        assert_eq!(
            render_filename_template("\\/", &ctx()),
            "主播A/2026-08-07_12-30-45_主播A_001.m4a"
        );
    }

    #[test]
    fn backslash_separators_are_supported() {
        // 模板可用反斜杠分隔目录（Windows 习惯）
        let rendered = render_filename_template("rec\\{anchor_name}.{ext}", &ctx());
        assert_eq!(rendered, "rec/主播A.m4a");
    }

    #[test]
    fn traversal_components_are_sanitized() {
        // 路径穿越注入：.. / ../.. 与绝对路径前缀全部消毒，无法逃逸输出目录
        let rendered = render_filename_template("../../evil/{anchor_name}.{ext}", &ctx());
        assert!(
            !rendered.contains(".."),
            "渲染结果不得含 .. 段: {}",
            rendered
        );
        assert!(rendered.starts_with('_'), ".. 段应被消毒为 _: {}", rendered);

        let rendered2 = render_filename_template("C:\\windows\\{anchor_name}.{ext}", &ctx());
        assert!(
            !rendered2.contains(':') && !rendered2.contains('\\'),
            "盘符与反斜杠应被消毒: {}",
            rendered2
        );
    }

    #[test]
    fn illegal_chars_in_anchor_name_are_sanitized() {
        let mut c = ctx();
        c.anchor_name = "主播:A/B*";
        let rendered = render_filename_template("{anchor_name}/{date}.{ext}", &c);
        // 值内的 `/` 把路径拆成两个组件（模板本身把它当目录分隔符），
        // 每个组件再消毒：`:` → `_`、`*` → `_`，不产生穿越段
        assert_eq!(rendered, "主播_A/B_/2026-08-07.m4a");
    }

    #[test]
    fn room_id_and_ext_are_substituted() {
        let mut c = ctx();
        c.ext = "mp3";
        let rendered = render_filename_template("{room_id}/{anchor_name}.{ext}", &c);
        assert_eq!(rendered, "123456/主播A.mp3");
    }
}
