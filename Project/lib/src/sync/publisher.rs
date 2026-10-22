//! 状态发布者 - 发布状态变更事件

use super::channel::StateChannel;
use std::sync::Arc;

/// 状态变更事件
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateEvent {
    pub agent_id: String,
    pub key: String,
    pub value: String,
    pub version: u64,
}

/// 状态发布者
pub struct StatePublisher {
    agent_id: String,
    channel: Arc<StateChannel>,
    version: u64,
}

impl StatePublisher {
    pub fn new(agent_id: &str, channel: Arc<StateChannel>) -> Self {
        Self { agent_id: agent_id.to_string(), channel, version: 0 }
    }

    /// 发布状态变更
    pub fn publish(&mut self, key: &str, value: &str) -> u64 {
        self.version += 1;
        let event = StateEvent {
            agent_id: self.agent_id.clone(),
            key: key.to_string(),
            value: value.to_string(),
            version: self.version,
        };
        self.channel.send(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_publish_creates_event() {
        let ch = Arc::new(StateChannel::new(16));
        let mut pub1 = StatePublisher::new("agent1", ch.clone());

        let version = pub1.publish("status", "running");
        assert_eq!(version, 1);

        let version = pub1.publish("status", "stopped");
        assert_eq!(version, 2);
    }
}
