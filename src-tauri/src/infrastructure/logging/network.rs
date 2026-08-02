//! 网络请求插桩环形缓冲（Task 15）
//!
//! spider 的 API 请求在调用点记录（URL / 方法 / 状态码 / 耗时 / 主播 room_id），
//! 写入容量 500 的环形缓冲供调试页「网络请求」模块查询。
//!
//! ## 插桩方案说明
//!
//! 不引入 reqwest middleware（保持依赖面与复杂度可控），而是在
//! `MissevanClient::check_live` / `get_anchor_profile` 调用点记录。
//! `MissevanClient` 在多处创建（detector / anchor_cmds / 录制 monitor），
//! 因此 store 采用进程级全局单例（`global_store`）：tauri 托管状态与
//! spider 插桩共用同一实例，无需把 store 显式传遍所有调用点。
//!
//! 写入前对 URL 与错误信息做脱敏（Cookie / Authorization / Password 值 → `***`）。

use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, OnceLock, RwLock};

use super::buffer::sanitize_message;

/// 环形缓冲容量（设计文档 §10：500 条）
const NETWORK_LOG_CAPACITY: usize = 500;

/// 单条网络请求记录
#[derive(Debug, Clone, Serialize)]
pub struct NetworkLog {
    /// RFC3339 时间戳
    pub timestamp: String,
    /// HTTP 方法（GET / POST ...）
    pub method: String,
    /// 请求 URL（已脱敏）
    pub url: String,
    /// HTTP 状态码；0 = 请求失败（网络错误 / 响应读取失败）
    pub status: u16,
    /// 请求耗时（毫秒）
    pub duration_ms: u64,
    /// 关联主播 room_id（非主播请求为 None）
    pub anchor_id: Option<String>,
    /// 失败原因（已脱敏）；成功为 None
    pub error: Option<String>,
}

impl NetworkLog {
    /// 构造一条记录（timestamp 自动填充；URL 与 error 写入前脱敏）
    pub fn new(
        method: &str,
        url: &str,
        status: u16,
        duration_ms: u64,
        anchor_id: Option<&str>,
        error: Option<String>,
    ) -> Self {
        Self {
            timestamp: chrono::Utc::now().to_rfc3339(),
            method: method.to_string(),
            url: sanitize_message(url),
            status,
            duration_ms,
            anchor_id: anchor_id.map(String::from),
            error: error.map(|e| sanitize_message(&e)),
        }
    }
}

/// 网络日志环形缓冲（容量 500，超限丢最旧）
pub struct NetworkLogStore {
    inner: Arc<RwLock<VecDeque<NetworkLog>>>,
    capacity: usize,
}

impl NetworkLogStore {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(VecDeque::with_capacity(capacity))),
            capacity,
        }
    }

    /// 追加一条记录（超限丢最旧）
    pub fn record(&self, entry: NetworkLog) {
        let mut buf = self.inner.write().unwrap_or_else(|p| p.into_inner());
        if buf.len() >= self.capacity {
            buf.pop_front();
        }
        buf.push_back(entry);
    }

    /// 全部记录（最新在前）
    pub fn all(&self) -> Vec<NetworkLog> {
        self.inner
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .rev()
            .cloned()
            .collect()
    }

    pub fn clear(&self) {
        self.inner
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }

    pub fn len(&self) -> usize {
        self.inner
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// 进程级全局 store（tauri 托管状态与 spider 插桩共用同一实例）
pub fn global_store() -> Arc<NetworkLogStore> {
    static STORE: OnceLock<Arc<NetworkLogStore>> = OnceLock::new();
    STORE
        .get_or_init(|| Arc::new(NetworkLogStore::new(NETWORK_LOG_CAPACITY)))
        .clone()
}

/// 记录一条网络请求（调用点入口；对 URL / error 做脱敏后写入）
pub fn record(entry: NetworkLog) {
    let entry = NetworkLog {
        url: sanitize_message(&entry.url),
        error: entry.error.map(|e| sanitize_message(&e)),
        ..entry
    };
    global_store().record(entry);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(url: &str, status: u16) -> NetworkLog {
        NetworkLog::new("GET", url, status, 12, Some("100000002"), None)
    }

    #[test]
    fn store_capacity_drops_oldest() {
        let store = NetworkLogStore::new(3);
        store.record(sample("https://a/1", 200));
        store.record(sample("https://a/2", 200));
        store.record(sample("https://a/3", 200));
        store.record(sample("https://a/4", 500));
        let all = store.all();
        assert_eq!(all.len(), 3);
        assert!(all[0].url.ends_with("/4"));
        assert!(all[2].url.ends_with("/2"));
        assert_eq!(all[0].status, 500);
    }

    #[test]
    fn store_newest_first_and_clear() {
        let store = NetworkLogStore::new(10);
        store.record(sample("https://a/1", 200));
        store.record(sample("https://a/2", 404));
        let all = store.all();
        assert_eq!(all.len(), 2);
        assert!(all[0].url.ends_with("/2"));
        assert_eq!(all[0].anchor_id.as_deref(), Some("100000002"));
        assert_eq!(all[0].duration_ms, 12);

        store.clear();
        assert!(store.is_empty());
    }

    #[test]
    fn record_sanitizes_url_and_error() {
        let store = NetworkLogStore::new(10);
        store.record(NetworkLog::new(
            "GET",
            "https://fm.missevan.com/api/v2/live/123?token=secret&password=abc",
            200,
            5,
            Some("123"),
            None,
        ));
        store.record(NetworkLog::new(
            "GET",
            "https://fm.missevan.com/api/v2/live/123",
            0,
            5,
            Some("123"),
            Some("请求失败, Cookie: a1b2c3".to_string()),
        ));
        let all = store.all();
        assert!(all[1].url.contains("password=***"));
        assert!(all[0].error.as_deref().unwrap().contains("Cookie: ***"));
    }

    #[test]
    fn global_store_records_and_clears() {
        // 唯一使用全局 store 的测试（其他测试用独立实例，避免并行互扰）
        let store = global_store();
        store.clear();
        record(sample("https://fm.missevan.com/api/v2/live/1001", 200));
        record(sample("https://fm.missevan.com/api/v2/live/1002", 404));
        let all = store.all();
        assert_eq!(all.len(), 2);
        assert!(all[0].url.contains("1002"));
        store.clear();
        assert!(store.is_empty());
    }
}
