use crate::services::instrument_registry::CandidateTransferStatus;
use crate::services::market_data::{MarketQuality, MarketRead, MarketSource};
use crate::state::AppState;
use futures::future::join;
#[cfg(test)]
use shared_types::ArbitrageType;
use shared_types::{
    p0_hedge_leg_product, strategy_execution_cycle, ApiProblem, ArbitrageOpportunityDto,
    ExecutionCostProfile, ExecutionGuard, FeeProduct, HedgeDepthStatus, HedgeExecutableNotional,
    HedgeExecutionParams, HedgeLegQuote, HedgeLegRole, HedgeSizing, HedgeTicket, LegCostBreakdown,
    MarketDataCoverage, MarketDataHealth, MarketDataQuality, MarketDataSourceKind,
    OneCycleCostProfile, OpportunityLegMarketEvidence, OrderBookInfo, OrderRecord, OrderSide,
    OrderType, RoundTripCostBreakdown, SpotLegMode, StrategyExecutionCycle, StrategyKind,
    TradeFeeSnapshot, TRANSFER_ROUTE_EVIDENCE_KEY,
};
use uuid::Uuid;

const TICKET_TTL_MS: i64 = 60_000;
const STALE_MARKET_MS: i64 = 30_000;
const EXECUTION_DEPTH_BPS: f64 = 5.0;

pub(crate) async fn build_ticket(
    state: &AppState,
    opp: &ArbitrageOpportunityDto,
    params: &HedgeExecutionParams,
    long_notional_cap: f64,
    short_notional_cap: f64,
) -> HedgeTicket {
    let now_ms = common::time::now_ms();
    let target_notional = (params.capital_usd * params.leverage).max(0.0);
    let notional_caps = [long_notional_cap, short_notional_cap];
    let (long_leg, short_leg, target_base_quantity) =
        build_ticket_leg_quotes(state, opp, notional_caps, now_ms).await;
    let sizing = sizing(
        params,
        target_notional,
        notional_caps,
        target_base_quantity,
        &long_leg,
        &short_leg,
    );
    let mut blockers = opp.execution_blockers.clone();
    blockers.extend(long_leg.blockers.iter().cloned());
    blockers.extend(short_leg.blockers.iter().cloned());
    let use_maker_fee = use_maker_fee(params);
    let long_fee = fee_lookup(state, opp, HedgeLegRole::Long, use_maker_fee, now_ms);
    let short_fee = fee_lookup(state, opp, HedgeLegRole::Short, use_maker_fee, now_ms);
    blockers.extend(long_fee.blockers.iter().cloned());
    blockers.extend(short_fee.blockers.iter().cloned());
    let fee_snapshots = fee_snapshots(&long_fee, &short_fee);
    let cost = ticket_cost(
        opp,
        long_fee.snapshot.as_ref(),
        short_fee.snapshot.as_ref(),
        &long_leg,
        &short_leg,
    );
    let profit_proof = strategy_profit_proof(
        opp.strategy_kind,
        opp.spot_leg_mode,
        &long_leg,
        &short_leg,
        cost.as_ref(),
        now_ms,
    );
    let fees_required = state.trading_service().risk_config().live_trading_enabled;
    let long_target_notional = target_base_quantity
        .and_then(|quantity| long_leg.open_vwap_price.map(|price| quantity * price))
        .unwrap_or(long_notional_cap);
    let short_target_notional = target_base_quantity
        .and_then(|quantity| short_leg.open_vwap_price.map(|price| quantity * price))
        .unwrap_or(short_notional_cap);
    let mut guards = guards(&GuardInputs {
        opp,
        long: &long_leg,
        short: &short_leg,
        long_target_notional,
        short_target_notional,
        blockers: &blockers,
        fees_required,
        fee_snapshot_count: fee_snapshots.len(),
        cost: cost.as_ref(),
        profit_proof: &profit_proof,
    });
    guards.push(execution_order_guard(&long_leg, &short_leg));
    guards.push(transfer_route_guard(state, opp, now_ms));
    append_failed_guard_blockers(&mut blockers, &guards);

    HedgeTicket {
        ticket_id: format!("ticket-{}", Uuid::new_v4()),
        opportunity_id: opp.id.clone(),
        strategy: opp.strategy_kind,
        spot_leg_mode: opp.spot_leg_mode,
        symbol: opp.symbol.clone(),
        created_at_ms: now_ms,
        market_checked_at_ms: now_ms,
        expires_at_ms: now_ms + TICKET_TTL_MS,
        long_leg,
        short_leg,
        cost,
        fee_snapshots,
        sizing,
        guards,
        blockers: dedup(blockers),
    }
}

fn transfer_route_guard(
    state: &AppState,
    opportunity: &ArbitrageOpportunityDto,
    now_ms: i64,
) -> ExecutionGuard {
    let (passed, detail) = match state
        .instrument_registry()
        .candidate_transfer_status(opportunity, now_ms)
    {
        CandidateTransferStatus::NotApplicable => (
            true,
            "当前策略不涉及跨场现货再平衡，无需充提路径".to_owned(),
        ),
        CandidateTransferStatus::NotRequired { detail }
        | CandidateTransferStatus::Available { detail, .. } => (true, detail),
        CandidateTransferStatus::Warming { detail }
        | CandidateTransferStatus::Blocked { detail } => (false, detail),
    };
    ExecutionGuard {
        key: TRANSFER_ROUTE_EVIDENCE_KEY.to_owned(),
        label: "充提路径".to_owned(),
        passed,
        detail,
        preflight_outcome: None,
    }
}

pub(crate) fn ticket_expired(ticket: &HedgeTicket) -> bool {
    common::time::now_ms() > ticket.expires_at_ms
}

pub(crate) fn ticket_ready(ticket: &HedgeTicket) -> bool {
    ticket.blockers.is_empty() && ticket.guards.iter().all(|guard| guard.passed)
}

pub(crate) fn ticket_profit_proof(
    ticket: &HedgeTicket,
) -> arbitrage::profit_proof::StrategyProfitProof {
    strategy_profit_proof(
        ticket.strategy,
        ticket.spot_leg_mode,
        &ticket.long_leg,
        &ticket.short_leg,
        ticket.cost.as_ref(),
        ticket_market_checked_at_ms(ticket),
    )
}

pub(crate) fn ticket_market_checked_at_ms(ticket: &HedgeTicket) -> i64 {
    if ticket.market_checked_at_ms > 0 {
        ticket.market_checked_at_ms
    } else {
        ticket.created_at_ms
    }
}

fn strategy_profit_proof(
    strategy: Option<StrategyKind>,
    spot_leg_mode: Option<SpotLegMode>,
    long_leg: &HedgeLegQuote,
    short_leg: &HedgeLegQuote,
    cost: Option<&ExecutionCostProfile>,
    observed_at_ms: i64,
) -> arbitrage::profit_proof::StrategyProfitProof {
    arbitrage::profit_proof::evaluate_strategy_profit(
        arbitrage::profit_proof::StrategyProfitProofInput {
            strategy,
            spot_leg_mode,
            funding: arbitrage::profit_proof::FundingWindowInput {
                long_funding_bps: long_leg.funding_bps,
                short_funding_bps: short_leg.funding_bps,
                long_next_settlement_ms: long_leg.next_funding_time,
                short_next_settlement_ms: short_leg.next_funding_time,
                long_interval_hours: long_leg.funding_interval_hours,
                short_interval_hours: short_leg.funding_interval_hours,
            },
            executable_price: arbitrage::profit_proof::ExecutablePriceInput {
                long_open_price: long_leg.open_vwap_price,
                short_open_price: short_leg.open_vwap_price,
                long_observed_at_ms: leg_observed_at_ms(long_leg),
                short_observed_at_ms: leg_observed_at_ms(short_leg),
            },
            gross_edge_bps: cost.map(|value| value.gross_edge_bps),
            total_cost_bps: cost.map(|value| value.total_cost_bps),
            mismatch_buffer_bps: cost
                .map(|value| value.one_cycle.funding_window_mismatch_buffer_bps),
            target_buffer_bps: cost.map(|value| value.one_cycle.target_buffer_bps),
            observed_at_ms,
        },
    )
}

fn leg_observed_at_ms(leg: &HedgeLegQuote) -> i64 {
    leg.market_timestamp_ms
        .filter(|timestamp| *timestamp > 0)
        .or_else(|| {
            leg.market_evidence
                .as_ref()
                .map(|evidence| evidence.health.observed_at_ms)
                .filter(|timestamp| *timestamp > 0)
        })
        .unwrap_or_default()
}

pub(crate) fn append_guard(ticket: &mut HedgeTicket, guard: ExecutionGuard) {
    if !guard.passed {
        ticket.blockers.push(guard.detail.clone());
        ticket.blockers = dedup(std::mem::take(&mut ticket.blockers));
    }
    ticket.guards.push(guard);
}

fn order_side_label(side: OrderSide) -> &'static str {
    match side {
        OrderSide::Buy => "买入腿",
        OrderSide::Sell => "卖出腿",
    }
}

mod cost;
mod execution_order;
mod fees;
mod guards;
mod orderbooks;
mod pairing;
mod pricing;
mod quote;
mod quote_health;
mod sizing;
mod spec;
mod submit_market;
#[cfg(test)]
mod tests;

use cost::*;
#[cfg(test)]
use execution_order::execution_order_for_quotes;
pub(crate) use execution_order::{execution_order, opposite_role, role_label, HedgeExecutionOrder};
use execution_order::{execution_order_guard, refresh_execution_order_guard};
pub(crate) use fees::*;
use guards::*;
use orderbooks::*;
pub(crate) use orderbooks::{cached_opportunity_orderbooks, CachedLegOrderbook};
use pairing::*;
use pricing::*;
use quote::*;
use quote_health::*;
use sizing::*;
use spec::*;
pub(crate) use submit_market::{
    refresh_second_leg_market, refresh_submit_market, SubmitMarketSnapshot,
};
