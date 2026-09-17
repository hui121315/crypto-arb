use super::*;
use arbitrage::models::{RawOpportunity, RawOpportunityExtra};
use arbitrage::{CostBreakdown, OpportunityBuilder, PositionSizing, RiskMetrics};
use shared_types::{
    ArbitrageType, ClientOrderIdDerivation, ExecutionCostProfile, ExecutionMode, ExecutionRun,
    ExecutionRunLeg, ExecutionRunState, FeeProduct, FundingRateData, HedgeExecutionParams,
    HedgeLegQuote, HedgeLegRole, MarginMode, MarketDataHealth, MarketDataQuality,
    MarketDataSourceKind, OneCycleCostProfile, OpportunityLegMarketEvidence,
    OrderPayloadPricePolicy, OrderSide, OrderType, PositionInfo, StrategyKind, TimeInForce,
    VenueOrderKind,
};

#[path = "hedge_tests/compile_basic.rs"]
mod compile_basic;
#[path = "hedge_tests/compile_venue_capabilities.rs"]
mod compile_venue_capabilities;
#[path = "hedge_tests/compile_venue_guards.rs"]
mod compile_venue_guards;
#[path = "hedge_tests/compile_venue_market.rs"]
mod compile_venue_market;
#[path = "hedge_tests/fixtures.rs"]
mod fixtures;
#[path = "hedge_tests/position_evidence.rs"]
mod position_evidence;
#[path = "hedge_tests/pr_ak.rs"]
mod pr_ak;
#[path = "hedge_tests/preview.rs"]
mod preview;
#[path = "hedge_tests/pricing_confirm.rs"]
mod pricing_confirm;
#[path = "hedge_tests/pricing_confirm_context.rs"]
mod pricing_confirm_context;
#[path = "hedge_tests/pricing_confirm_partial_outcome.rs"]
mod pricing_confirm_partial_outcome;
#[path = "hedge_tests/readiness.rs"]
mod readiness;

use fixtures::*;
