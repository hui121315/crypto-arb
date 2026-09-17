#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::too_many_lines
    )
)]

pub mod spec;

pub use spec::{HedgeMode, SizeClause, StrategyLimits, StrategySpec, WhenClause};
