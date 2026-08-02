use std::collections::VecDeque;
use crate::infrastructure::notification::types::Notification;

/// 固定容量环形缓冲区，存储最近的 N 条通知
pub struct RingBuffer {
    buffer: VecDeque<Notification>,
    capacity: usize,
}

impl RingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, notification: Notification) {
        if self.buffer.len() >= self.capacity {
            self.buffer.pop_front();
        }
        self.buffer.push_back(notification);
    }

    pub fn all(&self) -> Vec<Notification> {
        self.buffer.iter().rev().cloned().collect()
    }

    pub fn filter_by_level(&self, level: crate::infrastructure::notification::types::NotificationLevel) -> Vec<Notification> {
        self.buffer
            .iter()
            .rev()
            .filter(|n| n.level == level)
            .cloned()
            .collect()
    }

    pub fn filter_by_source(&self, source: &str) -> Vec<Notification> {
        self.buffer
            .iter()
            .rev()
            .filter(|n| n.source == source)
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::notification::types::{NotificationLevel, Notification};

    #[test]
    fn test_push_and_all() {
        let mut buf = RingBuffer::new(3);
        assert!(buf.is_empty());

        buf.push(Notification::new("TST_001", NotificationLevel::Info, "test", "msg 1"));
        buf.push(Notification::new("TST_002", NotificationLevel::Warning, "test", "msg 2"));

        assert_eq!(buf.len(), 2);
        assert_eq!(buf.all().len(), 2);
    }

    #[test]
    fn test_capacity_limit() {
        let mut buf = RingBuffer::new(2);
        buf.push(Notification::new("TST_001", NotificationLevel::Info, "test", "msg 1"));
        buf.push(Notification::new("TST_002", NotificationLevel::Info, "test", "msg 2"));
        buf.push(Notification::new("TST_003", NotificationLevel::Info, "test", "msg 3"));

        assert_eq!(buf.len(), 2);
        assert_eq!(buf.all().first().unwrap().code, "TST_003");
    }

    #[test]
    fn test_filter_by_level() {
        let mut buf = RingBuffer::new(10);
        buf.push(Notification::new("A", NotificationLevel::Info, "t", "m"));
        buf.push(Notification::new("B", NotificationLevel::Error, "t", "m"));
        buf.push(Notification::new("C", NotificationLevel::Info, "t", "m"));

        let errors = buf.filter_by_level(NotificationLevel::Error);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, "B");
    }
}
