pub mod r#loop;
pub mod stats;

/// 双重验证归并：直播展示状态 = API 检测结果 || 正在录制
///
/// 规格「直播状态异常修复」：
/// - API 判直播但无录制行为 → **仍是直播**（录制可能因门控未启动：
///   enable_check=false / 并发上限 / 流地址缺失，均不影响「直播中」展示）；
/// - API 判离线但录制进行中 → **保持「直播中」**（FFmpeg 正在录说明流存在，
///   离线可能是 API 抖动 / 风控误报）。
///
/// 归并点选在**后端状态生产处**（检测循环事件推送 / get_recording_status /
/// 调试统计聚合）而非前端展示层，理由见 live-page-fix-report.md：
/// 录制状态有 engine / monitor / detector 三个推送来源，各来源的 is_live
/// 语义不一致；单一归并函数在后端统一成「合并后的直播事实」下发，
/// 前端无需重复归并逻辑，也不存在事件时序上的显示回闪。
pub fn merge_live_state(api_live: bool, is_recording: bool) -> bool {
    api_live || is_recording
}

#[cfg(test)]
mod tests {
    use super::merge_live_state;

    #[test]
    fn offline_and_not_recording_is_offline() {
        assert!(!merge_live_state(false, false));
    }

    #[test]
    fn api_live_without_recording_is_still_live() {
        // 门控（enable_check=false / 并发上限等）未启动录制不影响直播展示
        assert!(merge_live_state(true, false));
    }

    #[test]
    fn api_offline_while_recording_stays_live() {
        // FFmpeg 正在录说明流存在：API 误报离线不翻转直播状态
        assert!(merge_live_state(false, true));
    }

    #[test]
    fn api_live_while_recording_is_live() {
        assert!(merge_live_state(true, true));
    }
}
