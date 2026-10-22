//! LLM集成模块
//!
//! 支持多Provider调用，工具定义和消息管理。

pub mod provider;
pub mod message;
pub mod tool;
pub mod openai;
pub mod stream;
pub mod attention;
pub mod tool_executor;
#[path = "loop_mod.rs"]
pub mod agent_loop;

pub use provider::*;
pub use message::*;
pub use tool::*;
pub use openai::*;
pub use stream::*;
pub use attention::*;
pub use tool_executor::*;
pub use agent_loop::*;

// Re-export set_work_dir and set_search_api_key for external use
pub use tool_executor::set_work_dir;
pub use tool_executor::set_search_api_key;
