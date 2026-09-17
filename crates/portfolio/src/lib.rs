#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::too_many_lines
    )
)]
//! Portfolio aggregation primitives for UI-facing summary and risk snapshots.

pub mod liquidation;
pub mod pairing;
pub mod profit_exit;
pub mod risk;
pub mod snapshot;
pub mod summary;

pub use liquidation::annotate_liquidation_distance;
pub use pairing::pair_positions;
pub use profit_exit::{
    profit_exit_candidates, ProfitExitCandidate, ProfitExitTrigger, ProfitExitValuation,
};
pub use risk::{compute_risk, RiskInputs};
pub use shared_types::PortfolioSnapshot;
pub use snapshot::{new_shared, SharedPortfolioSnapshot};
pub use summary::{compute_summary, SummaryInputs};
