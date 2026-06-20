use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;
use url::Url;

/// 直播信息（包含头像）
#[derive(Debug, Clone, Serialize)]
pub struct LiveInfo {
    pub anchor_name: String,
    pub is_live: bool,
    pub title: Option<String>,
    pub stream_url: Option<String>,
    pub avatar: Option<String>,
}

/// 获取原始 JSON（不做解析，供 commands 使用）
pub async fn get_live_info_raw(
    url: &str,
    proxy: Option<&str>,
    cookie: Option<&str>,
) -> Result<Value> {
    let room_id = extract_room_id(url)?;
    let api_url = format!("https://fm.missevan.com/api/v2/live/{}", room_id);

    let mut client_builder = Client::builder();
    if let Some(proxy_str) = proxy {
        let proxy = reqwest::Proxy::all(proxy_str)?;
        client_builder = client_builder.proxy(proxy);
    }
    let client = client_builder.build()?;

    let mut request = client
        .get(&api_url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:124.0) Gecko/20100101 Firefox/124.0",
        )
        .header("Accept", "application/json, text/plain, */*")
        .header(
            "Accept-Language",
            "zh-CN,zh;q=0.9,en;q=0.8,en-GB;q=0.7,en-US;q=0.6",
        )
        .header(
            "Referer",
            format!("https://fm.missevan.com/live/{}", room_id),
        );

    if let Some(cookie_str) = cookie {
        request = request.header("Cookie", cookie_str);
    }

    let response = request.send().await?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        bail!("HTTP {}: {}", status, text);
    }

    let json: Value = response.json().await?;
    Ok(json)
}

/// 从原始 JSON 中提取头像 URL（优先 iconurl，其次 avatar）
pub fn extract_avatar_from_json(json: &Value) -> Option<String> {
    json["info"]["creator"]["iconurl"]
        .as_str()
        .or_else(|| json["info"]["creator"]["avatar"].as_str())
        .map(|s| s.to_string())
}

/// 获取结构化的直播信息（包含主播名、状态、标题、流地址、头像）
pub async fn get_live_info(
    url: &str,
    proxy: Option<&str>,
    cookie: Option<&str>,
) -> Result<LiveInfo> {
    let json = get_live_info_raw(url, proxy, cookie).await?;

    let anchor_name = match json["info"]["creator"]["username"].as_str() {
        Some(name) => name.to_string(),
        None => {
            let snippet = serde_json::to_string_pretty(&json).unwrap_or_default();
            let limited = if snippet.len() > 500 {
                &snippet[..500]
            } else {
                &snippet
            };
            bail!("无法获取主播名，JSON 结构可能已变更。片段：\n{}", limited);
        }
    };

    let is_live = json["info"]["room"]["status"]["broadcasting"]
        .as_bool()
        .unwrap_or(false);
    let avatar = extract_avatar_from_json(&json);

    if !is_live {
        return Ok(LiveInfo {
            anchor_name,
            is_live: false,
            title: None,
            stream_url: None,
            avatar,
        });
    }

    let title = json["info"]["room"]["name"].as_str().map(|s| s.to_string());
    let stream_url = json["info"]["room"]["channel"]["flv_pull_url"]
        .as_str()
        .map(|s| s.to_string());

    Ok(LiveInfo {
        anchor_name,
        is_live: true,
        title,
        stream_url,
        avatar,
    })
}

/// 从 URL 中提取房间 ID
fn extract_room_id(url: &str) -> Result<String> {
    let parsed = Url::parse(url)?;
    let path = parsed.path();
    let id = path
        .trim_end_matches('/')
        .split('/')
        .last()
        .context("URL 格式错误，无法提取房间 ID")?;
    Ok(id.to_string())
}
