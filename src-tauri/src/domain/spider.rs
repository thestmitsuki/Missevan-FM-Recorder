use crate::infrastructure::error::types::AppError;
use crate::infrastructure::logging::network::{self, NetworkLog};
use serde::{Deserialize, Serialize};
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
                        &body[..body.len().min(200)]
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
    client: reqwest::Client,
}

impl Clone for MissevanClient {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
        }
    }
}

impl MissevanClient {
    pub fn new() -> Result<Self, AppError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .build()
            .map_err(|e| AppError::internal(format!("创建 HTTP 客户端失败: {}", e)))?;

        Ok(Self { client })
    }

    /// 带代理的客户端
    ///
    /// 预留公共 API：代理设置（GlobalConfig.proxy_*）尚未接线到检测/录制请求，
    /// 设置页相应字段标注「暂未生效」；接线后由这里按 proxy_type/addr/port 构建。
    #[allow(dead_code)]
    pub fn with_proxy(proxy_url: &str) -> Result<Self, AppError> {
        let proxy = reqwest::Proxy::all(proxy_url)
            .map_err(|e| AppError::config(format!("代理配置无效: {}", e)))?;

        let client = reqwest::Client::builder()
            .proxy(proxy)
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .build()
            .map_err(|e| AppError::internal(format!("创建 HTTP 客户端失败: {}", e)))?;

        Ok(Self { client })
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
                    &body[..body.len().min(200)]
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
                    &body[..body.len().min(200)]
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
}
