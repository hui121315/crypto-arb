use crate::lifecycle::nav_persist;
use crate::services::{
    account_equity_scope, account_quality, account_state, execution_environment,
    market_data::MarketQuality, portfolio_pnl::PortfolioPnlToday, venue_operation_health,
};
use crate::state::AppState;
use portfolio::{
    annotate_liquidation_distance, compute_risk, compute_summary, pair_positions, RiskInputs,
    SummaryInputs,
};
use shared_types::{
    normalized_venue_name, AccountEquityScope, AccountFieldQuality, AccountFieldQualityStatus,
    AccountFieldSubject, AccountStateSnapshot, ApiProblem, CloseRun, ExecutionMode, ExecutionRun,
    ExecutionRunLeg, ExecutionRunState, FundingRateData, HardLimitsUsage, HedgeLegRole,
    HistoryBackendStatus, HistoryPage, HistoryResponse, ListStatus, LiveOrderState, OrderRecord,
    OrderSide, PortfolioNavBreakdown, PortfolioNavEvidence, PortfolioNavHistoryRow,
    PortfolioSnapshot, PortfolioSummary, PortfolioValueEvidence, PositionInfo, PositionOrigin,
    PositionPairEvidence, PositionPairEvidenceSource, PositionRow, PositionSeverity, PositionSide,
    RiskSnapshot, RuntimeProblem, TickerInfo, VenueOperationHealth, VenueOperationStatus,
};
use std::{
    cmp::Reverse,
    collections::{BTreeSet, HashMap},
};

const NAV_HISTORY_MS: i64 = 26 * 60 * 60 * 1_000;
const NAV_LOOKBACK_MS: i64 = 24 * 60 * 60 * 1_000;
const NAV_SAMPLE_MS: i64 = 5 * 60 * 1_000;
const NAV_HISTORY_DEFAULT_LIMIT: usize = 288;
const NAV_HISTORY_MAX_LIMIT: usize = 1_000;
const NAV_HISTORY_SOURCE: &str = "portfolio_nav_history";
const RECENT_CLOSE_RUN_LIMIT: usize = 8;
const PORTFOLIO_FUNDING_SOURCE: &str = "portfolio_funding_runtime";

mod build;
mod nav;
mod outcome;
mod pricing;
mod risk;
mod snapshot;
#[cfg(test)]
mod tests;

use build::*;
pub(crate) use nav::*;
pub(crate) use outcome::*;
use pricing::*;
pub(crate) use risk::*;
pub(crate) use snapshot::*;
