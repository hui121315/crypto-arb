//! 投资组合 / 风险 DTO。

use crate::{
    enums::OrderSide,
    execution_ledger::ExecutionLedgerQuality,
    live_trading::{OrderRecord, OrderUpdateSource},
    orders::{AccountFieldQualityStatus, AccountStateSnapshot, VenueBalanceInfo},
    problem::ApiProblem,
    review::ReviewPnlField,
    system::RuntimeProblem,
    venues::VenueOperationHealth,
};
use serde::{Deserialize, Serialize};

pub const CLOSE_ALL_POSITIONS_CONFIRMATION_PHRASE: &str = "CLOSE_ALL_POSITIONS";
pub const CLOSE_RUN_COMPENSATION_CONFIRMATION_PHRASE: &str = "COMPENSATE_CLOSE_RUN";
pub const CLOSE_RUN_MANUAL_TERMINAL_CONFIRMATION_PHRASE: &str = "MANUAL_TERMINATE_CLOSE_RUN";

mod close;
mod positions;
mod profit_exit;
mod risk;
mod snapshot;
#[cfg(test)]
mod tests;

pub use close::*;
pub use positions::*;
pub use profit_exit::*;
pub use risk::*;
pub use snapshot::*;
