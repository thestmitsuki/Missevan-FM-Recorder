use crate::domain::config::model::GlobalConfig;
use crate::infrastructure::error::types::AppError;
use crate::infrastructure::logging::network::{self, NetworkLog};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;

/// 直播检测结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveCheckResult {
    pub is_live: bool,
    pub anchor_name: Option<String>,
    pub title: Option<String>,
    pub stream_url: Option<String>,
    pub avatar: Option<String>,
}

/// 检测失败分类（检测循环据此决定重试策略与状态判定）
///
/// | 分类 | 触发条件 | 处理策略 | 状态判定 |
/// | --- | --- | --- | --- |
/// | `Server` | HTTP 5XX / 429 | 重试 + 指数退避；429 额外冷却 | 不判离线（未知） |
/// | `Network` | 连接失败 / 超时 / DNS 等本地网络问题 | 重试 + 指数退避 | 不判离线（未知） |
/// | `Format` | JSON 解析失败 / 结构不符（API 格式变化） | 记 warn，不重试 | 不判离线（未知） |
/// | `Other` | 其他（如 4XX 房间不存在） | 不重试 | 视为离线 |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckErrorKind {
    /// 服务器端错误（HTTP 5XX / 429）：不反映主播状态，重试 + 退避，不判离线
    Server,
    /// 本地网络错误（连接失败 / 超时 / DNS）：重试，短暂视为「未知」而非离线
    Network,
    /// 响应格式变化（JSON 解析失败）：记 warn，判定「未知」
    Format,
    /// 其他错误（如 4XX 房间不存在 / 403 拒绝访问）：视为离线
    Other,
}

/// `check_live` 的失败载体：分类 + 原始错误 + HTTP 状态码（429 判定冷却用）
#[derive(Debug)]
pub struct CheckError {
    pub kind: CheckErrorKind,
    pub status: Option<u16>,
    pub error: AppError,
}

impl CheckError {
    pub fn message(&self) -> &str {
        &self.error.message
    }

    /// 是否为可恢复的瞬时错误（Server/Network/Format）：
    /// 这类错误**不**构成「离线」证据，不应据此停止录制或翻转直播状态。
    pub fn is_transient(&self) -> bool {
        matches!(
            self.kind,
            CheckErrorKind::Server | CheckErrorKind::Network | CheckErrorKind::Format
        )
    }
}

impl std::fmt::Display for CheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{:?}] {}", self.kind, self.error)
    }
}

impl std::error::Error for CheckError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// HTTP 状态码 → 失败分类；成功（2XX）返回 None
pub fn classify_http_status(code: u16) -> Option<CheckErrorKind> {
    match code {
        429 => Some(CheckErrorKind::Server),
        500..=599 => Some(CheckErrorKind::Server),
        400..=499 => Some(CheckErrorKind::Other),
        _ => None,
    }
}

// 在 spider.rs 中定义返回结构
#[derive(Debug, Clone, serde::Serialize)]
pub struct AnchorProfile {
    pub name: String,
    pub avatar_url: String,
    /// 主播简介（API 可能缺失该字段，容错为 None）
    pub introduction: Option<String>,
}

/// 安全截断（M3 字节切片 panic 修复）：把字符串按字节截断到 ≤ max 字节，
/// 绝不落在 UTF-8 字符中间（直接 `&s[..n]` 在 n 落在多字节字符内会 panic）。
///
/// 用于错误信息里的响应片段展示——响应体来自远程 API，可能含中文/emoji 等
/// 多字节字符。实现：`char_indices` 只产生字符边界字节下标，取「起点 + 自身
/// 长度 ≤ max」的最后一个完整字符作为截断点，宁可少截也不拆字符；
/// max=0 或全为多字节时返回空串，绝不 panic。
fn truncate_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    s.char_indices()
        .take_while(|(i, c)| i + c.len_utf8() <= max)
        .map(|(i, c)| i + c.len_utf8())
        .last()
        .map(|end| &s[..end])
        .unwrap_or("")
}

impl MissevanClient {
    /// 记录一条网络请求（插桩调用点；status 0 = 请求/读取失败）
    fn record_request(
        &self,
        method: &str,
        url: &str,
        status: u16,
        start: Instant,
        anchor_id: Option<&str>,
        error: Option<String>,
    ) {
        network::record(NetworkLog::new(
            method,
            url,
            status,
            start.elapsed().as_millis() as u64,
            anchor_id,
            error,
        ));
    }

    /// 获取主播基本信息（名称 + 头像 URL）
    pub async fn get_anchor_profile(&self, room_id: &str) -> Result<AnchorProfile, AppError> {
        let start = Instant::now();
        let url = format!("https://fm.missevan.com/api/v2/live/{}", room_id);

        // 发送请求，并映射 reqwest::Error
        let resp = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                let err = AppError::network(format!("API 请求失败: {}", e))
                    .with_technical(format!("room_id: {}, error: {}", room_id, e))
                    .with_suggestion("请检查网络连接和代理设置")
                    .with_source("spider");
                self.record_request("GET", &url, 0, start, Some(room_id), Some(err.message.clone()));
                return Err(err);
            }
        };
        let status = resp.status();

        // 读取响应体，同样映射错误
        let body = match resp.text().await {
            Ok(b) => b,
            Err(e) => {
                let err = AppError::network(format!("读取响应失败: {}", e))
                    .with_technical(format!("room_id: {}", room_id))
                    .with_source("spider");
                self.record_request("GET", &url, 0, start, Some(room_id), Some(err.message.clone()));
                return Err(err);
            }
        };

        // 解析 JSON，映射 serde_json::Error
        let json: serde_json::Value = match serde_json::from_str(&body) {
            Ok(j) => j,
            Err(e) => {
                let err = AppError::network(format!("JSON 解析失败: {}", e))
                    .with_technical(format!(
                        "room_id: {}, 前200字符: {}",
                        room_id,
                        truncate_bytes(&body, 200)
                    ))
                    .with_source("spider");
                self.record_request("GET", &url, status.as_u16(), start, Some(room_id), Some(err.message.clone()));
                return Err(err);
            }
        };

        // 提取 creator 信息，若缺失则返回配置错误
        let parsed: Result<AnchorProfile, AppError> = (|| {
            let creator = &json["info"]["creator"];
            let name = creator["username"]
                .as_str()
                .ok_or_else(|| {
                    AppError::config("未找到主播名")
                        .with_technical(format!("room_id: {}", room_id))
                        .with_suggestion("请确认房间号是否正确")
                        .with_source("spider")
                })?
                .to_string();

            let avatar_url = creator["iconurl"]
                .as_str()
                .ok_or_else(|| {
                    AppError::config("未找到头像URL")
                        .with_technical(format!("room_id: {}", room_id))
                        .with_suggestion("请确认房间号是否正确")
                        .with_source("spider")
                })?
                .to_string();

            // 简介为可选字段：真实 API 返回 info.creator.introduction，缺失时返回 None
            let introduction = creator["introduction"].as_str().map(String::from);

            Ok(AnchorProfile {
                name,
                avatar_url,
                introduction,
            })
        })();

        self.record_request(
            "GET",
            &url,
            status.as_u16(),
            start,
            Some(room_id),
            parsed.as_ref().err().map(|e| e.message.clone()),
        );
        parsed
    }
}

/// 猫耳 FM API 客户端
pub struct MissevanClient {
    /// Arc 持有 reqwest::Client：clone 共享同一连接池/TLS 会话（G9 复用
    /// 的前提——共享实例的克隆零拷贝；测试可用 `Arc::ptr_eq` 验证池复用）
    client: Arc<reqwest::Client>,
}

impl Clone for MissevanClient {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
        }
    }
}

#[cfg(test)]
impl MissevanClient {
    /// 与另一实例是否共享同一 reqwest 连接池（G9 缓存命中判定）：
    /// 克隆/缓存命中的实例指向同一 `Arc<Client>`，重建的实例指向新 Arc。
    fn shares_pool_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.client, &other.client)
    }
}

// ── G9（02 审查）：命令层 HTTP client 复用 ──────────────────────────────
//
// 此前 anchor_cmds / monitor / lib.rs 每次调用 `from_config` 都新建
// reqwest Client——每个 Client 自带独立连接池/TLS 会话，命令层频繁调用
// 时每次都重新 TLS 握手，纯性能冗余（无资源泄漏）。修复：`from_config`
// 经「配置指纹缓存」返回共享实例——reqwest::Client 内部是 Arc（clone 为
// 引用计数 +1，零拷贝），配置未变化时所有调用方复用同一连接池。
//
// 指纹 = 影响客户端行为的字段子集（代理配置 + API 超时）。任一字段变化
//（设置页保存代理/超时）→ 指纹不匹配 → 重建并替换缓存；检测循环持有的
// 旧 client 不受影响（与既有行为一致：循环客户端只在启动时构建一次）。
// 指纹碰撞概率可忽略，最坏后果是一次请求用了旧代理配置，无安全问题。

/// 共享客户端缓存条目：指纹 + 最近一次构建的客户端
struct SharedClientEntry {
    fingerprint: u64,
    client: MissevanClient,
}

static SHARED_CLIENT_CACHE: std::sync::Mutex<Option<SharedClientEntry>> =
    std::sync::Mutex::new(None);

/// 计算影响客户端行为的配置指纹（代理字段 + API 超时；不含无关字段）。
/// proxy_password 只参与哈希，不在缓存条目中留存明文。
fn config_fingerprint(config: &GlobalConfig) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    config.proxy_type.hash(&mut h);
    config.proxy_addr.hash(&mut h);
    config.proxy_port.hash(&mut h);
    config.proxy_auth.hash(&mut h);
    config.proxy_username.hash(&mut h);
    config.proxy_password.hash(&mut h);
    config.api_timeout_secs.hash(&mut h);
    h.finish()
}

impl MissevanClient {
    /// 默认客户端（无代理，30s 超时）。生产路径统一走 `from_config`
    ///（全局代理 + api_timeout_secs），本构造仅供测试构造 client。
    #[cfg(test)]
    pub fn new() -> Result<Self, AppError> {
        Self::build_client(None, 30)
    }

    /// 按全局配置构建客户端（§11.1 网络分类接线）：
    /// - 全局代理（proxy_type: none | http | socks5 + proxy_addr/proxy_port，
    ///   proxy_auth 时经 reqwest basic_auth 附带账号密码）——检测循环 / 录制
    ///   monitor / 主播简介获取 / 头像请求共用 MissevanClient，统一生效；
    /// - API 请求超时（api_timeout_secs，≥1 秒兜底）；
    /// - 代理配置非法（地址空/端口 0/URL 构造失败）→ 降级直连并记 warn，
    ///   不阻断应用启动。
    ///
    /// G9 复用：配置指纹未变化时返回共享实例（连接池/TLS 会话复用），
    /// 避免命令层每次调用重建 Client；配置变化（代理/超时）时自动重建。
    pub fn from_config(config: &GlobalConfig) -> Result<Self, AppError> {
        let fingerprint = config_fingerprint(config);
        // 命中缓存（配置未变）→ 直接返回共享实例的克隆（reqwest::Client 内部
        // Arc，clone 零拷贝）。锁内不执行任何 IO/await，仅读引用，无阻塞风险。
        if let Ok(guard) = SHARED_CLIENT_CACHE.lock() {
            if let Some(entry) = guard.as_ref() {
                if entry.fingerprint == fingerprint {
                    return Ok(entry.client.clone());
                }
            }
        }
        let proxy = Self::proxy_from_config(config);
        let client = Self::build_client(proxy, config.api_timeout_secs.max(1) as u64)?;
        if let Ok(mut guard) = SHARED_CLIENT_CACHE.lock() {
            *guard = Some(SharedClientEntry {
                fingerprint,
                client: client.clone(),
            });
        }
        Ok(client)
    }

    /// 暴露底层 `reqwest::Client`（只读借用）——供一次性特殊请求复用共享
    /// 连接池：更新检查（update_cmds::check_update，GitHub API）等请求
    /// 在共享 client 上以**每请求** header/timeout 覆盖构建，不新建 Client。
    /// 注意：勿在此之上构造长超时流式下载（FFmpeg 下载走独立 600s 客户端，
    /// 见 wizard_cmds::download_ffmpeg——共享池的代理语义与超时不适用）。
    pub fn reqwest_client(&self) -> &reqwest::Client {
        &self.client
    }

    /// 从配置构造 reqwest 代理（纯逻辑，便于单测）：
    /// proxy_type=none / 地址空 / 端口 0 → None；http → `http://host:port`；
    /// socks5 → `socks5://host:port`（reqwest `socks` feature）；未知类型或
    /// URL 非法 → 记 warn 返回 None（降级直连）。
    fn proxy_from_config(config: &GlobalConfig) -> Option<reqwest::Proxy> {
        if config.proxy_type == "none"
            || config.proxy_addr.trim().is_empty()
            || config.proxy_port == 0
        {
            return None;
        }
        let url = match config.proxy_type.as_str() {
            "http" => format!("http://{}:{}", config.proxy_addr, config.proxy_port),
            "socks5" => format!("socks5://{}:{}", config.proxy_addr, config.proxy_port),
            other => {
                tracing::warn!("未知代理类型（降级为直连）: {}", other);
                return None;
            }
        };
        let proxy = reqwest::Proxy::all(&url);
        match proxy {
            Ok(p) => Some(if config.proxy_auth {
                p.basic_auth(&config.proxy_username, &config.proxy_password)
            } else {
                p
            }),
            Err(e) => {
                tracing::warn!("代理配置无效（降级为直连）: {}", e);
                None
            }
        }
    }

    /// 构建客户端：可选代理 + API 超时（秒）
    fn build_client(
        proxy: Option<reqwest::Proxy>,
        timeout_secs: u64,
    ) -> Result<Self, AppError> {
        let mut builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .connect_timeout(std::time::Duration::from_secs(10))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36");
        if let Some(proxy) = proxy {
            builder = builder.proxy(proxy);
        }
        let client = builder
            .build()
            .map_err(|e| AppError::internal(format!("创建 HTTP 客户端失败: {}", e)))?;

        Ok(Self {
            client: Arc::new(client),
        })
    }

    /// 检测主播直播状态
    ///
    /// 错误按 `CheckErrorKind` 分类（规格「直播状态异常修复」）：
    /// - 请求/读取失败 → `Network`（本地网络问题，不判离线）
    /// - HTTP 5XX / 429 → `Server`（服务器端问题，不判离线，调用方重试 + 退避）
    /// - 其他 4XX → `Other`（视为离线）
    /// - JSON 解析失败 → `Format`（API 格式变化，记 warn，判未知）
    pub async fn check_live(
        &self,
        room_id: &str,
        cookie: Option<&str>,
    ) -> Result<LiveCheckResult, CheckError> {
        let start = Instant::now();
        let url = format!("https://fm.missevan.com/api/v2/live/{}", room_id);

        let mut request = self
            .client
            .get(&url)
            .header(
                "Referer",
                format!("https://fm.missevan.com/live/{}", room_id),
            )
            .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8");

        if let Some(c) = cookie {
            request = request.header("Cookie", c);
        }

        let resp = match request.send().await {
            Ok(r) => r,
            Err(e) => {
                let err = AppError::network(format!("API 请求失败: {}", e))
                    .with_technical(format!("room_id: {}, error: {}", room_id, e))
                    .with_suggestion("请检查网络连接和代理设置")
                    .with_source("spider");
                self.record_request("GET", &url, 0, start, Some(room_id), Some(err.message.clone()));
                return Err(CheckError {
                    kind: CheckErrorKind::Network,
                    status: None,
                    error: err,
                });
            }
        };

        let status = resp.status();
        let body = match resp.text().await {
            Ok(b) => b,
            Err(e) => {
                let err = AppError::network(format!("读取响应失败: {}", e))
                    .with_technical(format!("room_id: {}", room_id))
                    .with_source("spider");
                self.record_request("GET", &url, 0, start, Some(room_id), Some(err.message.clone()));
                return Err(CheckError {
                    kind: CheckErrorKind::Network,
                    status: Some(status.as_u16()),
                    error: err,
                });
            }
        };

        if !status.is_success() {
            let err = AppError::network(format!("API 返回错误状态: {}", status))
                .with_technical(format!(
                    "HTTP {}: {} (前200字符)",
                    status,
                    truncate_bytes(&body, 200)
                ))
                .with_source("spider");
            let kind = classify_http_status(status.as_u16()).unwrap_or(CheckErrorKind::Other);
            self.record_request(
                "GET",
                &url,
                status.as_u16(),
                start,
                Some(room_id),
                Some(err.message.clone()),
            );
            return Err(CheckError {
                kind,
                status: Some(status.as_u16()),
                error: err,
            });
        }

        let parsed = self.parse_live_response(&body, room_id);
        self.record_request(
            "GET",
            &url,
            status.as_u16(),
            start,
            Some(room_id),
            parsed.as_ref().err().map(|e| e.message().to_string()),
        );
        parsed
    }

    /// 解析直播检测响应
    fn parse_live_response(
        &self,
        body: &str,
        room_id: &str,
    ) -> Result<LiveCheckResult, CheckError> {
        let json: serde_json::Value = serde_json::from_str(body).map_err(|e| CheckError {
            // 格式变化（如 API 改版）不代表主播离线：记 warn + 判定「未知」
            kind: CheckErrorKind::Format,
            status: None,
            error: AppError::network(format!("JSON 解析失败: {}", e))
                .with_technical(format!(
                    "room_id: {}, 前200字符: {}",
                    room_id,
                    truncate_bytes(&body, 200)
                ))
                .with_source("spider"),
        })?;

        let room = &json["info"]["room"];
        let status = &room["status"];

        let broadcasting = status["broadcasting"].as_bool().unwrap_or(false);
        let open = status["open"].as_i64().unwrap_or(0);
        // close_time 在真实 API 中是数值（epoch 毫秒）或 null，用 is_null() 判定直播是否已结束
        let close_time = &status["close_time"];
        let is_live = open == 1 && broadcasting && close_time.is_null();

        let creator = &json["info"]["creator"];

        Ok(LiveCheckResult {
            is_live,
            anchor_name: creator["username"].as_str().map(String::from),
            title: room["name"].as_str().map(String::from), // 直播间标题
            stream_url: room["channel"]["flv_pull_url"].as_str().map(String::from),
            avatar: creator["iconurl"].as_str().map(String::from),
        })
    }

    /// 从 URL 中提取 room_id（与前端 liveUrl 校验规则对齐）：
    /// 仅接受 https://fm.missevan.com/live/<数字>（可带尾斜杠、查询串或锚点），
    /// 路径只允许 [live, <数字>] 或 [live, <数字>, ""]（尾斜杠产生空尾段），
    /// 拒绝 /live/123/456 等前后端取段不一致的 URL。
    pub fn extract_room_id(url: &str) -> Option<String> {
        let parsed = url::Url::parse(url).ok()?;
        if parsed.host_str() != Some("fm.missevan.com") {
            return None;
        }
        let segments: Vec<&str> = parsed.path_segments()?.collect();
        let room_id = match segments.as_slice() {
            ["live", id] => *id,
            ["live", id, ""] => *id, // 尾斜杠
            _ => return None,
        };
        if room_id.is_empty() || !room_id.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Some(room_id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_room_id() {
        let url = "https://fm.missevan.com/live/100000002";
        assert_eq!(
            MissevanClient::extract_room_id(url),
            Some("100000002".to_string())
        );
    }

    #[test]
    fn test_extract_room_id_invalid() {
        assert_eq!(MissevanClient::extract_room_id("not-a-url"), None);
    }

    #[test]
    fn test_extract_room_id_trailing_slash() {
        // 尾斜杠与查询/锚点允许，取 /live/ 后唯一一段数字
        assert_eq!(
            MissevanClient::extract_room_id("https://fm.missevan.com/live/100000002/"),
            Some("100000002".to_string())
        );
        assert_eq!(
            MissevanClient::extract_room_id("https://fm.missevan.com/live/100000002?tab=1"),
            Some("100000002".to_string())
        );
        assert_eq!(
            MissevanClient::extract_room_id("https://fm.missevan.com/live/100000002#anchor"),
            Some("100000002".to_string())
        );
    }

    #[test]
    fn test_extract_room_id_multi_segment_rejected() {
        // /live/123/456：前端取 123、旧后端取 456 会静默改错主播，必须拒绝
        assert_eq!(
            MissevanClient::extract_room_id("https://fm.missevan.com/live/123/456"),
            None
        );
        assert_eq!(
            MissevanClient::extract_room_id("https://fm.missevan.com/live/123/456/"),
            None
        );
        // 双斜杠（/live//123）同样拒绝，前端正则不匹配
        assert_eq!(
            MissevanClient::extract_room_id("https://fm.missevan.com/live//123"),
            None
        );
    }

    #[test]
    fn test_extract_room_id_wrong_host_or_segment() {
        assert_eq!(
            MissevanClient::extract_room_id("https://other.com/live/123"),
            None
        );
        assert_eq!(
            MissevanClient::extract_room_id("https://fm.missevan.com/other/123"),
            None
        );
        assert_eq!(
            MissevanClient::extract_room_id("https://fm.missevan.com/live/abc"),
            None
        );
        assert_eq!(
            MissevanClient::extract_room_id("https://fm.missevan.com/live/"),
            None
        );
    }

    #[test]
    fn test_parse_live_response_offline() {
        let client = MissevanClient::new().unwrap();
        // 模拟未直播响应（真实 API 结构，close_time 为数值 epoch 毫秒）
        let body = r#"{
            "info": {
                "room": {
                    "name": "今晚的直播",
                    "status": {
                        "open": 0,
                        "broadcasting": false,
                        "close_time": 1784502379027
                    },
                    "channel": { "flv_pull_url": "" }
                },
                "creator": { "username": "测试主播", "iconurl": "" }
            }
        }"#;

        let result = client.parse_live_response(body, "123456").unwrap();
        assert!(!result.is_live);
        assert_eq!(result.anchor_name, Some("测试主播".to_string()));
        assert_eq!(result.title, Some("今晚的直播".to_string()));
    }

    #[test]
    fn test_parse_live_response_online() {
        let client = MissevanClient::new().unwrap();
        let body = r#"{
            "info": {
                "room": {
                    "name": "今晚唱歌",
                    "status": {
                        "open": 1,
                        "broadcasting": true,
                        "close_time": null
                    },
                    "channel": { "flv_pull_url": "https://stream.example.com/live.flv" }
                },
                "creator": {
                    "username": "测试主播",
                    "iconurl": "https://avatar.example.com/1.jpg"
                }
            }
        }"#;

        let result = client.parse_live_response(body, "100000002").unwrap();
        assert!(result.is_live);
        assert_eq!(result.anchor_name, Some("测试主播".to_string()));
        assert_eq!(result.title, Some("今晚唱歌".to_string()));
        assert_eq!(
            result.stream_url,
            Some("https://stream.example.com/live.flv".to_string())
        );
        assert_eq!(
            result.avatar,
            Some("https://avatar.example.com/1.jpg".to_string())
        );
    }

    #[test]
    fn close_time_numeric_value_means_not_live_ended() {
        let client = MissevanClient::new().unwrap();
        // close_time 是数值（epoch 毫秒）而非 null → 直播已结束
        let body = r#"{
            "info": {
                "room": {
                    "status": {
                        "open": 1,
                        "broadcasting": true,
                        "close_time": 1784502379027
                    }
                },
                "creator": { "username": "测试主播" }
            }
        }"#;

        let result = client.parse_live_response(body, "100000002").unwrap();
        assert!(!result.is_live);
    }

    #[test]
    fn close_time_null_means_live() {
        let client = MissevanClient::new().unwrap();
        // close_time 为 null → 直播进行中
        let body = r#"{
            "info": {
                "room": {
                    "status": {
                        "open": 1,
                        "broadcasting": true,
                        "close_time": null
                    }
                },
                "creator": { "username": "测试主播" }
            }
        }"#;

        let result = client.parse_live_response(body, "100000002").unwrap();
        assert!(result.is_live);
    }

    // ── 错误分类（规格：直播状态异常修复）──

    #[test]
    fn classify_429_and_5xx_as_server_errors() {
        // 429 限流 / 5XX 服务器错误：不反映主播状态，不判离线
        assert_eq!(
            classify_http_status(429),
            Some(CheckErrorKind::Server)
        );
        for code in [500u16, 502, 503, 504, 521, 599] {
            assert_eq!(classify_http_status(code), Some(CheckErrorKind::Server), "code={}", code);
        }
    }

    #[test]
    fn classify_other_4xx_as_other() {
        // 404 房间不存在 / 403 拒绝：视为离线（明确不可用）
        for code in [400u16, 401, 403, 404, 418, 451] {
            assert_eq!(classify_http_status(code), Some(CheckErrorKind::Other), "code={}", code);
        }
    }

    #[test]
    fn classify_success_as_none() {
        for code in [200u16, 201, 204, 301, 302] {
            assert_eq!(classify_http_status(code), None, "code={}", code);
        }
    }

    #[test]
    fn parse_format_error_is_format_kind_not_offline() {
        let client = MissevanClient::new().unwrap();
        // 非 JSON（如风控返回 HTML 验证页）→ Format：记 warn + 判「未知」，不判离线
        let err = client
            .parse_live_response("<html>captcha</html>", "123456")
            .unwrap_err();
        assert_eq!(err.kind, CheckErrorKind::Format);
        assert!(err.is_transient());
    }

    #[test]
    fn check_error_transient_classification() {
        let mk = |kind: CheckErrorKind| CheckError {
            kind,
            status: None,
            error: AppError::network("test"),
        };
        assert!(mk(CheckErrorKind::Server).is_transient());
        assert!(mk(CheckErrorKind::Network).is_transient());
        assert!(mk(CheckErrorKind::Format).is_transient());
        // Other（4XX 明确不可用）不算瞬时错误：可据此判离线
        assert!(!mk(CheckErrorKind::Other).is_transient());
    }

    // ── 全局代理接线（from_config / proxy_from_config）──

    fn proxy_config(proxy_type: &str) -> GlobalConfig {
        let mut c = GlobalConfig::default();
        c.proxy_type = proxy_type.to_string();
        c.proxy_addr = "127.0.0.1".to_string();
        c.proxy_port = 8080;
        c
    }

    #[test]
    fn proxy_none_or_empty_config_yields_no_proxy() {
        // 默认配置（proxy_type=none）→ 无代理
        assert!(MissevanClient::proxy_from_config(&GlobalConfig::default()).is_none());
        // 类型非 none 但地址空 / 端口 0 → 视为未配置
        let mut c = proxy_config("http");
        c.proxy_addr = "  ".to_string();
        assert!(MissevanClient::proxy_from_config(&c).is_none());
        let mut c = proxy_config("http");
        c.proxy_port = 0;
        assert!(MissevanClient::proxy_from_config(&c).is_none());
    }

    #[test]
    fn proxy_http_and_socks5_build_successfully() {
        assert!(MissevanClient::proxy_from_config(&proxy_config("http")).is_some());
        assert!(MissevanClient::proxy_from_config(&proxy_config("socks5")).is_some());
        // 认证开启时带账号密码（basic_auth）同样可构建
        let mut c = proxy_config("http");
        c.proxy_auth = true;
        c.proxy_username = "user".to_string();
        c.proxy_password = "pass".to_string();
        assert!(MissevanClient::proxy_from_config(&c).is_some());
    }

    #[test]
    fn proxy_unknown_type_falls_back_to_none() {
        let c = proxy_config("ftp");
        assert!(MissevanClient::proxy_from_config(&c).is_none());
    }

    #[test]
    fn from_config_applies_proxy_and_api_timeout() {
        // 触碰共享缓存 → 持串行锁（与其他 from_config 测试互斥）
        let _cache_guard = cache_test_guard();
        // 带代理：客户端构建成功（连接不会发出，仅验证构造路径）
        let c = proxy_config("http");
        assert!(MissevanClient::from_config(&c).is_ok());

        // 代理非法（地址非 IP/域名的畸形串会构造失败）→ 降级直连，不报错
        let mut c = proxy_config("http");
        c.proxy_addr = "http://bad url with spaces".to_string();
        assert!(
            MissevanClient::from_config(&c).is_ok(),
            "代理配置非法应降级直连而非报错"
        );

        // api_timeout_secs=0 → 兜底 1s（不 panic、不创建 0 超时）
        let mut c = GlobalConfig::default();
        c.api_timeout_secs = 0;
        assert!(MissevanClient::from_config(&c).is_ok());
    }

    // ── G9（02 审查）：命令层 HTTP client 复用（配置指纹缓存）──

    /// 缓存相关测试的全局串行锁：cargo test 默认多线程并发执行，共享静态
    /// `SHARED_CLIENT_CACHE` 会被其他 from_config 测试并发覆盖——所有
    /// 触碰共享缓存的测试必须持此锁串行执行，断言才稳定。
    static CACHE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn cache_test_guard() -> std::sync::MutexGuard<'static, ()> {
        CACHE_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// 清空共享缓存，保证测试相互独立（缓存为进程级静态量）
    fn reset_shared_client_cache() {
        *SHARED_CLIENT_CACHE.lock().unwrap() = None;
    }

    #[test]
    fn config_fingerprint_reflects_only_network_fields() {
        // 影响客户端行为的字段（代理 + 超时）变化 → 指纹变化
        let a = GlobalConfig::default();
        let base = config_fingerprint(&a);
        let mut b = GlobalConfig::default();
        b.proxy_type = "http".to_string();
        b.proxy_addr = "127.0.0.1".to_string();
        b.proxy_port = 8080;
        assert_ne!(base, config_fingerprint(&b), "代理配置变化必须改变指纹");
        let mut c = GlobalConfig::default();
        c.api_timeout_secs = 60;
        assert_ne!(base, config_fingerprint(&c), "超时变化必须改变指纹");
        // 无关字段（输出目录等）变化 → 指纹不变（不触发客户端重建）
        let mut d = GlobalConfig::default();
        d.output_dir = "/elsewhere".to_string();
        d.check_interval_secs = 30;
        assert_eq!(base, config_fingerprint(&d), "无关字段不得改变指纹");
    }

    #[test]
    fn from_config_reuses_shared_client_when_config_unchanged() {
        let _cache_guard = cache_test_guard();
        reset_shared_client_cache();
        let cfg = GlobalConfig::default();
        let c1 = MissevanClient::from_config(&cfg).unwrap();
        // 首次构建后缓存已填充且指纹一致
        {
            let guard = SHARED_CLIENT_CACHE.lock().unwrap();
            let entry = guard.as_ref().expect("首次构建必须写入缓存");
            assert_eq!(entry.fingerprint, config_fingerprint(&cfg));
        }
        // 再次调用（配置未变）→ 缓存命中，与 c1 共享同一连接池
        let c2 = MissevanClient::from_config(&cfg).unwrap();
        assert!(
            c1.shares_pool_with(&c2),
            "配置未变必须复用共享客户端（同一连接池）"
        );
        // 克隆同样共享（reqwest::Client 内部 Arc 语义）
        assert!(c1.shares_pool_with(&c1.clone()));
        reset_shared_client_cache();
    }

    #[test]
    fn from_config_rebuilds_client_when_network_config_changes() {
        let _cache_guard = cache_test_guard();
        reset_shared_client_cache();
        let mut cfg = GlobalConfig::default();
        let c1 = MissevanClient::from_config(&cfg).unwrap();
        // 代理变化 → 指纹不匹配 → 重建新客户端
        cfg.proxy_type = "http".to_string();
        cfg.proxy_addr = "127.0.0.1".to_string();
        cfg.proxy_port = 8080;
        let c2 = MissevanClient::from_config(&cfg).unwrap();
        assert!(
            !c1.shares_pool_with(&c2),
            "代理配置变化必须重建客户端（新连接池）"
        );
        // 改回默认配置：缓存条目已被代理配置替换 → 再次重建（与最早的 c1
        // 不同池——旧默认客户端已随缓存条目替换而释放）
        let c3 = MissevanClient::from_config(&GlobalConfig::default()).unwrap();
        assert!(
            !c1.shares_pool_with(&c3),
            "缓存被替换后重建（与最早的 c1 不同池）"
        );
        // 配置稳定后连续调用 → 命中缓存，与 c3 共享同一池
        let c4 = MissevanClient::from_config(&GlobalConfig::default()).unwrap();
        assert!(
            c3.shares_pool_with(&c4),
            "配置稳定后应命中共享缓存（同一连接池）"
        );
        // 缓存条目始终是最后一次构建的配置
        {
            let guard = SHARED_CLIENT_CACHE.lock().unwrap();
            let entry = guard.as_ref().unwrap();
            assert_eq!(entry.fingerprint, config_fingerprint(&GlobalConfig::default()));
        }
        reset_shared_client_cache();
    }

    // ── M3：UTF-8 安全截断（字节切片 panic 修复）──

    #[test]
    fn truncate_bytes_never_panics_on_utf8_boundaries() {
        // 中文/emoji 混合内容：对所有截断点（0..=len 逐字节）都不 panic，
        // 结果必须是原串前缀且长度 ≤ 上限
        let s = "🎙️主播A正在直播中🎉🔥测试123";
        for n in 0..=s.len() {
            let t = truncate_bytes(s, n);
            assert!(t.len() <= n, "n={} 截断超限 len={}", n, t.len());
            assert!(s.starts_with(t), "截断结果必须是原串前缀: n={}", n);
        }
        // 具体边界：200 字节恰好是 4 字节 emoji 的整数倍 → 完整截断
        let emoji = "😀".repeat(100); // 400 字节
        let t = truncate_bytes(&emoji, 200);
        assert_eq!(t.len(), 200);
        // 截断点落在多字节字符中间：中文(6B) + 50×emoji(200B) = 206B，
        // 200 落进第 49 个 emoji 内部 → 截到第 48 个 emoji 末尾（6+192=198B）
        let mixed = format!("中文{}", "😀".repeat(50));
        let t2 = truncate_bytes(&mixed, 200);
        assert_eq!(t2, format!("中文{}", "😀".repeat(48)));
        // 极短上限不 panic，返回空串
        assert_eq!(truncate_bytes(&mixed, 1), "");
        assert_eq!(truncate_bytes(&mixed, 0), "");
    }

    #[test]
    fn truncate_bytes_keeps_short_strings_intact() {
        // 不超过上限时原样返回（含中文短串）
        assert_eq!(truncate_bytes("abc中文", 200), "abc中文");
        assert_eq!(truncate_bytes("", 5), "");
        // 边界恰好是完整字符时整段保留
        assert_eq!(truncate_bytes("a中", 4), "a中");
    }

    #[test]
    fn truncate_bytes_no_replacement_char() {
        // 截断结果不得产生 U+FFFD 替换符（说明按字节硬切过）
        let s = "😀😀😀中文";
        let t = truncate_bytes(s, 5);
        assert!(!t.contains('�'));
        assert!(t.is_char_boundary(t.len()));
    }
}
