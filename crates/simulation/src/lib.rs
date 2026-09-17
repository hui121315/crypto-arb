#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::too_many_lines
    )
)]
//! 模拟交易：仓位 + PnL + 持久化。
//!
//! V1 范围：现金账户、Long/Short 仓位、基础杠杆保证金、PnL（实现/未实现）、JSON 文件持久化。

pub mod persistence;
pub mod portfolio;
pub mod types;

pub use persistence::{
    from_json, load_from_file, save_to_file, to_json, PersistError, PersistResult,
};
pub use portfolio::{SimError, SimPortfolio, SimResult};
pub use types::{default_leverage, ClosedTrade, OpenRequest, PositionSide, SimPosition};
