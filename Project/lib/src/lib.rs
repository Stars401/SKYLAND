pub mod proto;
pub mod types;
pub mod utils;
pub mod handoff;
pub mod memory;
pub mod sync;
pub mod llm;
pub mod permissions;
pub mod native_notify;

pub mod skyland {
    tonic::include_proto!("skyland");
}
