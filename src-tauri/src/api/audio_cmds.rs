//! 内置播放器 HTTP 流式音源命令（H6）。
//!
//! 前端不再用 `convertFileSrc`（asset://）或 blob URL，改为向后端要一个
//! `http://127.0.0.1:PORT/{token}/{id}` 流式 URL 设给 `<audio>`。全平台统一：
//! WebKitGTK 与 Chromium 对 127.0.0.1 http 都是原生支持（含 Range/206 seek），
//! 也不整文件进内存（边下边播），不暴露绝对路径。
use std::sync::Arc;

use tauri::State;

use super::http_audio::HttpAudioServer;

/// 请求播放某个录音文件，返回可设给 `<audio>.src` 的流式 URL。
///
/// - 校验文件存在 → 注册到服务白名单（id 映射）→ 返回带 token 的完整 URL；
/// - 前端拿到后设 `audio.src = url`，播放结束/切歌调用 `stop_http` 释放；
/// - 返回 `Err`（文件缺失/非文件/服务未启动）时前端提示播放失败。
#[tauri::command]
pub async fn play_http(
    server: State<'_, Option<Arc<HttpAudioServer>>>,
    path: String,
) -> Result<String, String> {
    let Some(server) = server.as_ref() else {
        return Err("播放服务不可用".into());
    };
    server.register(&std::path::PathBuf::from(&path))
}

/// 停止/释放当前播放（切歌或停止时调用），旧 URL 立即失效（404）。
/// 撤销整个 token（全部作废）——当前单播放场景够用。
#[tauri::command]
pub async fn stop_http(server: State<'_, Option<Arc<HttpAudioServer>>>) -> Result<(), String> {
    if let Some(server) = server.as_ref() {
        server.revoke_token();
    }
    Ok(())
}