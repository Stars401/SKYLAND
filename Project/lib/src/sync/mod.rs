//! 状态同步模块
//!
//! 支持智能体间状态发布/订阅、实时同步和冲突解决。

pub mod publisher;
pub mod subscriber;
pub mod channel;
pub mod conflict;

pub use publisher::*;
pub use subscriber::*;
pub use channel::*;
pub use conflict::*;
