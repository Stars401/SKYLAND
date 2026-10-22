//! 状态同步集成测试

use skyland_lib::sync::{
    StateChannel, StatePublisher, StateSubscriber, ConflictResolver, ConflictStrategy,
};

#[test]
fn test_pub_sub_full_flow() {
    let ch = std::sync::Arc::new(StateChannel::new(16));
    let mut code_pub = StatePublisher::new("code_agent", ch.clone());
    let mut review_pub = StatePublisher::new("review_agent", ch.clone());
    let sub = StateSubscriber::new(ch);

    code_pub.publish("status", "running");
    review_pub.publish("status", "reviewing");

    let mut events = Vec::new();
    while let Some(e) = sub.try_recv() {
        events.push(e);
    }

    assert_eq!(events.len(), 2);
    assert!(events.iter().any(|e| e.agent_id == "code_agent"));
    assert!(events.iter().any(|e| e.agent_id == "review_agent"));
}

#[test]
fn test_conflict_last_write_wins() {
    let mut resolver = ConflictResolver::new(ConflictStrategy::LastWriteWins);
    use skyland_lib::sync::publisher::StateEvent;

    let e1 = StateEvent { agent_id: "a1".into(), key: "s".into(), value: "v1".into(), version: 1 };
    let e2 = StateEvent { agent_id: "a2".into(), key: "s".into(), value: "v2".into(), version: 2 };

    resolver.resolve(e1);
    let winner = resolver.resolve(e2).unwrap();
    assert_eq!(winner.value, "v2");
}

#[test]
fn test_channel_overflow_handling() {
    let ch = std::sync::Arc::new(StateChannel::new(2));
    let mut pub1 = StatePublisher::new("a1", ch.clone());
    let mut pub2 = StatePublisher::new("a2", ch.clone());
    let sub = StateSubscriber::new(ch);

    pub1.publish("k", "v1");
    pub2.publish("k", "v2");

    // Should receive at least events that fit in buffer
    let count = std::iter::from_fn(|| sub.try_recv()).count();
    assert!(count >= 2);
}

#[test]
fn test_multi_agent_event_ordering() {
    let ch = std::sync::Arc::new(StateChannel::new(10));
    let mut pub1 = StatePublisher::new("code", ch.clone());
    let sub = StateSubscriber::new(ch);

    let versions: Vec<_> = (0..3).map(|i| pub1.publish("k", &format!("v{}", i))).collect();
    assert_eq!(versions, vec![1, 2, 3]);

    let events: Vec<_> = std::iter::from_fn(|| sub.try_recv()).collect();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].version, 1);
    assert_eq!(events[2].version, 3);
}

#[test]
fn test_subscriber_handler_dispatch() {
    use std::sync::{Arc, Mutex};
    let ch = Arc::new(StateChannel::new(10));
    let mut pub1 = StatePublisher::new("agent", ch.clone());
    let mut sub = StateSubscriber::new(ch);

    let hits = Arc::new(Mutex::new(0));
    let h = hits.clone();
    sub.on_event(move |_| { *h.lock().unwrap() += 1; });

    pub1.publish("k", "v");
    sub.try_recv();
    assert_eq!(*hits.lock().unwrap(), 1);
}
