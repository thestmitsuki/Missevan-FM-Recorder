//! 文件名模板渲染（§11.1 `filename_template` 接线：不再硬编码输出路径）。
//!
//! 支持变量：
//! - `{anchor_name}` 主播名
//! - `{room_id}` 房间号
//! - `{date}` 日期（YYYY-MM-DD）
//! - `{time}` 时间（HH-MM-SS）
//! - `{ext}` 录制格式扩展名（m4a / mp3）
//!
//! 安全与兼容：
//! - 渲染结果按 `/` 或 `\` 拆分为路径组件后逐组件 `sanitize_path_component`
//!   （Windows 非法字符/控制字符 → `_`，`..` 子串 → `_`），再以 `/` 拼接——
//!   模板无法通过 `..` 或绝对路径逃逸出输出目录；
//! - 模板为空 / 不含任何变量时回退默认模板
//!   `{anchor_name}/{date}_{time}_{anchor_name}.{ext}`；渲染后无有效路径
//!   组件（如模板仅含分隔符）同样回退默认；
//! - 旧输出（无模板时代的 `{主播名}-{房间号}/{主播名}_{时间戳}.{ext}`）不受影响：
//!   渲染只作用于新录制的输出路径，已存在文件/目录原样保留。
//!
//! 默认模板不含录制序号的原因（实装审查跟进）：早期默认模板含 `{index}`（3 位
//! 补零，每主播单调递增），用于防「同主播同秒重复录制」文件名相同被 ffmpeg `-y`
//! 覆盖。该变量已判定冗余并移除——非分段模式下 engine 的 `deduplicate_output_path`
//! （目标文件已存在 → 扩展名前自动追加 `_2`/`_3`…）已兜底同秒碰撞与上次残留，
//! 且能覆盖 `{index}` 防不住的两个同名主播同秒碰撞；分段模式由 ffmpeg `%03d`
//! 序号管理，不依赖模板。`{index}` 从变量集移除后，用户自定义模板中残留的
//! `{index}` 作为**未识别变量原样保留**（纯文本替换语义，见 `replace_variables`
//! 与测试 `unknown_index_variable_is_kept_literal`），同名碰撞仍由
//! `deduplicate_output_path` 兜底。

use chrono::{DateTime, Local};

use crate::domain::recorder::engine::sanitize_path_component;

/// 默认模板（与 GlobalConfig 默认值一致）
pub const DEFAULT_TEMPLATE: &str = "{anchor_name}/{date}_{time}_{anchor_name}.{ext}";

/// 模板渲染上下文
pub struct TemplateContext<'a> {
    pub anchor_name: &'a str,
    pub room_id: &'a str,
    pub now: DateTime<Local>,
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

/// 变量替换（纯文本替换，不做消毒——消毒在组件拆分后统一进行）。
/// 未识别的变量（如用户自定义模板中残留的 `{index}`）**原样保留**，不删除、
/// 不报错——同名碰撞由 engine `deduplicate_output_path` 兜底。
fn replace_variables(template: &str, ctx: &TemplateContext) -> String {
    let tpl = if template.trim().is_empty() || !template.contains('{') {
        DEFAULT_TEMPLATE
    } else {
        template
    };
    let date = ctx.now.format("%Y-%m-%d").to_string();
    let time = ctx.now.format("%H-%M-%S").to_string();
    tpl.replace("{anchor_name}", ctx.anchor_name)
        .replace("{room_id}", ctx.room_id)
        .replace("{date}", &date)
        .replace("{time}", &time)
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
            ext: "m4a",
        }
    }

    #[test]
    fn default_template_renders_subdir_and_variables() {
        let rendered = render_filename_template(DEFAULT_TEMPLATE, &ctx());
        assert_eq!(rendered, "主播A/2026-08-07_12-30-45_主播A.m4a");
    }

    #[test]
    fn default_template_has_no_recording_index() {
        // 实装审查跟进：`{index}` 录制序号已从默认模板移除——非分段模式的
        // 同秒碰撞/上次残留由 engine `deduplicate_output_path` 兜底（目标文件
        // 已存在自动追加 _2/_3…），分段模式由 ffmpeg %03d 管理。默认模板作为
        // 空模板/无变量模板的回退路径，渲染结果不得再带 `_001` 尾部序号。
        assert!(
            !DEFAULT_TEMPLATE.contains("{index}"),
            "默认模板不得含 {{index}}: {}",
            DEFAULT_TEMPLATE
        );
        let rendered = render_filename_template("", &ctx());
        assert!(
            !rendered.contains("_001"),
            "默认模板渲染结果不得带录制序号: {}",
            rendered
        );
    }

    #[test]
    fn unknown_index_variable_is_kept_literal() {
        // 用户自定义模板中残留的 `{index}`（旧默认模板/旧配置）：不是已识别
        // 变量 → 纯文本替换后**原样保留**在渲染结果中（不删除、不报错）。
        // 同名碰撞仍由输出路径去重（deduplicate_output_path）兜底，`{index}`
        // 仅作为字面文件名成分存在。
        let rendered = render_filename_template(
            "{date}_{time}_{anchor_name}_{index}.{ext}",
            &ctx(),
        );
        assert_eq!(rendered, "2026-08-07_12-30-45_主播A_{index}.m4a");
    }

    #[test]
    fn render_full_path_directory_and_filename_parts() {
        // 渲染结果含完整相对路径：目录部分与音频文件名部分都来自模板
        let rendered = render_filename_template(
            "{anchor_name}-{room_id}/{anchor_name}.{ext}",
            &ctx(),
        );
        assert_eq!(rendered, "主播A-123456/主播A.m4a");
        let (dir, file) = rendered.rsplit_once('/').expect("模板含子目录");
        assert_eq!(dir, "主播A-123456", "目录部分来自模板");
        assert_eq!(file, "主播A.m4a", "音频文件名部分来自模板");
    }

    #[test]
    fn render_no_subdir_template_yields_single_component() {
        // 模板无子目录：渲染结果为单组件路径（无 '/'）——录制时不建多余目录
        let rendered = render_filename_template(
            "{date}_{time}_{anchor_name}.{ext}",
            &ctx(),
        );
        assert_eq!(rendered, "2026-08-07_12-30-45_主播A.m4a");
        assert!(
            !rendered.contains('/'),
            "无子目录模板不得产生路径分隔符: {}",
            rendered
        );
    }

    #[test]
    fn render_multi_level_subdir_preserves_hierarchy() {
        let rendered =
            render_filename_template("{anchor_name}/{date}/{time}.{ext}", &ctx());
        let (dir, file) = rendered.rsplit_once('/').expect("多级子目录");
        assert_eq!(dir, "主播A/2026-08-07");
        assert_eq!(file, "12-30-45.m4a");
    }

    #[test]
    fn old_default_template_without_index_renders() {
        // 旧默认模板（含 {anchor_name}/ 子目录、无 {index}）与当前默认模板一致：
        // 旧配置里的模板由 serde 原样保留（见 model.rs 测试），渲染不失败且
        // 目录/文件名部分各自正确
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
            "{anchor_name}_{room_id}_{date}_{time}.{ext}",
            &ctx(),
        );
        assert_eq!(rendered, "主播A_123456_2026-08-07_12-30-45.m4a");
    }

    #[test]
    fn empty_template_falls_back_to_default() {
        assert_eq!(
            render_filename_template("", &ctx()),
            "主播A/2026-08-07_12-30-45_主播A.m4a"
        );
        assert_eq!(
            render_filename_template("   ", &ctx()),
            "主播A/2026-08-07_12-30-45_主播A.m4a"
        );
        // 不含任何变量（`{` 都不存在）→ 回退默认
        assert_eq!(
            render_filename_template("literal-name", &ctx()),
            "主播A/2026-08-07_12-30-45_主播A.m4a"
        );
    }

    #[test]
    fn separator_only_template_falls_back_to_default() {
        // 仅含分隔符 → 拆分后无有效组件 → 回退默认
        assert_eq!(
            render_filename_template("/", &ctx()),
            "主播A/2026-08-07_12-30-45_主播A.m4a"
        );
        assert_eq!(
            render_filename_template("\\/", &ctx()),
            "主播A/2026-08-07_12-30-45_主播A.m4a"
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
