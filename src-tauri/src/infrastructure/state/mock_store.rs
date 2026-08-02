use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

/// 模拟直播数据条目（前端调试面板可编辑）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockLiveData {
    pub room_id: String,
    pub name: String,
    pub is_live: bool,
    pub stream_url: String,
    pub local_file: Option<String>,
}

/// Mock 数据源：内存中的模拟主播表 + 模式开关
///
/// - `entries`: room_id -> MockLiveData
/// - `mock_mode`: 是否启用模拟检测（开启后 DetectionLoop 不再发真实请求）
#[derive(Debug, Default)]
pub struct MockStore {
    entries: Arc<RwLock<HashMap<String, MockLiveData>>>,
    mock_mode: AtomicBool,
}

impl MockStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前是否处于 mock 模式
    pub fn is_mock_mode(&self) -> bool {
        self.mock_mode.load(Ordering::Relaxed)
    }

    /// 开启 / 关闭 mock 模式
    pub fn set_mode(&self, enable: bool) {
        self.mock_mode.store(enable, Ordering::Relaxed);
    }

    /// 按 room_id 查询模拟条目
    pub fn get(&self, room_id: &str) -> Option<MockLiveData> {
        self.entries
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(room_id)
            .cloned()
    }

    /// 新增 / 覆盖模拟条目（以 room_id 为键）
    pub fn upsert(&self, data: MockLiveData) {
        self.entries
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(data.room_id.clone(), data);
    }

    /// 删除指定模拟条目
    pub fn remove(&self, room_id: &str) -> Option<MockLiveData> {
        self.entries
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(room_id)
    }

    /// 批量设置所有条目的直播状态
    pub fn set_all_live(&self, live: bool) {
        for entry in self
            .entries
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values_mut()
        {
            entry.is_live = live;
        }
    }

    /// 清空所有模拟条目
    pub fn reset(&self) {
        self.entries
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }

    /// 列出所有模拟条目
    pub fn list(&self) -> Vec<MockLiveData> {
        self.entries
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(room_id: &str, name: &str, is_live: bool) -> MockLiveData {
        MockLiveData {
            room_id: room_id.to_string(),
            name: name.to_string(),
            is_live,
            stream_url: format!("mock://stream/{}", room_id),
            local_file: None,
        }
    }

    #[test]
    fn set_mode_controls_mock_flag() {
        let store = MockStore::new();
        assert!(!store.is_mock_mode());
        store.set_mode(true);
        assert!(store.is_mock_mode());
        store.set_mode(false);
        assert!(!store.is_mock_mode());
    }

    #[test]
    fn upsert_then_list_contains_entry_and_remove_removes() {
        let store = MockStore::new();
        store.upsert(sample("1001", "主播A", true));
        store.upsert(sample("1002", "主播B", false));

        assert_eq!(store.list().len(), 2);
        assert!(store.get("1001").is_some());
        assert_eq!(store.get("1001").unwrap().name, "主播A");
        assert!(store.get("1001").unwrap().is_live);

        store.remove("1001");
        assert!(store.get("1001").is_none());
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn upsert_same_room_id_overwrites_entry() {
        let store = MockStore::new();
        store.upsert(sample("1001", "主播A", false));
        store.upsert(sample("1001", "主播A改名", true));

        let list = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "主播A改名");
        assert!(list[0].is_live);
    }

    #[test]
    fn set_all_live_updates_every_entry() {
        let store = MockStore::new();
        store.upsert(sample("1001", "A", false));
        store.upsert(sample("1002", "B", false));
        store.upsert(sample("1003", "C", false));

        store.set_all_live(true);
        for entry in store.list() {
            assert!(entry.is_live);
        }

        store.set_all_live(false);
        for entry in store.list() {
            assert!(!entry.is_live);
        }
    }

    #[test]
    fn reset_clears_all_entries() {
        let store = MockStore::new();
        store.upsert(sample("1001", "A", true));
        store.upsert(sample("1002", "B", true));

        store.reset();
        assert!(store.list().is_empty());
        assert!(store.get("1001").is_none());
        assert!(store.get("1002").is_none());
    }
}
