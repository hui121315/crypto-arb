#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::too_many_lines
    )
)]
//! Review primitives for executed trades, missed opportunities, and strategy PnL.

pub mod executed;
pub mod fixtures;
pub mod missed;
pub mod realized_pnl;
pub mod strategy_perf;

pub use executed::{executed_from_orders, executed_page_from_orders, normalize_net_pnl};
pub use fixtures::{sample_executed, sample_missed, sample_strategy_performance};
pub use missed::filter_recent_missed;
pub use realized_pnl::{
    apply_realized_pnl, realized_pnl_by_group, realized_pnl_by_group_with_close_runs,
    realized_pnl_field_quality, RealizedPnlFieldQuality, RealizedPnlRow,
};
pub use strategy_perf::{compute_performance, compute_performance_by_environment};
