//! 交接文档解析模块

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("文档格式错误: {0}")]
    FormatError(String),

    #[error("缺少必要字段: {0}")]
    MissingField(String),

    #[error("解析错误: {0}")]
    ParseError(String),
}

pub mod session;
pub mod board;
pub mod agents;

pub use session::*;
pub use board::*;
pub use agents::*;
