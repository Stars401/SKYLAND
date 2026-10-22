//! 状态同步通道 - 基于mpsc的线程安全事件传递

use super::publisher::StateEvent;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Mutex;

/// 状态同步通道
pub struct StateChannel {
    tx: SyncSender<StateEvent>,
    rx: Mutex<Receiver<StateEvent>>,
}

impl StateChannel {
    /// 创建新的状态通道
    /// buffer_size: 缓冲区大小
    pub fn new(buffer_size: usize) -> Self {
        let (tx, rx) = mpsc::sync_channel(buffer_size.max(1));
        Self { tx, rx: Mutex::new(rx) }
    }

    /// 发送事件（阻塞直到有空间）
    pub fn send(&self, event: StateEvent) -> u64 {
        let version = event.version;
        self.tx.send(event).ok();
        version
    }

    /// 同步接收事件（阻塞）
    pub fn recv(&self) -> Option<StateEvent> {
        self.rx.lock().ok()?.recv().ok()
    }

    /// 非阻塞接收事件
    pub fn try_recv(&self) -> Option<StateEvent> {
        self.rx.lock().ok()?.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_send_recv() {
        let ch = StateChannel::new(10);
        ch.send(StateEvent { agent_id: "a".into(), key: "k".into(), value: "v".into(), version: 1 });
        let e = ch.recv().unwrap();
        assert_eq!(e.agent_id, "a");
        assert_eq!(e.value, "v");
    }

    #[test]
    fn test_channel_try_recv_empty() {
        let ch = StateChannel::new(10);
        assert!(ch.try_recv().is_none());
    }

    #[test]
    fn test_channel_buffer_full() {
        let ch = StateChannel::new(2);
        ch.send(StateEvent { agent_id: "a".into(), key: "1".into(), value: "1".into(), version: 1 });
        ch.send(StateEvent { agent_id: "a".into(), key: "2".into(), value: "2".into(), version: 2 });
        // Buffer has room for 2, both should succeed
        assert!(ch.try_recv().is_some());
        assert!(ch.try_recv().is_some());
    }
}
