#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::too_many_lines
    )
)]
//! 跨 crate 通用工具：错误、日志、配置、时间、签名。

pub mod config;
pub mod error;
pub mod logging;
pub mod request_id;
pub mod signing;
pub mod time;

pub use error::{AppError, AppResult};
pub use http::StatusCode;
