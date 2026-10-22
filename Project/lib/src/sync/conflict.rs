//! 冲突解决器 - 处理状态冲突

use super::publisher::StateEvent;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictStrategy {
    LastWriteWins,
    FirstWriteWins,
    PreferAgent,
}

pub struct ConflictResolver {
    strategy: ConflictStrategy,
    preferred_agent: Option<String>,
    state: HashMap<String, StateEvent>,
}

impl ConflictResolver {
    pub fn new(strategy: ConflictStrategy) -> Self {
        Self { strategy, preferred_agent: None, state: HashMap::new() }
    }

    pub fn with_preferred(mut self, agent: &str) -> Self {
        self.preferred_agent = Some(agent.to_string());
        self
    }

    pub fn resolve(&mut self, event: StateEvent) -> Option<StateEvent> {
        let key = event.key.clone();

        if let Some(existing) = self.state.get(&key) {
            let same_agent = self.preferred_agent.as_ref()
                .map_or(false, |pref| event.agent_id == *pref);

            let takes_effect = match self.strategy {
                ConflictStrategy::LastWriteWins => event.version > existing.version,
                ConflictStrategy::FirstWriteWins => event.version < existing.version,
                ConflictStrategy::PreferAgent => same_agent,
            };

            if takes_effect {
                self.state.insert(key, event.clone());
                Some(event)
            } else {
                None
            }
        } else {
            self.state.insert(key, event.clone());
            Some(event)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(agent: &str, key: &str, val: &str, ver: u64) -> StateEvent {
        StateEvent { agent_id: agent.into(), key: key.into(), value: val.into(), version: ver }
    }

    #[test]
    fn test_last_write_wins() {
        let mut r = ConflictResolver::new(ConflictStrategy::LastWriteWins);
        assert!(r.resolve(ev("a", "s", "v1", 1)).is_some());
        let w = r.resolve(ev("b", "s", "v2", 2)).unwrap();
        assert_eq!(w.value, "v2");
    }

    #[test]
    fn test_first_write_wins() {
        let mut r = ConflictResolver::new(ConflictStrategy::FirstWriteWins);
        assert!(r.resolve(ev("a", "s", "v1", 1)).is_some());
        assert!(r.resolve(ev("b", "s", "v2", 2)).is_none());
    }

    #[test]
    fn test_prefer_agent() {
        let mut r = ConflictResolver::new(ConflictStrategy::PreferAgent).with_preferred("agent1");
        assert!(r.resolve(ev("agent1", "s", "primary", 1)).is_some());
        assert!(r.resolve(ev("agent2", "s", "secondary", 2)).is_none());
    }

    #[test]
    fn test_same_version_keeps_original() {
        let mut r = ConflictResolver::new(ConflictStrategy::LastWriteWins);
        r.resolve(ev("a", "k", "v1", 1));
        assert!(r.resolve(ev("b", "k", "v2", 1)).is_none());
    }
}
