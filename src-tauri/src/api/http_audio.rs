//! 本地 HTTP 流式播放服务（全平台统一音源，H6）。
//!
//! ## 背景
//! Linux（WebKitGTK）媒体栈走 GStreamer，不认 Tauri 的 `asset://` 自定义 scheme
//!（gsturidecodebin 无 "asset" URI handler）；此前 Linux 用 blob URL workaround
//!（fetch asset:// → createObjectURL），但整文件进内存——300MB 录音首播秒级卡顿、
//! 常驻 300MB，且 blob:`tauri://localhost` 桥接在部分环境仍不稳。
//!
//! ## 方案（H6）
//! 后端启动一个**仅回环（127.0.0.1）+ 随机端口 + token 鉴权 + id 白名单映射**的
//! 极简 HTTP 流式服务，前端 `<audio src="http://127.0.0.1:PORT/{token}/{id}">`
//! 直连。`http://127.0.0.1` 是 WebKitGTK 媒体栈 / Chromium 原生支持的 scheme，
//! 全平台统一、支持 Range/206 流式 seek、不整文件进内存（边下边播）。
//!
//! ## 安全模型（Q2 确认）
//! - **仅回环**：绑定 127.0.0.1，不暴露局域网/公网；
//! - **token 鉴权**：每次启动生成随机 token，URL 路径首段为 token，不匹配即 404；
//! - **id 白名单映射**：URL 路径第二段是后端自管的自增 id，服务端按 id 查绝对路径
//!   （后端持有 `HashMap<id, 绝对路径>`）——前端只拿到完整 URL 字符串，**接触不到
//!   绝对路径与 token 本身**，也无目录遍历（路径不是前端传的 raw 字符串）；
//! - **只读 + Range**：仅响应 GET/HEAD，支持 Range/206（seek 必需），不写任何文件；
//! - **生命周期**：持久监听一次（Q1=A），应用退出（App 信号）即停；换文件/停止时
//!   前端 invoke 撤销旧 id，旧 URL 立即 404。
//!
//! ## 实现要点
//! tiny_http 0.12（轻量、零依赖）在独立线程循环 recv；文件用 `Response::from_file`
//! 直接服务（tiny_http 处理 body 流式写），配合 Content-Range/Accept-Ranges 头实现
//! 206。服务全生命周期由 `HttpAudioServer` 持有，随 AppState 管理。
use std::collections::HashMap;
use std::io::Read;
use std::io::SeekFrom;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rand::Rng;

use crate::tr;

/// 服务内部状态（线程间共享）：id → 绝对路径 映射与当前 token。
#[derive(Default)]
struct Inner {
    /// 自增文件 id → 绝对路径（白名单映射表）。
    files: HashMap<u64, PathBuf>,
    /// 当前有效 token（URL 首段）；换 token 即令旧 URL 全部失效。
    token: String,
}

/// HTTP 流式播放服务句柄（挂 AppState）。
pub struct HttpAudioServer {
    inner: Arc<Mutex<Inner>>,
    base_url: String,
    server: tiny_http::Server,
    serving: Arc<Mutex<bool>>,
}

impl HttpAudioServer {
    /// 启动回环 HTTP 服务，返回句柄（base URL 如 `http://127.0.0.1:PORT`）。
    pub fn start() -> Result<Self, String> {
        // 随机端口：tiny_http 绑定 127.0.0.1:0（OS 分配空闲端口）。
        let server = tiny_http::Server::http("127.0.0.1:0")
            .map_err(|e| tr!("player.service_start_failed", err = e))?;
        let port = server.server_addr().to_ip().map(|s| s.port()).unwrap_or(0);
        let base_url = format!("http://127.0.0.1:{port}");

        let inner = Arc::new(Mutex::new(Inner {
            files: HashMap::new(),
            token: Self::gen_token(),
        }));

        Ok(Self {
            inner,
            base_url,
            server,
            serving: Arc::new(Mutex::new(true)),
        })
    }

    /// 生成 URL 随机 token（32 位 hex，熵足够防猜测）。
    fn gen_token() -> String {
        let mut rng = rand::thread_rng();
        (0..16).map(|_| format!("{:02x}", rng.gen::<u8>())).collect()
    }

    /// 在主线程驱动 recv 循环（由调用方在独立任务/线程中阻塞执行）。
    /// 返回后服务停止。循环内逐请求处理：URL 鉴权 → Range 响应。
    pub fn run(&self) {
        while *self.serving.lock().unwrap() {
            match self.server.recv() {
                Ok(mut req) => {
                    let resp = self.handle_request(&mut req);
                    let _ = req.respond(resp);
                }
                Err(_) => break,
            }
        }
    }

    /// 处理单请求：仅 GET/HEAD + token/id 鉴权 + Range/206。
    fn handle_request(
        &self,
        req: &mut tiny_http::Request,
    ) -> tiny_http::Response<Box<dyn std::io::Read + Send + 'static>> {
        let not_get_or_head =
            req.method() != &tiny_http::Method::Get && req.method() != &tiny_http::Method::Head;
        if not_get_or_head {
            return Self::not_found();
        }

        // URL 解析：/{token}/{id}
        let url = req.url().to_string();
        let segments: Vec<&str> = url.trim_start_matches('/').split('/').collect();
        if segments.len() != 2 {
            return Self::not_found();
        }
        let (token, id_str) = (segments[0], segments[1]);
        let id: u64 = match id_str.parse() {
            Ok(v) => v,
            Err(_) => return Self::not_found(),
        };

        let guard = self.inner.lock().unwrap();
        if guard.token != token {
            return Self::not_found();
        }
        let Some(path) = guard.files.get(&id) else {
            return Self::not_found();
        };
        let path = path.clone();
        drop(guard);

        Self::serve_file(req, &path)
    }

    /// 服务单个文件（支持 Range/206）。
    fn serve_file(
        req: &tiny_http::Request,
        path: &std::path::Path,
    ) -> tiny_http::Response<Box<dyn std::io::Read + Send + 'static>> {
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(_) => return Self::not_found(),
        };
        let total = match file.metadata() {
            Ok(m) => m.len(),
            Err(_) => return Self::not_found(),
        };
        let is_head = req.method() == &tiny_http::Method::Head;

        // Range 解析
        let (start, len) = Self::parse_range(req, total);
        if start >= total {
            return Self::range_not_satisfiable(total);
        }

        // 定位到 start（seek）后再 take len 字节
        let mut file = file;
        if std::io::Seek::seek(&mut file, SeekFrom::Start(start)).is_err() {
            return Self::not_found();
        }
        let reader: Box<dyn std::io::Read + Send + 'static> =
            Box::new(std::io::BufReader::new(file).take(len));

        let mime = Self::guess_mime(path);
        let content_range = format!("bytes {}-{}/{}", start, start + len - 1, total);
        let mut headers: Vec<tiny_http::Header> = vec![
            tiny_http::Header::from_bytes("Accept-Ranges".as_bytes(), b"bytes").unwrap(),
            tiny_http::Header::from_bytes("Content-Type".as_bytes(), mime.as_bytes()).unwrap(),
            tiny_http::Header::from_bytes("Content-Length".as_bytes(), len.to_string().as_bytes()).unwrap(),
        ];
        let status = if start > 0 || len < total {
            headers.push(tiny_http::Header::from_bytes("Content-Range".as_bytes(), content_range.as_bytes()).unwrap());
            206
        } else {
            200
        };

        let body: Box<dyn std::io::Read + Send + 'static> = if is_head {
            // HEAD：无 body，仅响应头
            Box::new(std::io::empty())
        } else {
            reader
        };

        tiny_http::Response::new(
            tiny_http::StatusCode(status as u16),
            headers,
            body,
            Some(len as usize),
            None,
        )
    }

    /// 解析 Range 头，返回 (start, len)。纯字节范围，不做多段。
    fn parse_range(req: &tiny_http::Request, total: u64) -> (u64, u64) {
        let range = req
            .headers()
            .iter()
            .find(|h| h.field.equiv(&"Range"))
            .map(|h| h.value.as_str().to_string())
            .unwrap_or_default();
        Self::parse_range_from_spec(&range, total)
    }

    /// 由原始 Range spec 串（可能为空）解析 (start, len)，供 serve_file 与纯函数测试复用。
    fn parse_range_from_spec(spec_in: &str, total: u64) -> (u64, u64) {
        let spec = spec_in.trim();
        if spec.is_empty() {
            return (0, total);
        }
        // 只认 "bytes=..." 前缀（大小写不敏感）
        let lower = spec.to_ascii_lowercase();
        let Some(eq) = lower.find("bytes=") else {
            return (0, total);
        };
        let rest = &lower[eq + "bytes=".len()..];
        let s = rest.split(',').next().unwrap_or("").trim(); // 多段忽略，取第一段
        let Some(dash) = s.find('-') else {
            return (0, total);
        };
        let start_str = s[..dash].trim();
        let end_str = s[dash + 1..].trim();

        if start_str.is_empty() {
            // 后缀范围 -N：最后 N 字节
            let suffix: u64 = end_str.parse().unwrap_or(0);
            let start = total.saturating_sub(suffix);
            return (start, total - start);
        }
        let start: u64 = start_str.parse().unwrap_or(0);
        if end_str.is_empty() {
            (start, total.saturating_sub(start))
        } else {
            let end: u64 = end_str.parse().unwrap_or(total.saturating_sub(1));
            let last = end.min(total.saturating_sub(1));
            (start, last.saturating_sub(start).saturating_add(1))
        }
    }

    /// 根据扩展名猜测 MIME（音频）。
    fn guess_mime(path: &std::path::Path) -> String {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("mp3") => "audio/mpeg".into(),
            Some("m4a") | Some("mp4") => "audio/mp4".into(),
            Some("wav") => "audio/wav".into(),
            Some("ogg") => "audio/ogg".into(),
            Some("flac") => "audio/flac".into(),
            Some("aac") => "audio/aac".into(),
            _ => "application/octet-stream".into(),
        }
    }

    fn not_found() -> tiny_http::Response<Box<dyn std::io::Read + Send + 'static>> {
        tiny_http::Response::new(
            tiny_http::StatusCode(404),
            vec![],
            Box::new(std::io::Cursor::new(b"not found".to_vec())),
            Some(9),
            None,
        )
    }

    fn range_not_satisfiable(total: u64) -> tiny_http::Response<Box<dyn std::io::Read + Send + 'static>> {
        let body = format!("range not satisfiable, total={}", total).into_bytes();
        let len = body.len();
        tiny_http::Response::new(
            tiny_http::StatusCode(416),
            vec![],
            Box::new(std::io::Cursor::new(body)),
            Some(len),
            None,
        )
    }

    /// 注册一个要播放的文件，返回其可播放的完整 URL。
    /// 前端拿到 URL 后应尽快设给 audio.src；可多次调用（队列逐首注册）。
    pub fn register(&self, path: &std::path::Path) -> Result<String, String> {
        match std::fs::metadata(path) {
            Ok(m) if m.is_file() => {}
            Ok(_) => return Err(tr!("player.not_a_file", path = path.display())),
            Err(_) => return Err(tr!("player.file_not_found", path = path.display())),
        }
        let mut guard = self.inner.lock().unwrap();
        let id = next_id(&mut guard);
        guard.files.insert(id, path.to_path_buf());
        Ok(format!("{}/{}/{}", self.base_url, guard.token, id))
    }

    /// 撤销当前 token（整体作废，如停止全部播放时）。
    pub fn revoke_token(&self) {
        let mut guard = self.inner.lock().unwrap();
        guard.files.clear();
        guard.token = Self::gen_token();
    }

    /// 停止服务（exit 时调用）：置位 serving，主循环退出。
    /// 当前生命周期由主循环随 serving 标志退出驱动；本方法保留供显式关闭场景复用。
    #[allow(dead_code)]
    pub fn shutdown(&self) {
        *self.serving.lock().unwrap() = false;
    }
}

/// 自增 id 分配。
fn next_id(guard: &mut Inner) -> u64 {
    let mut next = 1u64;
    while guard.files.contains_key(&next) {
        next += 1;
    }
    next
}

#[cfg(test)]
mod tests {
    use super::HttpAudioServer;
    use super::{Inner, next_id};
    use std::collections::HashMap;

    fn parse_spec(spec: &str, total: u64) -> (u64, u64) {
        HttpAudioServer::parse_range_from_spec(spec, total)
    }

    #[test]
    fn range_full_file_without_header() {
        // 无 Range 头：整文件 (0, total)
        assert_eq!(parse_spec("", 300_000_000), (0, 300_000_000));
    }

    #[test]
    fn range_start_only() {
        // bytes=100- ：从 100 到末尾
        assert_eq!(parse_spec("bytes=100-", 1000), (100, 900));
    }

    #[test]
    fn range_bounded() {
        // bytes=100-199 ：100 字节
        assert_eq!(parse_spec("bytes=100-199", 1000), (100, 100));
    }

    #[test]
    fn range_suffix() {
        // bytes=-100 ：末尾 100 字节
        assert_eq!(parse_spec("bytes=-100", 1000), (900, 100));
    }

    #[test]
    fn range_beyond_total_clamps_end() {
        // bytes=50-9999 但只有 1000 字节：end 钳到 999
        assert_eq!(parse_spec("bytes=50-9999", 1000), (50, 950));
    }

    #[test]
    fn range_invalid_starts_at_zero() {
        // 非法 spec 回退整文件
        assert_eq!(parse_spec("bogus", 1000), (0, 1000));
    }

    #[test]
    fn next_id_skips_occupied() {
        let mut inner = Inner {
            files: HashMap::new(),
            token: String::new(),
        };
        inner.files.insert(1, "a".into());
        assert_eq!(next_id(&mut inner), 2);
        inner.files.insert(2, "b".into());
        assert_eq!(next_id(&mut inner), 3);
    }
}