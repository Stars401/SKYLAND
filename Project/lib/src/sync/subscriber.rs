//! 状态订阅者 - 接收状态变更事件

use super::channel::StateChannel;
use super::publisher::StateEvent;
use std::sync::Arc;

/// 状态变更处理器
pub type StateHandler = Box<dyn Fn(&StateEvent) + Send + 'static>;

/// 状态订阅者
pub struct StateSubscriber {
    channel: Arc<StateChannel>,
    handlers: Vec<StateHandler>,
}

impl StateSubscriber {
    pub fn new(channel: Arc<StateChannel>) -> Self {
        Self { channel, handlers: Vec::new() }
    }

    /// 注册事件处理器
    pub fn on_event<F>(&mut self, handler: F)
    where
        F: Fn(&StateEvent) + Send + 'static,
    {
        self.handlers.push(Box::new(handler));
    }

    /// 同步等待并处理一个事件
    pub fn wait_next(&self) -> Option<StateEvent> {
        let event = self.channel.recv()?;
        for handler in &self.handlers {
            handler(&event);
        }
        Some(event)
    }

    /// 非阻塞尝试获取事件
    pub fn try_recv(&self) -> Option<StateEvent> {
        let event = self.channel.try_recv()?;
        for handler in &self.handlers {
            handler(&event);
        }
        Some(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::publisher::StatePublisher;

    #[allow(unused_mut)]
    #[test]
    fn test_subscriber_receives_events() {
        let ch = Arc::new(StateChannel::new(16));
        let mut pub1 = StatePublisher::new("agent1", ch.clone());
        let mut sub = StateSubscriber::new(ch);

        pub1.publish("key", "val1");

        let event = sub.try_recv().unwrap();
        assert_eq!(event.agent_id, "agent1");
        assert_eq!(event.value, "val1");
    }

    #[test]
    fn test_subscriber_handler_called() {
        use std::sync::Mutex;
        let ch = Arc::new(StateChannel::new(16));
        let mut pub1 = StatePublisher::new("agent1", ch.clone());
        let mut sub = StateSubscriber::new(ch);

        let received = Arc::new(Mutex::new(Vec::new()));
        let r = received.clone();
        sub.on_event(move |e| r.lock().unwrap().push(e.value.clone()));

        pub1.publish("k", "v1");
        pub1.publish("k", "v2");

        sub.try_recv();
        assert_eq!(received.lock().unwrap().len(), 1);
    }
}
