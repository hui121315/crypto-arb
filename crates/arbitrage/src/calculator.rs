//! 套利机会计算器：将原始机会 + 风险/仓位/成本指标合成完整 DTO。

use crate::algorithms::{funding_timeline, futures_fields, perp_cross_policy, time_value};
use crate::models::{
    CostBreakdown, FundingMarketEvidence, PositionSizing, RawOpportunity, RiskMetrics,
};
use crate::strategy_registry;
use chrono::Utc;
use shared_types::{
    p0_hedge_leg_product, strategy_execution_cycle, ArbitrageOpportunityDto, ArbitrageType,
    ExecutionCostProfile, FeeProduct, FundingDiffWindowStats, FundingPrediction, HedgeLegRole,
    IndexCompositionQuality, IndexCompositionRiskProfile, IndexCompositionStatus,
    MarketDataQuality, MarketDataSourceKind, OneCycleCostProfile, OpportunityLegMarketEvidence,
    ProfitabilityEvidence, Recommendation, RiskLevel, RoundTripCostBreakdown, SpotLegMode,
    StrategyExecutionCycle, StrategyKind, YieldBasis, DEFERRED_INVENTORY_OR_BORROW_BLOCKER,
    DEFERRED_PERP_PRICE_SPREAD_EXIT_BLOCKER, DEFERRED_SPOT_PERP_TICKET_BLOCKER,
    FUNDING_WS_EVIDENCE_BLOCKER, PROFITABILITY_EVIDENCE_SOURCE,
};

const MIN_ONE_CYCLE_PROFIT_BPS: f64 = 0.0;

#[derive(Debug)]
pub struct OpportunityBuilder<'a> {
    pub raw: &'a RawOpportunity,
    pub metrics: &'a RiskMetrics,
    pub position: &'a PositionSizing,
    pub cost: &'a CostBreakdown,
    pub min_holding_periods: u32,
    pub net_single_yield: f64,
    pub data_source: &'a str,
    pub confidence: f64,
}

impl<'a> OpportunityBuilder<'a> {
    pub fn build(self) -> ArbitrageOpportunityDto {
        self.build_at(Utc::now())
    }

    pub fn build_at(self, observed_at: chrono::DateTime<Utc>) -> ArbitrageOpportunityDto {
        let (raw, m, p, c) = (self.raw, self.metrics, self.position, self.cost);
        let observed_at_ms = observed_at.timestamp_millis();

        let derived = OpportunityDerived::from_inputs(&self, raw, c, observed_at_ms);
        let strategy_kind = raw
            .extra
            .strategy_kind
            .unwrap_or_else(|| futures_fields::classify_strategy(raw.arb_type));
        let (long_action, short_action) = action_labels(raw);

        let execution_cost =
            execution_cost_profile_at(raw, c, self.min_holding_periods, observed_at_ms);
        let risk_warnings = build_warnings(raw, c);
        let execution_blockers =
            build_execution_blockers(raw, self.net_single_yield, &execution_cost, observed_at_ms);
        let recommendation = recommendation_from_profit(
            &execution_cost,
            derived.risk_level,
            execution_blockers.is_empty(),
        );
        let strategy_description =
            strategy_description(raw, &long_action, &short_action, self.net_single_yield);
        let funding_window_alignment = funding_window_alignment_minutes(raw);
        let settlement_countdown =
            futures_fields::settlement_countdown_seconds(derived.time_to_settlement_ms);
        let history_windows = history_windows(raw);
        let (spread_8h, long_rate_8h, short_rate_8h) = legacy_funding_fields(raw);

        ArbitrageOpportunityDto {
            id: stable_opportunity_id(raw),
            symbol: raw.symbol.clone(),
            arb_type: raw.arb_type,
            type_label: type_label_for(raw),
            long_exchange: raw.long_exchange.clone(),
            short_exchange: raw.short_exchange.clone(),
            spread_8h,
            long_rate_8h,
            short_rate_8h,
            long_rate: raw.long_rate.rate,
            short_rate: raw.short_rate.rate,
            single_yield: raw.single_yield,
            net_single_yield: self.net_single_yield,
            raw_single_yield: raw.single_yield,
            settlement_interval: settlement_interval_hours(raw),
            risk_adjusted_yield: derived.risk_adjusted_yield,
            trading_cost_rate: c.round_trip_cost,
            min_holding_periods: self.min_holding_periods,
            risk_level: derived.risk_level,
            volatility: m.volatility,
            sharpe_ratio: m.sharpe_ratio,
            score: 0.0,
            score_breakdown: None,
            ranking_key: None,
            recommendation,
            optimal_position: p.optimal_position,
            max_position: p.max_position,
            liquidity_score: derived.liquidity_score,
            volume_24h: raw.long_rate.volume_24h.min(raw.short_rate.volume_24h),
            long_volume_24h: raw.long_rate.volume_24h,
            short_volume_24h: raw.short_rate.volume_24h,
            data_source: self.data_source.into(),
            confidence: self.confidence,
            updated_at: observed_at,
            long_funding_interval: raw.long_rate.funding_interval,
            short_funding_interval: raw.short_rate.funding_interval,
            settlement_time_diff: derived.settlement_time_diff,
            strategy_description,
            long_action,
            short_action,
            long_next_funding_time: raw.long_rate.next_funding_time,
            short_next_funding_time: raw.short_rate.next_funding_time,
            time_to_settlement_ms: derived.time_to_settlement_ms,
            is_snipe_ready: derived.is_snipe_ready,
            long_price: raw.extra.long_price,
            short_price: raw.extra.short_price,
            long_leg_market_evidence: raw.extra.long_leg_market_evidence.clone(),
            short_leg_market_evidence: raw.extra.short_leg_market_evidence.clone(),
            quote_conversions: raw.extra.quote_conversions.clone(),
            price_deviation: raw.extra.price_deviation,
            basis_spread: raw.extra.basis_spread,
            basis_annual_cost: raw.extra.basis_annual_cost,
            risk_warnings,
            execution_eligible: execution_blockers.is_empty(),
            execution_blockers,
            execution_cost: Some(execution_cost),
            index_composition: index_composition_profile(raw),
            strategy_kind: Some(strategy_kind),
            strategy_category: Some(strategy_kind.category()),
            spot_leg_mode: raw.extra.spot_leg_mode,
            basis_bps: basis_bps(raw, self.net_single_yield),
            annualized_funding_bps: annualized_funding_bps(raw, self.net_single_yield),
            triangular_path: raw.extra.triangular_path.clone(),
            onchain_metadata: raw.extra.onchain_metadata.clone(),
            predicted_next_funding: Some(derived.predicted_next_funding.clone()),
            funding_diff_window: history_windows.primary,
            funding_diff_windows: history_windows.all,
            borrow_cost_bps_per_day: futures_fields::borrow_cost_bps_per_day(raw.arb_type),
            funding_window_alignment_minutes: funding_window_alignment,
            // Venue funding caps are contract-specific. Keep this unknown until
            // official per-contract cap evidence is carried by the market DTO.
            funding_cap_distance_bps: None,
            min_hold_hours: Some(derived.min_hold_hours),
            settlement_countdown_seconds: Some(settlement_countdown),
        }
    }
}

fn funding_window_alignment_minutes(raw: &RawOpportunity) -> Option<i32> {
    futures_fields::funding_window_alignment_minutes(
        raw.long_rate.next_funding_time,
        raw.short_rate.next_funding_time,
    )
}

fn legacy_funding_fields(raw: &RawOpportunity) -> (f64, f64, f64) {
    if uses_native_one_cycle_fields(raw) {
        (raw.single_yield, raw.long_rate.rate, raw.short_rate.rate)
    } else {
        (raw.spread_8h, raw.long_rate.rate_8h, raw.short_rate.rate_8h)
    }
}

fn uses_native_one_cycle_fields(raw: &RawOpportunity) -> bool {
    matches!(
        raw.extra.strategy_kind,
        Some(
            StrategyKind::PerpCross
                | StrategyKind::PerpPriceSpread
                | StrategyKind::SpotPerp
                | StrategyKind::CrossSpotPerp
                | StrategyKind::SpotCross
        )
    ) || raw.extra.strategy_kind.is_none()
        && matches!(
            raw.arb_type,
            ArbitrageType::CrossExchange
                | ArbitrageType::SpotFutures
                | ArbitrageType::CrossSpotFutures
                | ArbitrageType::SpotCross
        )
}

#[cfg(test)]
pub(crate) fn execution_cost_profile(
    raw: &RawOpportunity,
    cost: &CostBreakdown,
    min_holding_periods: u32,
) -> ExecutionCostProfile {
    execution_cost_profile_at(
        raw,
        cost,
        min_holding_periods,
        Utc::now().timestamp_millis(),
    )
}

fn execution_cost_profile_at(
    raw: &RawOpportunity,
    cost: &CostBreakdown,
    min_holding_periods: u32,
    observed_at_ms: i64,
) -> ExecutionCostProfile {
    if is_perp_cross(raw) {
        return perp_cross_execution_cost_profile(raw, cost, min_holding_periods, observed_at_ms);
    }
    let gross_edge_bps =
        strategy_registry::gross_yield_at_horizon(raw, min_holding_periods).max(0.0) * 10_000.0;
    let wear_bps = wear_cost_bps(raw, cost);
    let bilateral_cost_bps = cost.round_trip_cost.max(0.0) * 10_000.0;
    let mismatch_buffer_bps = 0.0;
    let total_cost_bps = bilateral_cost_bps + mismatch_buffer_bps;
    let fee_bps = (bilateral_cost_bps - wear_bps).max(0.0);
    let one_cycle = one_cycle_cost_profile(gross_edge_bps, fee_bps, wear_bps, mismatch_buffer_bps);
    let realization = strategy_registry::yield_realization(raw);
    let one_time = realization != strategy_registry::YieldRealization::Recurring;
    let breakeven_periods = if one_time {
        1
    } else {
        bounded_breakeven_periods(gross_edge_bps, total_cost_bps, min_holding_periods.max(1))
    };
    let recommended_hold_periods = if one_time {
        1
    } else {
        breakeven_periods.saturating_add(1)
    };
    let interval = match realization {
        strategy_registry::YieldRealization::InstantSpread => 0.0,
        strategy_registry::YieldRealization::ConvergenceSpread => convergence_hold_hours(raw),
        strategy_registry::YieldRealization::ProjectedBasisCarry => {
            projected_basis_hold_hours(raw, observed_at_ms)
        }
        strategy_registry::YieldRealization::Recurring => settlement_interval_hours(raw) as f64,
    };
    let mut round_trip = raw.extra.cost_round_trip.clone();
    if let Some(round_trip) = round_trip.as_mut() {
        round_trip.total_cost_bps = total_cost_bps;
        round_trip.one_cycle_net_bps = one_cycle.net_bps;
        round_trip.funding_window_mismatch_buffer_bps = mismatch_buffer_bps;
        let evidence = profitability_evidence(raw, round_trip, observed_at_ms);
        round_trip.profitability_evidence = evidence;
    }
    ExecutionCostProfile {
        gross_edge_bps,
        fee_bps,
        wear_bps,
        total_cost_bps,
        one_cycle,
        breakeven_periods,
        breakeven_hours: breakeven_periods as f64 * interval,
        recommended_hold_periods,
        recommended_hold_hours: recommended_hold_periods as f64 * interval,
        net_bps_at_recommended_hold: if one_time {
            gross_edge_bps - total_cost_bps
        } else {
            gross_edge_bps * recommended_hold_periods as f64 - total_cost_bps
        },
        round_trip,
    }
}

fn perp_cross_execution_cost_profile(
    raw: &RawOpportunity,
    cost: &CostBreakdown,
    _min_holding_periods: u32,
    observed_at_ms: i64,
) -> ExecutionCostProfile {
    let wear_bps = wear_cost_bps(raw, cost);
    let bilateral_cost_bps = cost.round_trip_cost.max(0.0) * 10_000.0;
    let fee_bps = (bilateral_cost_bps - wear_bps).max(0.0);
    let projection = funding_timeline::first_funding_projection(raw);
    let gross_edge_bps = projection.map_or_else(
        || raw.single_yield.max(0.0) * 10_000.0,
        |value| value.funding_yield * 10_000.0,
    );
    let mismatch_buffer_bps = 0.0;
    let total_cost_bps = bilateral_cost_bps;
    let mut one_cycle =
        one_cycle_cost_profile(gross_edge_bps, fee_bps, wear_bps, mismatch_buffer_bps);
    one_cycle.yield_basis = Some(YieldBasis::NativeSettlement);
    one_cycle.long_next_settlement_ms =
        (raw.long_rate.next_funding_time > 0).then_some(raw.long_rate.next_funding_time);
    one_cycle.short_next_settlement_ms =
        (raw.short_rate.next_funding_time > 0).then_some(raw.short_rate.next_funding_time);
    let event_covers_cost = projection.is_some() && one_cycle.covers_round_trip_cost;
    let event_count = u32::from(event_covers_cost);
    let hold_hours = projection
        .filter(|_| event_covers_cost)
        .map_or(0.0, |value| {
            value.settlement_at_ms.saturating_sub(observed_at_ms).max(0) as f64 / 3_600_000.0
        });
    let mut round_trip = raw.extra.cost_round_trip.clone();
    if let Some(round_trip) = round_trip.as_mut() {
        round_trip.total_cost_bps = total_cost_bps;
        round_trip.one_cycle_net_bps = one_cycle.net_bps;
        round_trip.funding_window_mismatch_buffer_bps = mismatch_buffer_bps;
        round_trip.profitability_evidence = profitability_evidence(raw, round_trip, observed_at_ms);
    }
    ExecutionCostProfile {
        gross_edge_bps,
        fee_bps,
        wear_bps,
        total_cost_bps,
        one_cycle: one_cycle.clone(),
        breakeven_periods: event_count,
        breakeven_hours: hold_hours,
        recommended_hold_periods: event_count,
        recommended_hold_hours: hold_hours,
        net_bps_at_recommended_hold: one_cycle.net_bps,
        round_trip,
    }
}

fn profitability_evidence(
    raw: &RawOpportunity,
    round_trip: &RoundTripCostBreakdown,
    observed_at_ms: i64,
) -> ProfitabilityEvidence {
    let fee_snapshots = [
        round_trip.long_leg.fee_snapshot.as_ref(),
        round_trip.short_leg.fee_snapshot.as_ref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    ProfitabilityEvidence::from_fee_snapshots(
        PROFITABILITY_EVIDENCE_SOURCE,
        observed_at_ms,
        &fee_snapshots,
        profitability_history(raw),
    )
}

fn profitability_history(raw: &RawOpportunity) -> Option<shared_types::FundingHistoryEvidence> {
    if raw.extra.strategy_kind == Some(StrategyKind::PerpPriceSpread) {
        return None;
    }
    raw.extra
        .history_stats
        .as_ref()
        .map(|stats| stats.window.evidence.clone())
}

fn settlement_interval_hours(raw: &RawOpportunity) -> u32 {
    [
        active_funding_interval(raw, true),
        active_funding_interval(raw, false),
    ]
    .into_iter()
    .flatten()
    .min()
    .unwrap_or_else(|| {
        raw.long_rate
            .funding_interval
            .max(1)
            .min(raw.short_rate.funding_interval.max(1))
    })
}

fn active_funding_interval(raw: &RawOpportunity, long_leg: bool) -> Option<u32> {
    let rate = if long_leg {
        &raw.long_rate
    } else {
        &raw.short_rate
    };
    let has_funding =
        rate.rate.is_finite() && rate.rate.abs() > f64::EPSILON || rate.next_funding_time > 0;
    has_funding.then_some(rate.funding_interval.max(1))
}

fn projected_basis_hold_hours(raw: &RawOpportunity, observed_at_ms: i64) -> f64 {
    let next_settlement_ms = raw
        .long_rate
        .next_funding_time
        .min_nonzero(raw.short_rate.next_funding_time);
    time_value::time_to_settlement_ms_at(next_settlement_ms, observed_at_ms).max(0) as f64
        / 3_600_000.0
}

struct HistoryWindows {
    primary: Option<FundingDiffWindowStats>,
    all: Vec<FundingDiffWindowStats>,
}

fn history_windows(raw: &RawOpportunity) -> HistoryWindows {
    raw.extra.history_stats.as_ref().map_or_else(
        || HistoryWindows {
            primary: None,
            all: Vec::new(),
        },
        |stats| HistoryWindows {
            primary: Some(stats.window.clone()),
            all: stats.windows.clone(),
        },
    )
}

fn one_cycle_cost_profile(
    gross_edge_bps: f64,
    fee_bps: f64,
    wear_bps: f64,
    mismatch_buffer_bps: f64,
) -> OneCycleCostProfile {
    let open_fee_bps = fee_bps / 2.0;
    let close_fee_bps = fee_bps / 2.0;
    let open_slippage_bps = wear_bps / 2.0;
    let close_slippage_bps = wear_bps / 2.0;
    let net_bps = gross_edge_bps
        - open_fee_bps
        - close_fee_bps
        - open_slippage_bps
        - close_slippage_bps
        - mismatch_buffer_bps;
    OneCycleCostProfile {
        gross_edge_bps,
        open_fee_bps,
        close_fee_bps,
        open_slippage_bps,
        close_slippage_bps,
        funding_window_mismatch_buffer_bps: mismatch_buffer_bps,
        yield_basis: None,
        long_next_settlement_ms: None,
        short_next_settlement_ms: None,
        target_buffer_bps: MIN_ONE_CYCLE_PROFIT_BPS,
        net_bps,
        covers_round_trip_cost: net_bps > MIN_ONE_CYCLE_PROFIT_BPS,
    }
}

fn breakeven_periods(gross_edge_bps: f64, total_cost_bps: f64) -> u32 {
    if gross_edge_bps <= f64::EPSILON {
        return u32::MAX;
    }
    (total_cost_bps / gross_edge_bps).ceil().max(1.0) as u32
}

fn bounded_breakeven_periods(gross_edge_bps: f64, total_cost_bps: f64, floor: u32) -> u32 {
    breakeven_periods(gross_edge_bps, total_cost_bps)
        .min(1_000)
        .max(floor.min(1_000))
}

fn wear_cost_bps(raw: &RawOpportunity, cost: &CostBreakdown) -> f64 {
    raw.extra.cost_round_trip.as_ref().map_or_else(
        || {
            let trade_count = match strategy_execution_cycle(raw.extra.strategy_kind) {
                StrategyExecutionCycle::PairedOpenClose => 4.0,
                StrategyExecutionCycle::PairedOpenRebalance => 2.0,
            };
            cost.slippage.max(0.0) * trade_count * 10_000.0
        },
        |round_trip| (round_trip.open_slippage_bps + round_trip.close_slippage_bps).max(0.0),
    )
}

fn type_label_for(raw: &RawOpportunity) -> String {
    raw.extra
        .type_label
        .clone()
        .unwrap_or_else(|| type_label(raw.arb_type).into())
}

fn strategy_description(
    raw: &RawOpportunity,
    long_action: &str,
    short_action: &str,
    net_single_yield: f64,
) -> String {
    raw.extra.description.clone().unwrap_or_else(|| {
        format!(
            "{}，{}；目标周期净收益 {:.4}%",
            long_action,
            short_action,
            net_single_yield * 100.0
        )
    })
}

fn basis_bps(raw: &RawOpportunity, net_single_yield: f64) -> Option<f64> {
    raw.extra.basis_bps.or(Some(net_single_yield * 10_000.0))
}

fn annualized_funding_bps(raw: &RawOpportunity, _net_single_yield: f64) -> Option<f64> {
    if is_perp_cross(raw) {
        return None;
    }
    raw.extra.annualized_funding_bps
}

fn stable_opportunity_id(raw: &RawOpportunity) -> String {
    let key = raw
        .extra
        .strategy_kind
        .map(StrategyKind::as_query_value)
        .unwrap_or_else(|| arb_type_key(raw.arb_type));
    let mut id = String::with_capacity(
        key.len() + raw.long_exchange.len() + raw.short_exchange.len() + raw.symbol.len() + 64,
    );
    id.push_str(key);
    if let Some(mode) = raw.extra.spot_leg_mode {
        id.push('_');
        id.push_str(spot_leg_mode_key(mode));
    }
    id.push('_');
    id.push_str(&raw.long_exchange);
    id.push('_');
    id.push_str(&raw.short_exchange);
    id.push('_');
    id.push_str(&raw.symbol);
    id.push_str("_lm_");
    append_stable_id_component(&mut id, leg_market_symbol(raw, true));
    id.push_str("_sm_");
    append_stable_id_component(&mut id, leg_market_symbol(raw, false));
    id
}

fn leg_market_symbol(raw: &RawOpportunity, long_leg: bool) -> &str {
    let evidence = if long_leg {
        raw.extra.long_leg_market_evidence.as_ref()
    } else {
        raw.extra.short_leg_market_evidence.as_ref()
    };
    evidence
        .map(|value| value.symbol.trim())
        .filter(|value| !value.is_empty())
        .unwrap_or(raw.symbol.as_str())
}

fn append_stable_id_component(id: &mut String, value: &str) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.') {
            id.push(char::from(byte));
        } else {
            id.push('~');
            id.push(char::from(HEX[(byte >> 4) as usize]));
            id.push(char::from(HEX[(byte & 0x0f) as usize]));
        }
    }
}

const fn spot_leg_mode_key(mode: SpotLegMode) -> &'static str {
    match mode {
        SpotLegMode::BuySpot => "buy_spot",
        SpotLegMode::SellInventory => "sell_inventory",
        SpotLegMode::BorrowAndSell => "borrow_sell",
    }
}

fn arb_type_key(arb_type: ArbitrageType) -> &'static str {
    match arb_type {
        ArbitrageType::CrossExchange => "perp_cross",
        ArbitrageType::SpotFutures => "spot_perp",
        ArbitrageType::CrossSpotFutures => "cross_spot_perp",
        ArbitrageType::SpotCross => "spot_cross",
        ArbitrageType::Triangular => "triangular",
        ArbitrageType::FundingCarry => "funding_carry",
        ArbitrageType::OptionsPerpBasis => "options_perp_basis",
    }
}

fn action_labels(raw: &RawOpportunity) -> (String, String) {
    let fallback = match raw.arb_type {
        ArbitrageType::CrossExchange => (
            format!("{} 做多永续", raw.long_exchange),
            format!("{} 做空永续", raw.short_exchange),
        ),
        ArbitrageType::SpotFutures => (
            format!("{} 买入现货", raw.long_exchange),
            format!("{} 做空永续", raw.short_exchange),
        ),
        ArbitrageType::CrossSpotFutures => (
            format!("{} 买入现货", raw.long_exchange),
            format!("{} 做空永续", raw.short_exchange),
        ),
        ArbitrageType::SpotCross => (
            format!("{} 买入现货", raw.long_exchange),
            format!("{} 卖出现货", raw.short_exchange),
        ),
        ArbitrageType::Triangular => (
            format!("{} 执行三角路径", raw.long_exchange),
            format!("{} 回到计价资产", raw.short_exchange),
        ),
        ArbitrageType::FundingCarry => (
            format!("{} 持有收资金费腿", raw.long_exchange),
            format!("{} 持有对冲腿", raw.short_exchange),
        ),
        ArbitrageType::OptionsPerpBasis => (
            format!("{} 建立期权相对价值腿", raw.long_exchange),
            format!("{} 建立永续对冲腿", raw.short_exchange),
        ),
    };
    (
        raw.extra.long_action.clone().unwrap_or(fallback.0),
        raw.extra.short_action.clone().unwrap_or(fallback.1),
    )
}

#[derive(Debug)]
struct OpportunityDerived {
    liquidity_score: f64,
    risk_level: RiskLevel,
    risk_adjusted_yield: f64,
    time_to_settlement_ms: i64,
    is_snipe_ready: bool,
    settlement_time_diff: bool,
    predicted_next_funding: FundingPrediction,
    min_hold_hours: f64,
}

impl OpportunityDerived {
    fn from_inputs(
        builder: &OpportunityBuilder<'_>,
        raw: &RawOpportunity,
        cost: &CostBreakdown,
        observed_at_ms: i64,
    ) -> Self {
        let liquidity_score = liquidity_score_for(raw);
        let risk_level = strategy_registry::list_risk_level(raw);
        let next_funding_time = raw
            .long_rate
            .next_funding_time
            .min_nonzero(raw.short_rate.next_funding_time);
        let time_to_settlement_ms =
            time_value::time_to_settlement_ms_at(next_funding_time, observed_at_ms);
        let predicted_next_funding = futures_fields::predict_next_funding(
            raw.long_rate.rate,
            raw.short_rate.rate,
            builder.confidence,
        );

        Self {
            liquidity_score,
            risk_level,
            // Legacy wire slot. Scan cadence is not realized-return evidence.
            risk_adjusted_yield: 0.0,
            time_to_settlement_ms,
            is_snipe_ready: time_value::is_snipe_ready_at(next_funding_time, observed_at_ms),
            settlement_time_diff: if is_perp_cross(raw) {
                !funding_timeline::current_settlements_aligned(raw)
            } else if is_spot_perp(raw) {
                false
            } else {
                raw.long_rate.next_funding_time != raw.short_rate.next_funding_time
            },
            predicted_next_funding,
            min_hold_hours: match strategy_registry::yield_realization(raw) {
                strategy_registry::YieldRealization::InstantSpread => 0.0,
                strategy_registry::YieldRealization::ConvergenceSpread => {
                    convergence_hold_hours(raw)
                }
                strategy_registry::YieldRealization::ProjectedBasisCarry => {
                    if builder.min_holding_periods == u32::MAX {
                        0.0
                    } else {
                        time_to_settlement_ms.max(0) as f64 / 3_600_000.0
                    }
                }
                strategy_registry::YieldRealization::Recurring => {
                    funding_timeline::hold_hours(raw, builder.min_holding_periods, observed_at_ms)
                        .filter(|_| is_perp_cross(raw))
                        .unwrap_or_else(|| {
                            futures_fields::min_hold_hours(
                                builder.min_holding_periods,
                                settlement_interval_hours(raw),
                                cost.round_trip_cost,
                                builder.net_single_yield,
                            )
                        })
                }
            },
        }
    }
}

fn liquidity_score(long_vol: f64, short_vol: f64, reference: f64) -> f64 {
    let min_vol = long_vol.min(short_vol);
    ((min_vol / reference) * 100.0).clamp(0.0, 100.0)
}

pub(crate) fn liquidity_score_for(raw: &RawOpportunity) -> f64 {
    liquidity_score(
        raw.long_rate.volume_24h,
        raw.short_rate.volume_24h,
        10_000_000.0,
    )
}

fn type_label(t: ArbitrageType) -> &'static str {
    match t {
        ArbitrageType::CrossExchange => "跨所期期",
        ArbitrageType::SpotFutures => "同所期现",
        ArbitrageType::CrossSpotFutures => "跨所期现",
        ArbitrageType::SpotCross => "现货跨所",
        ArbitrageType::Triangular => "三角套利",
        ArbitrageType::FundingCarry => "资金费 Carry",
        ArbitrageType::OptionsPerpBasis => "期权-永续基差",
    }
}

fn one_cycle_shortfall_bps(raw: &RawOpportunity, cost: &CostBreakdown) -> f64 {
    (-one_cycle_net_bps(raw, cost)).max(0.0)
}

fn one_cycle_net_bps(raw: &RawOpportunity, cost: &CostBreakdown) -> f64 {
    let periods = strategy_registry::min_holding_periods(raw, cost);
    if periods == u32::MAX {
        return -cost.round_trip_cost.max(0.0) * 10_000.0;
    }
    strategy_registry::gross_yield_at_horizon(raw, periods) * 10_000.0
        - cost.round_trip_cost.max(0.0) * 10_000.0
}

pub(crate) fn price_deviation_bps(raw: &RawOpportunity) -> Option<f64> {
    raw.extra
        .price_deviation
        .filter(|value| value.is_finite())
        .map(|value| value.abs() * 10_000.0)
        .or_else(|| leg_price_deviation_bps(raw))
}

fn leg_price_deviation_bps(raw: &RawOpportunity) -> Option<f64> {
    let long = reference_price(raw.extra.long_price)?;
    let short = reference_price(raw.extra.short_price)?;
    let mid = ((long + short) * 0.5).max(f64::EPSILON);
    Some((long - short).abs() / mid * 10_000.0)
}

fn reference_price(value: Option<f64>) -> Option<f64> {
    value.filter(|price| price.is_finite() && *price > f64::EPSILON)
}

fn index_composition_profile(raw: &RawOpportunity) -> Option<IndexCompositionRiskProfile> {
    raw.extra
        .index_composition
        .as_ref()
        .map(|risk| IndexCompositionRiskProfile {
            status: index_composition_status(risk),
            overlap_score: risk.overlap_score.clamp(0.0, 1.0),
            long_quality: risk.long_quality,
            short_quality: risk.short_quality,
            blocker: risk.blocker.clone(),
            long_evidence: risk.long_evidence.clone(),
            short_evidence: risk.short_evidence.clone(),
        })
}

fn index_composition_status(risk: &crate::models::IndexCompositionRisk) -> IndexCompositionStatus {
    if risk.long_quality == IndexCompositionQuality::Error
        || risk.short_quality == IndexCompositionQuality::Error
    {
        return IndexCompositionStatus::Error;
    }
    if risk.long_quality == IndexCompositionQuality::Stale
        || risk.short_quality == IndexCompositionQuality::Stale
    {
        return IndexCompositionStatus::Stale;
    }
    if risk.long_quality == IndexCompositionQuality::Unsupported
        || risk.short_quality == IndexCompositionQuality::Unsupported
    {
        return IndexCompositionStatus::Unsupported;
    }
    if risk.long_quality != IndexCompositionQuality::Verified
        || risk.short_quality != IndexCompositionQuality::Verified
    {
        return IndexCompositionStatus::Unverified;
    }
    if risk.overlap_score < 0.75 {
        return IndexCompositionStatus::Mismatch;
    }
    if risk.hidden_price {
        return IndexCompositionStatus::HiddenPrice;
    }
    IndexCompositionStatus::Verified
}

fn recommendation_from_profit(
    cost: &ExecutionCostProfile,
    risk: RiskLevel,
    execution_ready: bool,
) -> Recommendation {
    if !has_verified_positive_one_cycle(cost) {
        return Recommendation::Avoid;
    }
    if !execution_ready || risk == RiskLevel::High {
        return Recommendation::Hold;
    }
    Recommendation::Buy
}

fn build_warnings(raw: &RawOpportunity, cost: &CostBreakdown) -> Vec<String> {
    let mut w = Vec::new();
    if warns_cost_amortization(raw) && cost.round_trip_cost > raw.spread_8h.abs() * 4.0 {
        w.push("交易成本较高，建议持仓多个周期摊销".into());
    }
    if raw.long_rate.is_outlier || raw.short_rate.is_outlier {
        w.push("某侧费率被识别为异常值，请人工确认".into());
    }
    if one_cycle_shortfall_bps(raw, cost) > 0.0 {
        w.push("单次净利润未覆盖双边开平成本，仅观察不执行".into());
    }
    if price_deviation_bps(raw).is_some_and(|bps| bps >= 50.0) {
        w.push("双边价格差较大，需在构建票据时重新核验可执行收益".into());
    }
    if let Some(risk) = raw.extra.index_composition.as_ref() {
        if risk.long_quality != IndexCompositionQuality::Verified
            || risk.short_quality != IndexCompositionQuality::Verified
        {
            w.push("指数成分未完成双边验证，仅观察不执行".into());
        } else if risk.overlap_score < 0.85 {
            w.push(format!(
                "指数成分重合度 {:.0}%，底层不完全一致，需阻断或重新核验",
                risk.overlap_score * 100.0
            ));
        }
    }
    w
}

fn warns_cost_amortization(raw: &RawOpportunity) -> bool {
    if is_perp_cross(raw) {
        return false;
    }
    if strategy_registry::yield_realization(raw) != strategy_registry::YieldRealization::Recurring {
        return false;
    }
    matches!(
        raw.arb_type,
        ArbitrageType::CrossExchange
            | ArbitrageType::FundingCarry
            | ArbitrageType::OptionsPerpBasis
    )
}

fn convergence_hold_hours(raw: &RawOpportunity) -> f64 {
    raw.extra
        .price_spread_convergence
        .map(|evidence| evidence.recommended_hold_ms.max(60_000) as f64 / 3_600_000.0)
        .unwrap_or(1.0 / 60.0)
}

fn build_execution_blockers(
    raw: &RawOpportunity,
    net_yield: f64,
    execution_cost: &ExecutionCostProfile,
    observed_at_ms: i64,
) -> Vec<String> {
    let mut blockers = Vec::new();
    for blocker in &raw.extra.execution_blockers {
        push_execution_blocker(&mut blockers, blocker);
    }
    if net_yield <= 0.0 {
        push_execution_blocker(&mut blockers, "当前净收益非正，仅观察不执行");
    }
    if !has_verified_cost_evidence(execution_cost) {
        push_execution_blocker(&mut blockers, "缺少双腿新鲜官方费率证据，不能证明费后收益");
    }
    if !has_verified_positive_one_cycle(execution_cost) {
        push_execution_blocker(&mut blockers, "单周期费后净收益下限非正，仅观察不执行");
    }
    match raw.extra.strategy_kind {
        Some(StrategyKind::PerpPriceSpread) => {
            push_execution_blocker(&mut blockers, DEFERRED_PERP_PRICE_SPREAD_EXIT_BLOCKER)
        }
        Some(StrategyKind::SpotPerp | StrategyKind::CrossSpotPerp) => {
            push_execution_blocker(&mut blockers, DEFERRED_SPOT_PERP_TICKET_BLOCKER)
        }
        _ => {}
    }
    append_trade_evidence_blockers(raw, &mut blockers);
    if missing_required_funding_ws_evidence(raw) {
        push_execution_blocker(&mut blockers, FUNDING_WS_EVIDENCE_BLOCKER);
    }
    if is_perp_cross(raw) {
        let entry = funding_timeline::assess_entry(raw, observed_at_ms);
        if !entry.passed {
            push_execution_blocker(&mut blockers, &entry.detail);
        }
    }
    if let Some(alignment) = perp_cross_price_alignment(raw, execution_cost) {
        if !alignment.passed {
            push_execution_blocker(&mut blockers, &alignment.detail);
        }
    }
    if requires_spot_inventory_or_borrow(raw.extra.spot_leg_mode)
        && !allows_prefunded_spot_inventory(raw)
    {
        push_execution_blocker(&mut blockers, DEFERRED_INVENTORY_OR_BORROW_BLOCKER);
    }
    if let Some(risk) = raw.extra.index_composition.as_ref() {
        if let Some(blocker) = risk.blocker.as_deref() {
            push_execution_blocker(&mut blockers, blocker);
        }
    }
    blockers
}

fn append_trade_evidence_blockers(raw: &RawOpportunity, blockers: &mut Vec<String>) {
    if !requires_trade_prices(raw) {
        return;
    }
    if missing_trade_price(raw) {
        push_execution_blocker(blockers, "缺少交易所双腿报价，仅观察不执行");
    }
    if missing_trade_market_evidence(raw) {
        push_execution_blocker(blockers, "缺少交易所双腿行情证据，仅观察不执行");
    }
}

fn has_verified_cost_evidence(cost: &ExecutionCostProfile) -> bool {
    cost.round_trip
        .as_ref()
        .is_some_and(|round_trip| round_trip.profitability_evidence.is_cost_verified())
}

fn has_verified_positive_one_cycle(cost: &ExecutionCostProfile) -> bool {
    has_verified_cost_evidence(cost)
        && cost.one_cycle.covers_round_trip_cost
        && cost.one_cycle.net_bps.is_finite()
        && cost.one_cycle.net_bps > f64::EPSILON
}

fn perp_cross_price_alignment(
    raw: &RawOpportunity,
    execution_cost: &ExecutionCostProfile,
) -> Option<perp_cross_policy::PriceAlignment> {
    is_perp_cross(raw).then(|| {
        perp_cross_policy::evaluate(perp_cross_policy::PriceAlignmentInput {
            long_open_price: raw.extra.long_price,
            short_open_price: raw.extra.short_price,
            long_observed_at_ms: market_observed_at_ms(raw.extra.long_leg_market_evidence.as_ref()),
            short_observed_at_ms: market_observed_at_ms(
                raw.extra.short_leg_market_evidence.as_ref(),
            ),
            projected_net_funding_bps: Some(
                execution_cost.one_cycle.net_bps - execution_cost.one_cycle.target_buffer_bps,
            ),
        })
    })
}

fn market_observed_at_ms(evidence: Option<&OpportunityLegMarketEvidence>) -> i64 {
    evidence.map_or(0, |evidence| evidence.health.observed_at_ms)
}

fn is_perp_cross(raw: &RawOpportunity) -> bool {
    raw.extra.strategy_kind == Some(StrategyKind::PerpCross)
        || raw.extra.strategy_kind.is_none() && raw.arb_type == ArbitrageType::CrossExchange
}

fn is_spot_perp(raw: &RawOpportunity) -> bool {
    raw.extra.strategy_kind == Some(StrategyKind::SpotPerp)
        || raw.extra.strategy_kind.is_none() && raw.arb_type == ArbitrageType::SpotFutures
}

fn allows_prefunded_spot_inventory(raw: &RawOpportunity) -> bool {
    raw.extra.strategy_kind == Some(StrategyKind::SpotCross)
        && raw.extra.spot_leg_mode == Some(SpotLegMode::SellInventory)
}

fn requires_spot_inventory_or_borrow(mode: Option<SpotLegMode>) -> bool {
    matches!(
        mode,
        Some(SpotLegMode::SellInventory | SpotLegMode::BorrowAndSell)
    )
}

fn requires_trade_prices(raw: &RawOpportunity) -> bool {
    matches!(
        raw.arb_type,
        ArbitrageType::CrossExchange
            | ArbitrageType::SpotFutures
            | ArbitrageType::CrossSpotFutures
            | ArbitrageType::SpotCross
    )
}

fn missing_trade_price(raw: &RawOpportunity) -> bool {
    !positive_price(raw.extra.long_price) || !positive_price(raw.extra.short_price)
}

fn missing_trade_market_evidence(raw: &RawOpportunity) -> bool {
    !fresh_ws_market_evidence(raw.extra.long_leg_market_evidence.as_ref())
        || !fresh_ws_market_evidence(raw.extra.short_leg_market_evidence.as_ref())
}

fn fresh_ws_market_evidence(evidence: Option<&OpportunityLegMarketEvidence>) -> bool {
    evidence.is_some_and(|evidence| {
        evidence.health.quality == MarketDataQuality::Fresh
            && evidence.health.source == MarketDataSourceKind::WsPush
            && evidence.health.observed_at_ms > 0
    })
}

fn missing_required_funding_ws_evidence(raw: &RawOpportunity) -> bool {
    let strategy = raw
        .extra
        .strategy_kind
        .or_else(|| Some(futures_fields::classify_strategy(raw.arb_type)));
    [HedgeLegRole::Long, HedgeLegRole::Short]
        .into_iter()
        .any(|role| {
            p0_hedge_leg_product(strategy, raw.extra.spot_leg_mode, role) == Some(FeeProduct::Perp)
                && !fresh_ws_funding_evidence(match role {
                    HedgeLegRole::Long => raw.extra.long_funding_evidence.as_ref(),
                    HedgeLegRole::Short => raw.extra.short_funding_evidence.as_ref(),
                })
        })
}

fn fresh_ws_funding_evidence(evidence: Option<&FundingMarketEvidence>) -> bool {
    evidence.is_some_and(|evidence| {
        evidence.quality == MarketDataQuality::Fresh
            && evidence.source == MarketDataSourceKind::WsPush
            && evidence.observed_at_ms > 0
    })
}

fn positive_price(value: Option<f64>) -> bool {
    reference_price(value).is_some()
}

fn push_execution_blocker(blockers: &mut Vec<String>, message: &str) {
    if !blockers.iter().any(|item| item == message) {
        blockers.push(message.to_owned());
    }
}

/// 内部辅助：取两个非零 i64 的较小值；若一方为 0 则取另一方。
trait MinNonZero {
    fn min_nonzero(self, other: Self) -> Self;
}

impl MinNonZero for i64 {
    fn min_nonzero(self, other: Self) -> Self {
        match (self == 0, other == 0) {
            (true, true) => 0,
            (true, false) => other,
            (false, true) => self,
            (false, false) => self.min(other),
        }
    }
}

#[cfg(test)]
mod mismatch_cost_tests {
    use super::*;

    #[test]
    fn perp_cross_rejects_staggered_events_and_never_amortizes_cost() {
        let now_ms = Utc::now().timestamp_millis();
        let mut raw = RawOpportunity {
            symbol: "BTC".to_owned(),
            arb_type: ArbitrageType::CrossExchange,
            long_exchange: "a".to_owned(),
            short_exchange: "b".to_owned(),
            long_rate: funding_rate("a", 0.000_2, 1, now_ms + 3_600_000),
            short_rate: funding_rate("b", 0.002, 8, now_ms + 7_200_000),
            spread_8h: 0.001_8,
            single_yield: 0.001_8,
            extra: crate::models::RawOpportunityExtra {
                strategy_kind: Some(StrategyKind::PerpCross),
                ..Default::default()
            },
        };
        let cost = CostBreakdown {
            round_trip_cost: 0.001,
            ..Default::default()
        };

        assert!(!funding_timeline::assess_entry(&raw, now_ms).passed);
        let staggered = execution_cost_profile(&raw, &cost, 1);
        assert_eq!(staggered.one_cycle.funding_window_mismatch_buffer_bps, 0.0);
        assert_eq!(staggered.recommended_hold_periods, 0);

        raw.short_rate.next_funding_time = raw.long_rate.next_funding_time + 10;
        let aligned = execution_cost_profile(&raw, &cost, 1);
        assert!(funding_timeline::assess_entry(&raw, now_ms).passed);
        assert_eq!(aligned.one_cycle.funding_window_mismatch_buffer_bps, 0.0);
        assert_eq!(aligned.total_cost_bps, 10.0);
        assert_eq!(aligned.recommended_hold_periods, 1);
        assert!((aligned.one_cycle.net_bps - 8.0).abs() < 1e-9);
    }

    #[test]
    fn spot_perp_hold_uses_time_until_next_native_event_not_full_interval() {
        let now_ms = Utc::now().timestamp_millis();
        let mut long_rate = funding_rate("spot", 0.0, 8, 0);
        long_rate.timestamp = now_ms;
        let mut short_rate = funding_rate("perp", 0.000_2, 8, now_ms + 1_800_000);
        short_rate.timestamp = now_ms;
        let raw = RawOpportunity {
            symbol: "BTC".to_owned(),
            arb_type: ArbitrageType::SpotFutures,
            long_exchange: "venue".to_owned(),
            short_exchange: "venue".to_owned(),
            long_rate,
            short_rate,
            spread_8h: 0.002,
            single_yield: 0.002,
            extra: crate::models::RawOpportunityExtra {
                strategy_kind: Some(StrategyKind::SpotPerp),
                ..Default::default()
            },
        };
        let cost = CostBreakdown {
            round_trip_cost: 0.001,
            ..Default::default()
        };

        let profile = execution_cost_profile(&raw, &cost, 1);

        assert!((profile.recommended_hold_hours - 0.5).abs() < 0.01);
    }

    fn funding_rate(
        exchange: &str,
        rate: f64,
        funding_interval: u32,
        next_funding_time: i64,
    ) -> shared_types::FundingRateData {
        shared_types::FundingRateData {
            symbol: "BTC".to_owned(),
            exchange: exchange.to_owned(),
            rate,
            rate_8h: rate,
            predicted_rate: None,
            next_funding_time,
            funding_interval,
            volume_24h: 1_000_000.0,
            timestamp: 1,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        }
    }
}

#[cfg(test)]
mod tests;
