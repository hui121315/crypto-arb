//! 套利引擎 V3：编排扫描器、归一化、风险、成本、仓位、计算器。
//!
//! 数据流：
//! ```text
//! OpportunityMarketDataSource::market_snapshot()
//!         │
//!         ▼
//!    Normalizer (EWMA + outlier；可选)
//!         │
//!         ▼
//!    Five registered product strategies scan one snapshot
//!         │
//!         ▼ Vec<RawOpportunity>
//!    ┌────────────────────────────────────────┐
//!    │ for each:                              │
//!    │   cost = strategy execution-cycle cost │
//!    │   net  = cost_model::net_single_yield  │
//!    │   pos  = position_optimizer            │
//!    │   dto  = OpportunityBuilder.build()    │
//!    └────────────────────────────────────────┘
//!         │
//!         ▼
//! Vec<ArbitrageOpportunityDto> (按可核验费后收益下限降序)
//! ```

use crate::algorithms::{
    cost_model, fee_evidence, leg_market_evidence, market_index::MarketScanIndex,
    perp_cross_policy, position_optimizer, price_spread_history::PriceSpreadHistoryCache,
};
use crate::calculator::OpportunityBuilder;
use crate::interfaces::{MarketDataSnapshot, OpportunityMarketDataSource};
use crate::models::{IndexCompositionRisk, OpportunityHistoryStats, RawOpportunity, RiskMetrics};
use crate::strategy_registry;
use shared_types::{
    normalized_venue_name, p0_hedge_leg_product, strategy_execution_cycle, ArbitrageConfig,
    ArbitrageOpportunityDto, ArbitrageType, FeeProduct, FundingDiffStatsRow,
    FundingDiffWindowStats, HedgeLegRole, IndexComponent, IndexCompositionEvidence,
    IndexCompositionQuality, IndexCompositionSnapshot, LegCostBreakdown, MarketDataQuality,
    MarketDataSnapshotOperation, MarketDataSnapshotStatusRow, MarketDataSourceKind,
    OpportunityDataCoverage, OpportunityLegMarketEvidence, OpportunityScanMeta,
    OpportunityScanOutcome, OpportunityScanReport, OrderSide, RoundTripCostBreakdown,
    StrategyExecutionCycle, StrategyKind, TradeFeeSnapshot,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::{cmp::Ordering, fmt};

const MAX_PERP_CROSS_PAIRS_PER_SYMBOL: usize = 6;

enum ScanCandidate {
    Emit(Box<ArbitrageOpportunityDto>),
    ExtremeYield,
    NonPositiveYield,
    UnprofitableAfterCost,
    BelowMinNetYield,
}

pub struct ArbitrageEngineV3 {
    pub config: ArbitrageConfig,
    pub data_source: Arc<dyn OpportunityMarketDataSource>,
    pub total_capital: f64,
    pub risk_tolerance: f64,
    price_spread_history: Arc<PriceSpreadHistoryCache>,
}

impl fmt::Debug for ArbitrageEngineV3 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArbitrageEngineV3")
            .field("config", &self.config)
            .field("total_capital", &self.total_capital)
            .field("risk_tolerance", &self.risk_tolerance)
            .finish_non_exhaustive()
    }
}

impl ArbitrageEngineV3 {
    pub fn new(
        config: ArbitrageConfig,
        data_source: Arc<dyn OpportunityMarketDataSource>,
        total_capital: f64,
        risk_tolerance: f64,
    ) -> Self {
        Self {
            config,
            data_source,
            total_capital,
            risk_tolerance,
            price_spread_history: Arc::new(PriceSpreadHistoryCache::default()),
        }
    }

    pub fn run_scan(&self) -> Vec<ArbitrageOpportunityDto> {
        self.run_scan_report().opportunities
    }

    pub fn run_scan_report(&self) -> OpportunityScanReport {
        let scan_start = std::time::Instant::now();
        let observed_at = chrono::Utc::now();
        let observed_at_ms = observed_at.timestamp_millis();
        let market = self.data_source.market_snapshot();
        let mut raw = self.scan_market_strategies(&market);
        leg_market_evidence::attach_leg_market_evidence(
            &mut raw,
            market.status.as_ref(),
            &market.funding_row_evidence,
            &market.perp_ticker_row_evidence,
            &market.spot_tick_row_evidence,
        );
        retain_perp_cross_frontier(
            &mut raw,
            self.config.default_slippage,
            self.config.min_net_yield,
        );
        attach_funding_diff_stats(&mut raw, &market.funding_diff_stats);
        attach_index_composition_risk(&mut raw, &market.index_compositions);
        self.price_spread_history
            .observe_candidates(&market, &raw, observed_at_ms);
        let mut meta = OpportunityScanMeta {
            candidate_count: raw.len(),
            coverage: data_coverage(&market),
            funding_row_evidence: market.funding_row_evidence.clone(),
            ..OpportunityScanMeta::default()
        };
        mark_market_data_problems(&market, &mut meta);

        let constraints = self.position_constraints();
        let mut opportunities = self.build_opportunities(raw, &constraints, &mut meta, observed_at);
        opportunities.sort_by(opportunity_profit_order);
        meta.emitted_count = opportunities.len();
        meta.scan_outcome = classify_scan_outcome(&meta);
        meta.scan_ms = scan_start.elapsed().as_millis() as u64;

        OpportunityScanReport {
            opportunities,
            meta,
        }
    }

    fn position_constraints(&self) -> position_optimizer::PositionConstraints {
        position_optimizer::PositionConstraints {
            max_position_ratio: self.config.max_position_ratio,
            max_exchange_ratio: self.config.max_exchange_ratio,
            max_symbol_ratio: self.config.max_symbol_ratio,
            // Wire-compatible config name; this is now a bounded allocation scale, not Kelly.
            allocation_scaling: self.config.kelly_scaling,
        }
    }

    fn build_opportunities(
        &self,
        raw: Vec<RawOpportunity>,
        constraints: &position_optimizer::PositionConstraints,
        meta: &mut OpportunityScanMeta,
        observed_at: chrono::DateTime<chrono::Utc>,
    ) -> Vec<ArbitrageOpportunityDto> {
        let mut out = Vec::with_capacity(raw.len());
        for r in raw {
            match self.evaluate_candidate(r, constraints, observed_at) {
                ScanCandidate::Emit(dto) => out.push(*dto),
                ScanCandidate::ExtremeYield => meta.dropped_extreme_yield_count += 1,
                ScanCandidate::NonPositiveYield => meta.dropped_non_positive_yield_count += 1,
                ScanCandidate::UnprofitableAfterCost => {
                    meta.dropped_unprofitable_after_cost_count += 1;
                }
                ScanCandidate::BelowMinNetYield => meta.dropped_below_min_net_yield_count += 1,
            }
        }
        out
    }

    fn evaluate_candidate(
        &self,
        mut r: RawOpportunity,
        constraints: &position_optimizer::PositionConstraints,
        observed_at: chrono::DateTime<chrono::Utc>,
    ) -> ScanCandidate {
        let scan_edge = strategy_registry::scan_edge(&r);
        // 防御性极值门必须读取策略真实单次收益，不能重新引入旧 8h 兼容字段。
        if scan_edge.abs()
            > strategy_registry::max_single_yield_for(&r, self.config.max_single_yield)
        {
            return ScanCandidate::ExtremeYield;
        }
        if scan_edge <= 0.0 || !scan_edge.is_finite() {
            return ScanCandidate::NonPositiveYield;
        }

        // 成本
        let observed_at_ms = observed_at.timestamp_millis();
        let cost =
            verified_row_round_trip_cost(&mut r, self.config.default_slippage, observed_at_ms);
        self.attach_price_spread_convergence(&mut r, &cost, observed_at_ms);

        // 周期数估算
        let min_periods = strategy_registry::min_holding_periods(&r, &cost);
        if min_periods == u32::MAX {
            return ScanCandidate::UnprofitableAfterCost;
        }
        let net_yield = strategy_registry::net_single_yield(&r, &cost, min_periods);
        if !net_yield.is_finite() || net_yield <= 0.0 || net_yield < self.config.min_net_yield {
            return ScanCandidate::BelowMinNetYield;
        }

        // Scan cadence is not settlement or fill history. Keep legacy metric fields neutral;
        // structural strategy risk is assigned by the calculator and realized risk by review.
        let metrics = RiskMetrics::default();

        // 仓位
        let position = position_optimizer::calculate_position(
            net_yield,
            self.total_capital,
            self.risk_tolerance,
            constraints,
        );

        // DTO
        ScanCandidate::Emit(Box::new(
            OpportunityBuilder {
                raw: &r,
                metrics: &metrics,
                position: &position,
                cost: &cost,
                min_holding_periods: min_periods,
                net_single_yield: net_yield,
                data_source: "market-data-cache",
                confidence: 0.85,
            }
            .build_at(observed_at),
        ))
    }

    fn scan_market_strategies(&self, market: &MarketDataSnapshot) -> Vec<RawOpportunity> {
        let index = MarketScanIndex::from_snapshot(market);
        let mut raw = Vec::new();
        for strategy in strategy_registry::p0_market_strategies() {
            let mut rows = strategy.scan(market, &index, &self.config);
            debug_assert!(rows
                .iter()
                .all(|row| row.extra.strategy_kind == Some(strategy.kind())));
            debug_assert!(rows.iter().all(|row| row.arb_type == strategy.arb_type()));
            raw.append(&mut rows);
        }
        raw
    }

    fn attach_price_spread_convergence(
        &self,
        opportunity: &mut RawOpportunity,
        cost: &crate::models::CostBreakdown,
        observed_at_ms: i64,
    ) {
        if opportunity.extra.strategy_kind != Some(StrategyKind::PerpPriceSpread) {
            return;
        }
        let evidence = self.price_spread_history.assess(
            opportunity,
            cost.round_trip_cost.max(0.0) * 10_000.0,
            observed_at_ms,
        );
        if let Some(blocker) =
            crate::algorithms::price_spread_history::convergence_blocker(evidence)
        {
            opportunity.extra.execution_blockers.push(blocker);
        }
        opportunity.extra.price_spread_convergence = Some(evidence);
    }
}

fn opportunity_profit_order(
    left: &ArbitrageOpportunityDto,
    right: &ArbitrageOpportunityDto,
) -> Ordering {
    right
        .execution_eligible
        .cmp(&left.execution_eligible)
        .then_with(|| opportunity_profit_floor(right).total_cmp(&opportunity_profit_floor(left)))
        .then_with(|| right.net_single_yield.total_cmp(&left.net_single_yield))
        .then_with(|| left.id.cmp(&right.id))
}

fn opportunity_profit_floor(row: &ArbitrageOpportunityDto) -> f64 {
    row.execution_cost
        .as_ref()
        .filter(|cost| cost.one_cycle.covers_round_trip_cost)
        .filter(|cost| {
            cost.round_trip.as_ref().is_some_and(|round_trip| {
                round_trip.profitability_evidence.is_cost_verified()
                    && round_trip.profitability_evidence.fee_evidence_ids.len() >= 2
            })
        })
        .map(|cost| cost.one_cycle.net_bps)
        .filter(|value| value.is_finite())
        .unwrap_or(f64::NEG_INFINITY)
}

#[derive(Debug)]
struct RankedPerpCross {
    symbol: String,
    rank: PerpCrossFrontierRank,
    row: RawOpportunity,
}

#[derive(Debug, Clone, Copy)]
struct PerpCrossFrontierRank {
    preflight_ready: bool,
    alignment_priority: u8,
    fee_verified: bool,
    fresh_ws_prices: bool,
    net_bps: f64,
    gross_bps: f64,
    min_volume_24h: f64,
}

fn retain_perp_cross_frontier(
    rows: &mut Vec<RawOpportunity>,
    default_slippage: f64,
    min_net_yield: f64,
) {
    let mut retained = Vec::with_capacity(rows.len());
    let mut ranked = Vec::new();
    for row in std::mem::take(rows) {
        if is_perp_cross_row(&row) {
            ranked.push(RankedPerpCross {
                symbol: row.symbol.to_ascii_uppercase(),
                rank: perp_cross_frontier_rank(&row, default_slippage, min_net_yield),
                row,
            });
        } else {
            retained.push(row);
        }
    }

    ranked.sort_by(|left, right| {
        left.symbol
            .cmp(&right.symbol)
            .then_with(|| compare_perp_cross_frontier(&left.rank, &right.rank))
            .then_with(|| left.row.long_exchange.cmp(&right.row.long_exchange))
            .then_with(|| left.row.short_exchange.cmp(&right.row.short_exchange))
    });

    let mut kept_by_symbol = HashMap::<String, usize>::new();
    for candidate in ranked {
        let kept = kept_by_symbol.entry(candidate.symbol).or_default();
        if *kept < MAX_PERP_CROSS_PAIRS_PER_SYMBOL {
            retained.push(candidate.row);
            *kept += 1;
        }
    }
    *rows = retained;
}

fn perp_cross_frontier_rank(
    row: &RawOpportunity,
    default_slippage: f64,
    min_net_yield: f64,
) -> PerpCrossFrontierRank {
    let cost = standard_round_trip_cost(
        row,
        default_slippage,
        StrategyExecutionCycle::PairedOpenClose,
    );
    let net_bps = cost
        .map(|round_trip_cost| (row.single_yield - round_trip_cost) * 10_000.0)
        .filter(|value| value.is_finite());
    let fresh_ws_prices = fresh_ws_evidence(row.extra.long_leg_market_evidence.as_ref())
        && fresh_ws_evidence(row.extra.short_leg_market_evidence.as_ref());
    let alignment = perp_cross_policy::evaluate(perp_cross_policy::PriceAlignmentInput {
        long_open_price: row.extra.long_price,
        short_open_price: row.extra.short_price,
        long_observed_at_ms: evidence_observed_at(row.extra.long_leg_market_evidence.as_ref()),
        short_observed_at_ms: evidence_observed_at(row.extra.short_leg_market_evidence.as_ref()),
        projected_net_funding_bps: net_bps,
    });
    let min_net_bps = finite_non_negative(min_net_yield) * 10_000.0;
    let positive_net = net_bps.is_some_and(|value| value > 0.0 && value >= min_net_bps);

    PerpCrossFrontierRank {
        preflight_ready: cost.is_some() && fresh_ws_prices && positive_net && alignment.passed,
        alignment_priority: alignment_priority(alignment.class),
        fee_verified: cost.is_some(),
        fresh_ws_prices,
        net_bps: net_bps.unwrap_or(f64::NEG_INFINITY),
        gross_bps: row.single_yield * 10_000.0,
        min_volume_24h: row.long_rate.volume_24h.min(row.short_rate.volume_24h),
    }
}

fn compare_perp_cross_frontier(
    left: &PerpCrossFrontierRank,
    right: &PerpCrossFrontierRank,
) -> Ordering {
    right
        .preflight_ready
        .cmp(&left.preflight_ready)
        .then_with(|| right.alignment_priority.cmp(&left.alignment_priority))
        .then_with(|| right.fee_verified.cmp(&left.fee_verified))
        .then_with(|| right.fresh_ws_prices.cmp(&left.fresh_ws_prices))
        .then_with(|| right.net_bps.total_cmp(&left.net_bps))
        .then_with(|| right.gross_bps.total_cmp(&left.gross_bps))
        .then_with(|| right.min_volume_24h.total_cmp(&left.min_volume_24h))
}

const fn alignment_priority(class: perp_cross_policy::PriceAlignmentClass) -> u8 {
    match class {
        perp_cross_policy::PriceAlignmentClass::Aligned => 3,
        perp_cross_policy::PriceAlignmentClass::Unproven => 2,
        perp_cross_policy::PriceAlignmentClass::HybridSpread => 1,
        perp_cross_policy::PriceAlignmentClass::Invalid => 0,
    }
}

fn fresh_ws_evidence(evidence: Option<&OpportunityLegMarketEvidence>) -> bool {
    evidence.is_some_and(|evidence| {
        evidence.health.quality == MarketDataQuality::Fresh
            && evidence.health.source == MarketDataSourceKind::WsPush
            && evidence.health.observed_at_ms > 0
    })
}

fn evidence_observed_at(evidence: Option<&OpportunityLegMarketEvidence>) -> i64 {
    evidence.map_or(0, |evidence| evidence.health.observed_at_ms)
}

fn is_perp_cross_row(row: &RawOpportunity) -> bool {
    row.extra.strategy_kind == Some(StrategyKind::PerpCross)
        || row.extra.strategy_kind.is_none() && row.arb_type == ArbitrageType::CrossExchange
}

fn verified_row_round_trip_cost(
    row: &mut RawOpportunity,
    default_slippage: f64,
    observed_at_ms: i64,
) -> crate::models::CostBreakdown {
    let cycle = strategy_execution_cycle(row.extra.strategy_kind);
    if let Some((long_fee, short_fee)) = row_fee_snapshots(row, observed_at_ms) {
        let cost = cost_from_fee_snapshots(&long_fee, &short_fee, default_slippage, cycle);
        row.extra.cost_round_trip = Some(round_trip_breakdown(
            &long_fee,
            &short_fee,
            default_slippage,
            cost.round_trip_cost,
            cycle,
        ));
        cost
    } else {
        push_cost_evidence_blocker(row);
        row_round_trip_cost(row, default_slippage)
    }
}

fn row_round_trip_cost(
    row: &RawOpportunity,
    default_slippage: f64,
) -> crate::models::CostBreakdown {
    cost_model::execution_cycle_cost_for_legs(
        row_cost_context(row, OrderSide::Buy),
        row_cost_context(row, OrderSide::Sell),
        finite_non_negative(default_slippage),
        0.5,
        strategy_execution_cycle(row.extra.strategy_kind),
    )
}

fn row_fee_snapshots(
    row: &RawOpportunity,
    observed_at_ms: i64,
) -> Option<(TradeFeeSnapshot, TradeFeeSnapshot)> {
    let long = row_fee_snapshot(row, OrderSide::Buy, observed_at_ms)?;
    let short = row_fee_snapshot(row, OrderSide::Sell, observed_at_ms)?;
    Some((long, short))
}

fn standard_round_trip_cost(
    row: &RawOpportunity,
    default_slippage: f64,
    cycle: StrategyExecutionCycle,
) -> Option<f64> {
    let long_fee = fee_evidence::standard_taker_fee_bps(
        row_exchange(row, OrderSide::Buy),
        row_fee_product(row, OrderSide::Buy),
    )?;
    let short_fee = fee_evidence::standard_taker_fee_bps(
        row_exchange(row, OrderSide::Sell),
        row_fee_product(row, OrderSide::Sell),
    )?;
    let (fee_cycles, slippage_legs) = match cycle {
        StrategyExecutionCycle::PairedOpenClose => (2.0, 4.0),
        StrategyExecutionCycle::PairedOpenRebalance => (1.0, 2.0),
    };
    let fee_bps = (long_fee.max(0.0) + short_fee.max(0.0)) * fee_cycles;
    Some(fee_bps / 10_000.0 + finite_non_negative(default_slippage) * slippage_legs)
}

fn row_fee_snapshot(
    row: &RawOpportunity,
    side: OrderSide,
    now_ms: i64,
) -> Option<TradeFeeSnapshot> {
    fee_evidence::standard_fee_snapshot(
        row_exchange(row, side),
        row_fee_symbol(row, side),
        row_fee_product(row, side),
        false,
        now_ms,
    )
}

fn row_fee_product(row: &RawOpportunity, side: OrderSide) -> FeeProduct {
    if row_is_spot(row, side) {
        FeeProduct::Spot
    } else {
        FeeProduct::Perp
    }
}

fn row_exchange(row: &RawOpportunity, side: OrderSide) -> &str {
    match side {
        OrderSide::Buy => &row.long_exchange,
        OrderSide::Sell => &row.short_exchange,
    }
}

fn cost_from_fee_snapshots(
    long: &TradeFeeSnapshot,
    short: &TradeFeeSnapshot,
    default_slippage: f64,
    cycle: StrategyExecutionCycle,
) -> crate::models::CostBreakdown {
    let slippage = finite_non_negative(default_slippage);
    let open_fee_bps = long.open_fee_bps + short.open_fee_bps;
    let close_fee_bps = match cycle {
        StrategyExecutionCycle::PairedOpenClose => long.close_fee_bps + short.close_fee_bps,
        StrategyExecutionCycle::PairedOpenRebalance => 0.0,
    };
    let fee_bps = open_fee_bps + close_fee_bps;
    let slippage_legs = match cycle {
        StrategyExecutionCycle::PairedOpenClose => 4.0,
        StrategyExecutionCycle::PairedOpenRebalance => 2.0,
    };
    let trade_count = slippage_legs;
    let round_trip_cost = fee_bps / 10_000.0 + slippage * slippage_legs;
    crate::models::CostBreakdown {
        fee_rate: fee_bps / (trade_count * 10_000.0),
        slippage,
        round_trip_cost,
    }
}

fn round_trip_breakdown(
    long: &TradeFeeSnapshot,
    short: &TradeFeeSnapshot,
    default_slippage: f64,
    round_trip_cost: f64,
    cycle: StrategyExecutionCycle,
) -> RoundTripCostBreakdown {
    let leg_slippage_bps = finite_non_negative(default_slippage) * 10_000.0;
    let open_slippage_bps = leg_slippage_bps * 2.0;
    let close_leg_slippage_bps = match cycle {
        StrategyExecutionCycle::PairedOpenClose => leg_slippage_bps,
        StrategyExecutionCycle::PairedOpenRebalance => 0.0,
    };
    let close_slippage_bps = close_leg_slippage_bps * 2.0;
    let open_fee_bps = long.open_fee_bps + short.open_fee_bps;
    let close_fee_bps = match cycle {
        StrategyExecutionCycle::PairedOpenClose => long.close_fee_bps + short.close_fee_bps,
        StrategyExecutionCycle::PairedOpenRebalance => 0.0,
    };
    RoundTripCostBreakdown {
        long_leg: leg_cost_breakdown(
            HedgeLegRole::Long,
            long,
            leg_slippage_bps,
            close_leg_slippage_bps,
            cycle,
        ),
        short_leg: leg_cost_breakdown(
            HedgeLegRole::Short,
            short,
            leg_slippage_bps,
            close_leg_slippage_bps,
            cycle,
        ),
        open_fee_bps,
        close_fee_bps,
        open_slippage_bps,
        close_slippage_bps,
        borrow_or_financing_bps: 0.0,
        funding_window_mismatch_buffer_bps: 0.0,
        min_profit_buffer_bps: 0.0,
        total_cost_bps: round_trip_cost.max(0.0) * 10_000.0,
        one_cycle_net_bps: 0.0,
        profitability_evidence: Default::default(),
    }
}

fn leg_cost_breakdown(
    role: HedgeLegRole,
    fee: &TradeFeeSnapshot,
    open_slippage_bps: f64,
    close_slippage_bps: f64,
    cycle: StrategyExecutionCycle,
) -> LegCostBreakdown {
    LegCostBreakdown {
        role,
        venue: fee.venue.clone(),
        symbol: fee.symbol.clone(),
        product: fee.product,
        open_fee_bps: fee.open_fee_bps,
        close_fee_bps: match cycle {
            StrategyExecutionCycle::PairedOpenClose => fee.close_fee_bps,
            StrategyExecutionCycle::PairedOpenRebalance => 0.0,
        },
        open_slippage_bps,
        close_slippage_bps,
        fee_snapshot: Some(fee.clone()),
    }
}

fn push_cost_evidence_blocker(row: &mut RawOpportunity) {
    let message = format!(
        "{} {} / {} 成本缺少双腿官方费率证据，仅观察不执行",
        row.symbol, row.long_exchange, row.short_exchange
    );
    if !row.extra.execution_blockers.contains(&message) {
        row.extra.execution_blockers.push(message);
    }
    row.extra.cost_round_trip = None;
}

fn row_cost_context(row: &RawOpportunity, side: OrderSide) -> cost_model::LegCostContext<'_> {
    cost_model::LegCostContext {
        exchange: match side {
            OrderSide::Buy => &row.long_exchange,
            OrderSide::Sell => &row.short_exchange,
        },
        product_type: row_cost_product(row, side),
    }
}

fn row_cost_product(row: &RawOpportunity, side: OrderSide) -> cost_model::ProductType {
    if row_is_spot(row, side) {
        cost_model::ProductType::Spot
    } else {
        cost_model::ProductType::Perp
    }
}

fn finite_non_negative(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn row_fee_symbol(row: &RawOpportunity, side: OrderSide) -> &str {
    match side {
        OrderSide::Buy => row
            .extra
            .long_depth_symbol
            .as_deref()
            .unwrap_or(&row.symbol),
        OrderSide::Sell => row
            .extra
            .short_depth_symbol
            .as_deref()
            .unwrap_or(&row.symbol),
    }
}

fn row_is_spot(row: &RawOpportunity, side: OrderSide) -> bool {
    let role = match side {
        OrderSide::Buy => HedgeLegRole::Long,
        OrderSide::Sell => HedgeLegRole::Short,
    };
    match p0_hedge_leg_product(row.extra.strategy_kind, row.extra.spot_leg_mode, role) {
        Some(FeeProduct::Spot) => return true,
        Some(FeeProduct::Perp) => return false,
        Some(FeeProduct::Margin | FeeProduct::Unknown) | None => {}
    }

    // Legacy rows without a typed P0 strategy retain their historical inference.
    match (row.arb_type, side) {
        (ArbitrageType::SpotCross, OrderSide::Buy | OrderSide::Sell) => true,
        (ArbitrageType::SpotFutures | ArbitrageType::CrossSpotFutures, OrderSide::Buy)
            if row.extra.long_depth_symbol.is_some() =>
        {
            true
        }
        (ArbitrageType::SpotFutures | ArbitrageType::CrossSpotFutures, OrderSide::Sell)
            if row.extra.short_depth_symbol.is_some() =>
        {
            true
        }
        _ => false,
    }
}

fn attach_funding_diff_stats(rows: &mut [RawOpportunity], stats: &[FundingDiffStatsRow]) {
    if stats.is_empty() {
        return;
    }
    let index = funding_diff_stats_index(stats);
    for row in rows {
        row.extra.history_stats = index.get(&funding_stats_key(row)).cloned();
    }
}

fn attach_index_composition_risk(
    rows: &mut [RawOpportunity],
    compositions: &[IndexCompositionSnapshot],
) {
    if compositions.is_empty() {
        return;
    }
    let index = index_composition_index(compositions);
    for row in rows {
        let long = index.get(&composition_key(&row.long_exchange, &row.symbol));
        let short = index.get(&composition_key(&row.short_exchange, &row.symbol));
        let Some(risk) = composition_risk(row, long.copied(), short.copied()) else {
            continue;
        };
        row.extra.index_composition = Some(risk);
    }
}

type IndexCompositionIndex<'a> = HashMap<(String, String), &'a IndexCompositionSnapshot>;

fn index_composition_index(compositions: &[IndexCompositionSnapshot]) -> IndexCompositionIndex<'_> {
    let mut out = HashMap::with_capacity(compositions.len());
    for row in compositions {
        out.insert(composition_key(&row.venue, &row.symbol), row);
    }
    out
}

fn composition_key(venue: &str, symbol: &str) -> (String, String) {
    (
        normalized_venue_name(venue),
        exchange::strip_common_suffixes(symbol).to_ascii_uppercase(),
    )
}

fn composition_risk(
    row: &RawOpportunity,
    long: Option<&IndexCompositionSnapshot>,
    short: Option<&IndexCompositionSnapshot>,
) -> Option<IndexCompositionRisk> {
    if long.is_none() && short.is_none() {
        return None;
    }
    let long_quality = long
        .map(|snapshot| snapshot.quality)
        .unwrap_or(IndexCompositionQuality::Unverified);
    let short_quality = short
        .map(|snapshot| snapshot.quality)
        .unwrap_or(IndexCompositionQuality::Unverified);
    let overlap_score = match (long, short) {
        (Some(a), Some(b))
            if a.quality == IndexCompositionQuality::Verified
                && b.quality == IndexCompositionQuality::Verified =>
        {
            composition_overlap_score(&a.components, &b.components)
        }
        _ => 0.0,
    };
    let hidden_price = long.is_some_and(has_hidden_component_price)
        || short.is_some_and(has_hidden_component_price);
    let missing_payload_evidence =
        long.is_some_and(lacks_payload_evidence) || short.is_some_and(lacks_payload_evidence);
    let blocker = composition_blocker(
        row,
        long_quality,
        short_quality,
        overlap_score,
        hidden_price,
        missing_payload_evidence,
    );
    Some(IndexCompositionRisk {
        overlap_score,
        long_quality,
        short_quality,
        hidden_price,
        blocker,
        long_evidence: long.map(composition_evidence),
        short_evidence: short.map(composition_evidence),
    })
}

fn composition_evidence(snapshot: &IndexCompositionSnapshot) -> IndexCompositionEvidence {
    IndexCompositionEvidence {
        source: snapshot.source.clone(),
        received_at_ms: snapshot.received_at_ms,
        freshness_ms: snapshot.freshness_ms,
        source_url: snapshot.source_url.clone(),
        payload_sha256: snapshot.payload_sha256.clone(),
        schema_version: snapshot.schema_version.clone(),
    }
}

fn has_hidden_component_price(snapshot: &IndexCompositionSnapshot) -> bool {
    snapshot.components.iter().any(|item| item.price.is_none())
}

/// Verified 快照必须携带官方 payload 证据（`source_url` + sha256）才能进执行。
fn lacks_payload_evidence(snapshot: &IndexCompositionSnapshot) -> bool {
    snapshot.quality == IndexCompositionQuality::Verified
        && (snapshot.source_url.is_none() || snapshot.payload_sha256.is_none())
}

fn composition_blocker(
    row: &RawOpportunity,
    long_quality: IndexCompositionQuality,
    short_quality: IndexCompositionQuality,
    overlap_score: f64,
    hidden_price: bool,
    missing_payload_evidence: bool,
) -> Option<String> {
    if long_quality != IndexCompositionQuality::Verified
        || short_quality != IndexCompositionQuality::Verified
    {
        return Some(format!(
            "{} 双边指数成分未完成验证，不能确认底层一致",
            row.symbol
        ));
    }
    if overlap_score < 0.75 {
        return Some(format!(
            "{} 双边指数成分重合度 {:.0}% 低于 75%，不能直接构建对冲",
            row.symbol,
            overlap_score * 100.0
        ));
    }
    if hidden_price {
        return Some(format!(
            "{} 指数成分存在隐藏价格，成分价格不可审计，仅观察不执行",
            row.symbol
        ));
    }
    if missing_payload_evidence {
        return Some(format!(
            "{} 指数成分缺官方 payload 证据（source_url/sha256），仅观察不执行",
            row.symbol
        ));
    }
    None
}

fn composition_overlap_score(long: &[IndexComponent], short: &[IndexComponent]) -> f64 {
    if long.is_empty() || short.is_empty() {
        return 0.0;
    }
    let long_weights = normalized_component_weights(long);
    let short_weights = normalized_component_weights(short);
    long_weights
        .iter()
        .filter_map(|(key, weight)| short_weights.get(key).map(|other| (*weight).min(*other)))
        .sum::<f64>()
        .clamp(0.0, 1.0)
}

fn normalized_component_weights(components: &[IndexComponent]) -> HashMap<String, f64> {
    let mut weights = HashMap::with_capacity(components.len());
    let positive_weight_sum: f64 = components
        .iter()
        .map(|item| item.weight.max(0.0))
        .filter(|value| value.is_finite())
        .sum();
    let fallback_weight = 1.0 / components.len() as f64;
    for component in components {
        let weight = if positive_weight_sum > f64::EPSILON {
            component.weight.max(0.0) / positive_weight_sum
        } else {
            fallback_weight
        };
        *weights.entry(component_key_label(component)).or_insert(0.0) += weight;
    }
    weights
}

fn component_key_label(component: &IndexComponent) -> String {
    format!(
        "{}:{}",
        normalized_venue_name(&component.name),
        exchange::strip_common_suffixes(&component.symbol)
    )
    .to_ascii_uppercase()
}

type FundingStatsIndex = HashMap<(String, String, String), OpportunityHistoryStats>;

fn funding_diff_stats_index(stats: &[FundingDiffStatsRow]) -> FundingStatsIndex {
    let mut out = HashMap::with_capacity(stats.len());
    for row in stats {
        let Some(history_stats) = opportunity_history_stats(&row.windows) else {
            continue;
        };
        out.insert(stats_row_key(row), history_stats);
    }
    out
}

fn longest_stats_window(windows: &[FundingDiffWindowStats]) -> Option<&FundingDiffWindowStats> {
    windows.iter().max_by_key(|window| window.cycles)
}

fn opportunity_history_stats(
    windows: &[FundingDiffWindowStats],
) -> Option<OpportunityHistoryStats> {
    let window = longest_stats_window(windows)?;
    let mut windows = windows.to_vec();
    windows.sort_by_key(|window| window.cycles);
    Some(OpportunityHistoryStats {
        window: window.clone(),
        windows,
    })
}

fn stats_row_key(row: &FundingDiffStatsRow) -> (String, String, String) {
    (
        row.symbol.to_ascii_uppercase(),
        normalized_venue_name(&row.long_exchange),
        normalized_venue_name(&row.short_exchange),
    )
}

fn funding_stats_key(row: &RawOpportunity) -> (String, String, String) {
    (
        row.symbol.to_ascii_uppercase(),
        normalized_venue_name(&row.long_exchange),
        normalized_venue_name(&row.short_exchange),
    )
}

fn data_coverage(market: &MarketDataSnapshot) -> OpportunityDataCoverage {
    let mut venues = HashSet::new();
    let funding_rows = market
        .funding
        .values()
        .map(|by_exchange| {
            venues.extend(by_exchange.keys().map(|venue| normalized_venue_name(venue)));
            by_exchange.len()
        })
        .sum();
    OpportunityDataCoverage {
        funding_symbols: market.funding.len(),
        funding_venues: venues.len(),
        funding_rows,
        perp_tickers: market.perp_tickers.len(),
        spot_ticks: market.spot_ticks.len(),
        index_compositions: market.index_compositions.len(),
    }
}

fn mark_market_data_problems(market: &MarketDataSnapshot, meta: &mut OpportunityScanMeta) {
    meta.market_data_status = market.status.clone();
    if let Some(status) = market.status.as_ref() {
        let degraded = status
            .rows
            .iter()
            .filter(|row| is_scan_market_problem(row))
            .map(market_status_label)
            .collect::<Vec<_>>();
        if !degraded.is_empty() {
            meta.market_data_problem_count = degraded.len();
            meta.degraded_venues = degraded;
            return;
        }
    }
    mark_market_data_coverage_problems(meta);
}

fn mark_market_data_coverage_problems(meta: &mut OpportunityScanMeta) {
    let mut degraded = Vec::new();
    if meta.coverage.funding_rows == 0 {
        degraded.push("funding");
    }
    if meta.coverage.perp_tickers == 0 {
        degraded.push("perp_tickers");
    }
    if meta.coverage.spot_ticks == 0 {
        degraded.push("spot_ticks");
    }
    meta.market_data_problem_count = degraded.len();
    meta.degraded_venues = degraded.into_iter().map(str::to_owned).collect();
}

fn classify_scan_outcome(meta: &OpportunityScanMeta) -> OpportunityScanOutcome {
    if meta.emitted_count > 0 {
        return OpportunityScanOutcome::Found;
    }
    if has_warming_scan_input(meta) {
        return OpportunityScanOutcome::Warming;
    }
    if has_partial_scan_input(meta) {
        return OpportunityScanOutcome::PartialUpstream;
    }
    if meta.candidate_count > 0 || dropped_candidate_count(meta) > 0 {
        return OpportunityScanOutcome::FilteredEmpty;
    }
    OpportunityScanOutcome::TrueEmpty
}

fn dropped_candidate_count(meta: &OpportunityScanMeta) -> usize {
    meta.dropped_extreme_yield_count
        + meta.dropped_non_positive_yield_count
        + meta.dropped_unprofitable_after_cost_count
        + meta.dropped_below_min_net_yield_count
}

fn has_warming_scan_input(meta: &OpportunityScanMeta) -> bool {
    if let Some(status) = meta.market_data_status.as_ref() {
        return status.rows.iter().any(is_warming_scan_status_row);
    }
    missing_required_coverage(&meta.coverage)
}

fn has_partial_scan_input(meta: &OpportunityScanMeta) -> bool {
    if let Some(status) = meta.market_data_status.as_ref() {
        return status
            .rows
            .iter()
            .any(|row| is_scan_market_problem(row) && !is_warming_scan_status_row(row));
    }
    meta.market_data_problem_count > 0 && !missing_required_coverage(&meta.coverage)
}

fn missing_required_coverage(coverage: &OpportunityDataCoverage) -> bool {
    coverage.funding_rows == 0 || coverage.perp_tickers == 0 || coverage.spot_ticks == 0
}

fn is_warming_scan_status_row(row: &MarketDataSnapshotStatusRow) -> bool {
    is_required_scan_operation(row.operation)
        && row.health.quality == MarketDataQuality::Missing
        && row
            .health
            .coverage
            .as_ref()
            .is_none_or(|coverage| coverage.received == 0)
}

fn is_required_scan_operation(operation: MarketDataSnapshotOperation) -> bool {
    matches!(
        operation,
        MarketDataSnapshotOperation::FundingRates
            | MarketDataSnapshotOperation::PerpTickers
            | MarketDataSnapshotOperation::SpotTicks
    )
}

fn is_scan_market_problem(row: &MarketDataSnapshotStatusRow) -> bool {
    row.health.quality != MarketDataQuality::Fresh && is_required_scan_operation(row.operation)
}

fn market_status_label(row: &MarketDataSnapshotStatusRow) -> String {
    let coverage = row
        .health
        .coverage
        .as_ref()
        .map(|coverage| format!(" {}/{}", coverage.received, coverage.requested))
        .unwrap_or_default();
    format!(
        "{}:{}={}{coverage}",
        row.venue,
        row.operation.as_str(),
        market_quality_label(row.health.quality)
    )
}

fn market_quality_label(quality: MarketDataQuality) -> &'static str {
    match quality {
        MarketDataQuality::Fresh => "fresh",
        MarketDataQuality::StaleAllowed => "stale_allowed",
        MarketDataQuality::StaleBlocked => "stale_blocked",
        MarketDataQuality::Missing => "missing",
        MarketDataQuality::RateLimited => "rate_limited",
        MarketDataQuality::CircuitOpen => "circuit_open",
        MarketDataQuality::Unsupported => "unsupported",
        MarketDataQuality::Unverified => "unverified",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interfaces::OpportunityMarketDataSource;
    use rust_decimal_macros::dec;
    use shared_types::{
        FundingDiffSampleHealth, FundingRateData, IndexComponent, IndexCompositionQuality,
        IndexCompositionSnapshot, MarketDataCoverage, MarketDataHealth, MarketDataRowEvidence,
        MarketDataSnapshotStatus, MarketDataSourceKind, SpotTick, TickerInfo,
    };
    use std::collections::HashMap;

    struct MockSource;

    impl OpportunityMarketDataSource for MockSource {
        fn market_snapshot(&self) -> MarketDataSnapshot {
            let now_ms = common::time::now_ms();
            let mk = |ex: &str, rate: f64| FundingRateData {
                symbol: "BTC".into(),
                exchange: ex.into(),
                rate,
                rate_8h: rate,
                predicted_rate: None,
                next_funding_time: now_ms + 8 * 3_600_000,
                funding_interval: 8,
                volume_24h: 1_500_000_000.0,
                timestamp: now_ms,
                smoothed_rate: None,
                rate_std: None,
                is_outlier: false,
            };
            let mut m: HashMap<String, HashMap<String, FundingRateData>> = HashMap::new();
            let inner = m.entry("BTC".into()).or_default();
            inner.insert("binance".into(), mk("binance", 0.0001));
            inner.insert("okx".into(), mk("okx", 0.0030));
            inner.insert("bybit".into(), mk("bybit", 0.0005));
            // ETH 无价差对（同费率）→ 应被过滤
            let inner_eth = m.entry("ETH".into()).or_default();
            inner_eth.insert("binance".into(), mk("binance", 0.0001));
            inner_eth.insert("okx".into(), mk("okx", 0.0001));
            MarketDataSnapshot {
                funding: Arc::new(m),
                perp_tickers: vec![
                    perp_with_spread("binance", "BTC", 100.0, 100.0),
                    perp_with_spread("okx", "BTC", 100.0, 100.0),
                    perp_with_spread("bybit", "BTC", 100.0, 100.0),
                    perp_with_spread("binance", "ETH", 100.0, 100.0),
                    perp_with_spread("okx", "ETH", 100.0, 100.0),
                ]
                .into(),
                ..Default::default()
            }
        }
    }

    struct MockMarketSource;

    struct MockMuSource;

    struct MockPerpPriceSource;

    struct MockSpotPerpSource;

    struct MockIndexCompositionSource;

    struct EmptyMarketSource;

    struct DegradedMarketStatusSource;

    struct HealthyFlatMarketSource;

    struct HealthyOpportunitySource;

    impl OpportunityMarketDataSource for EmptyMarketSource {
        fn market_snapshot(&self) -> MarketDataSnapshot {
            MarketDataSnapshot::default()
        }
    }

    impl OpportunityMarketDataSource for DegradedMarketStatusSource {
        fn market_snapshot(&self) -> MarketDataSnapshot {
            MarketDataSnapshot {
                status: Some(MarketDataSnapshotStatus {
                    observed_at_ms: 1,
                    rows: vec![status_row(
                        "bybit",
                        MarketDataSnapshotOperation::PerpTickers,
                        MarketDataQuality::RateLimited,
                    )],
                }),
                ..Default::default()
            }
        }
    }

    impl OpportunityMarketDataSource for HealthyFlatMarketSource {
        fn market_snapshot(&self) -> MarketDataSnapshot {
            MarketDataSnapshot {
                funding: group_rates(vec![
                    rate_with_volume("binance", "BTC", 0.0001, 2_000_000.0),
                    rate_with_volume("okx", "BTC", 0.0001, 2_000_000.0),
                ])
                .into(),
                perp_tickers: vec![
                    perp_with_spread("binance", "BTC", 100.0, 100.1),
                    perp_with_spread("okx", "BTC", 100.0, 100.1),
                ]
                .into(),
                spot_ticks: vec![
                    spot("binance", "BTC/USDT", 100.0),
                    spot("okx", "BTC/USDT", 100.0),
                ]
                .into(),
                status: Some(fresh_scan_status()),
                ..Default::default()
            }
        }
    }

    impl OpportunityMarketDataSource for HealthyOpportunitySource {
        fn market_snapshot(&self) -> MarketDataSnapshot {
            MarketDataSnapshot {
                funding: group_rates(vec![
                    rate_with_volume("binance", "BTC", 0.0001, 2_000_000.0),
                    rate_with_volume("okx", "BTC", 0.0010, 2_000_000.0),
                ])
                .into(),
                perp_tickers: vec![
                    perp_with_spread("binance", "BTC", 100.0, 100.1),
                    perp_with_spread("okx", "BTC", 100.0, 100.1),
                ]
                .into(),
                spot_ticks: vec![
                    spot("binance", "BTC/USDT", 100.0),
                    spot("okx", "BTC/USDT", 100.0),
                ]
                .into(),
                status: Some(fresh_scan_status()),
                ..Default::default()
            }
        }
    }

    impl OpportunityMarketDataSource for MockMarketSource {
        fn market_snapshot(&self) -> MarketDataSnapshot {
            MarketDataSnapshot {
                funding: group_rates(vec![rate("binance", "BTC", 0.0001)]).into(),
                funding_row_evidence: vec![row_evidence(
                    "binance",
                    "BTC",
                    MarketDataSnapshotOperation::FundingRates,
                    MarketDataQuality::Fresh,
                    MarketDataSourceKind::WsPush,
                )],
                perp_tickers: vec![perp("binance", "BTC", 101.0)].into(),
                perp_ticker_row_evidence: vec![row_evidence(
                    "binance",
                    "BTC",
                    MarketDataSnapshotOperation::PerpTickers,
                    MarketDataQuality::Fresh,
                    MarketDataSourceKind::WsPush,
                )],
                spot_ticks: vec![
                    spot("binance", "BTC/USDT", 100.0),
                    spot("okx", "BTC/USDT", 99.0),
                ]
                .into(),
                spot_tick_row_evidence: vec![
                    row_evidence(
                        "binance",
                        "BTC/USDT",
                        MarketDataSnapshotOperation::SpotTicks,
                        MarketDataQuality::Fresh,
                        MarketDataSourceKind::WsPush,
                    ),
                    row_evidence(
                        "okx",
                        "BTC/USDT",
                        MarketDataSnapshotOperation::SpotTicks,
                        MarketDataQuality::Fresh,
                        MarketDataSourceKind::WsPush,
                    ),
                ],
                status: Some(MarketDataSnapshotStatus {
                    observed_at_ms: 1,
                    rows: vec![
                        status_row(
                            "binance",
                            MarketDataSnapshotOperation::PerpTickers,
                            MarketDataQuality::Fresh,
                        ),
                        status_row(
                            "binance",
                            MarketDataSnapshotOperation::SpotTicks,
                            MarketDataQuality::Fresh,
                        ),
                        status_row(
                            "okx",
                            MarketDataSnapshotOperation::SpotTicks,
                            MarketDataQuality::StaleAllowed,
                        ),
                    ],
                }),
                ..Default::default()
            }
        }
    }

    impl OpportunityMarketDataSource for MockMuSource {
        fn market_snapshot(&self) -> MarketDataSnapshot {
            MarketDataSnapshot {
                funding: group_rates(vec![
                    rate_with_volume("hyperliquid:km", "MU", 0.000_005_707_8, 153_276.0),
                    rate_with_volume("hyperliquid:xyz", "MU", 0.000_027_958_7, 187_034_130.0),
                    rate_with_volume("bybit", "MU", 0.000_658_82, 5_522_527.0),
                    rate_with_volume("kucoin", "MU", 0.006, 831_918.0),
                ])
                .into(),
                perp_tickers: vec![
                    perp_with_spread("hyperliquid:km", "MU", 100.0, 100.0),
                    perp_with_spread("hyperliquid:xyz", "MU", 100.0, 100.0),
                    perp_with_spread("bybit", "MU", 100.0, 100.0),
                    perp_with_spread("kucoin", "MU", 100.0, 100.0),
                ]
                .into(),
                spot_ticks: vec![SpotTick {
                    venue: "kucoin".into(),
                    symbol: "USDT/USDC".into(),
                    bid: dec!(1),
                    ask: dec!(1),
                    last: dec!(1),
                    bid_size: Some(dec!(1000000)),
                    ask_size: Some(dec!(1000000)),
                    volume_24h: dec!(1000000),
                    exchange_ts_ms: Some(1),
                    received_at_ms: 1,
                }]
                .into(),
                ..Default::default()
            }
        }
    }

    impl OpportunityMarketDataSource for MockPerpPriceSource {
        fn market_snapshot(&self) -> MarketDataSnapshot {
            MarketDataSnapshot {
                funding: group_rates(vec![
                    rate_with_volume("binance", "MU", 0.000_01, 2_000_000.0),
                    rate_with_volume("kucoin", "MU", 0.004, 2_000_000.0),
                ])
                .into(),
                perp_tickers: vec![
                    perp_with_spread("binance", "MU", 99.0, 100.0),
                    perp_with_spread("kucoin", "MU", 101.0, 102.0),
                ]
                .into(),
                perp_ticker_row_evidence: vec![
                    row_evidence(
                        "binance",
                        "MU-USDT",
                        MarketDataSnapshotOperation::PerpTickers,
                        MarketDataQuality::Fresh,
                        MarketDataSourceKind::WsPush,
                    ),
                    row_evidence(
                        "kucoin",
                        "MUUSDTM",
                        MarketDataSnapshotOperation::PerpTickers,
                        MarketDataQuality::Fresh,
                        MarketDataSourceKind::WsPush,
                    ),
                ],
                funding_diff_stats: vec![diff_stats_row("MU", "binance", "kucoin", 95)],
                status: Some(MarketDataSnapshotStatus {
                    observed_at_ms: 1,
                    rows: vec![
                        status_row(
                            "binance",
                            MarketDataSnapshotOperation::PerpTickers,
                            MarketDataQuality::Fresh,
                        ),
                        status_row(
                            "kucoin",
                            MarketDataSnapshotOperation::PerpTickers,
                            MarketDataQuality::RateLimited,
                        ),
                    ],
                }),
                ..Default::default()
            }
        }
    }

    impl OpportunityMarketDataSource for MockSpotPerpSource {
        fn market_snapshot(&self) -> MarketDataSnapshot {
            let mut funding = rate_with_volume("binance", "BTC", 0.000_40, 50_000_000.0);
            funding.next_funding_time = funding.timestamp + 8 * 3_600_000;
            MarketDataSnapshot {
                funding: group_rates(vec![funding]).into(),
                perp_tickers: vec![perp_with_spread("binance", "BTC", 104.0, 105.0)].into(),
                spot_ticks: vec![spot("binance", "BTC/USDT", 100.0)].into(),
                ..Default::default()
            }
        }
    }

    impl OpportunityMarketDataSource for MockIndexCompositionSource {
        fn market_snapshot(&self) -> MarketDataSnapshot {
            MarketDataSnapshot {
                funding: group_rates(vec![
                    rate_with_volume("binance", "MU", 0.000_01, 2_000_000.0),
                    rate_with_volume("kucoin", "MU", 0.004_00, 2_000_000.0),
                ])
                .into(),
                perp_tickers: vec![
                    perp_with_spread("binance", "MU", 99.0, 100.0),
                    perp_with_spread("kucoin", "MU", 101.0, 102.0),
                ]
                .into(),
                index_compositions: vec![
                    index_composition("binance", "MU", "NASDAQ", 1.0),
                    index_composition("kucoin", "MU", "OTHER", 1.0),
                ]
                .into(),
                ..Default::default()
            }
        }
    }

    #[test]
    fn engine_finds_btc_opportunity() {
        let engine = ArbitrageEngineV3::new(
            ArbitrageConfig::default(),
            Arc::new(MockSource),
            100_000.0,
            0.5,
        );
        let opps = engine.run_scan();
        // BTC spread 0.0029 扬得过双腿官方费率成本；ETH 同费率被过滤
        assert!(!opps.is_empty(), "expected BTC opportunity");
        let Some(o) = opps.iter().find(|item| item.symbol == "BTC") else {
            panic!("expected BTC opportunity");
        };
        assert_eq!(o.symbol, "BTC");
        assert_eq!(o.data_source, "market-data-cache");
        assert_eq!(o.score, 0.0);
        assert!(o.score_breakdown.is_none());
        assert!(o.ranking_key.is_none());
        assert!(o.optimal_position >= 0.0);
        assert!(o.optimal_position <= 100_000.0 * 0.1 + 1e-6);
        assert!(o.execution_cost.as_ref().is_some_and(|cost| {
            cost.one_cycle.net_bps > 0.0
                && cost.one_cycle.covers_round_trip_cost
                && cost
                    .round_trip
                    .as_ref()
                    .is_some_and(|round_trip| round_trip.profitability_evidence.is_cost_verified())
        }));
        assert!(!o
            .execution_blockers
            .iter()
            .any(|item| item.contains("成本缺少双腿官方费率证据")));
    }

    #[test]
    fn scan_report_includes_candidate_and_coverage_meta() {
        let engine = ArbitrageEngineV3::new(
            ArbitrageConfig::default(),
            Arc::new(MockMarketSource),
            100_000.0,
            0.5,
        );

        let report = engine.run_scan_report();

        assert_eq!(report.meta.coverage.funding_symbols, 1);
        assert_eq!(report.meta.coverage.funding_venues, 1);
        assert_eq!(report.meta.coverage.funding_rows, 1);
        assert_eq!(report.meta.coverage.perp_tickers, 1);
        assert_eq!(report.meta.coverage.spot_ticks, 2);
        assert_eq!(report.meta.funding_row_evidence.len(), 1);
        assert_eq!(report.meta.funding_row_evidence[0].venue, "binance");
        assert_eq!(
            report.meta.funding_row_evidence[0].operation,
            MarketDataSnapshotOperation::FundingRates
        );
        assert_eq!(
            report.meta.funding_row_evidence[0].health.source,
            MarketDataSourceKind::WsPush
        );
        assert_eq!(report.meta.candidate_count, report.opportunities.len());
        assert_eq!(report.meta.emitted_count, report.opportunities.len());
        assert_eq!(report.meta.scan_outcome, OpportunityScanOutcome::Found);
    }

    #[test]
    fn scan_report_marks_empty_cache_as_market_data_degraded() {
        let engine = ArbitrageEngineV3::new(
            ArbitrageConfig::default(),
            Arc::new(EmptyMarketSource),
            100_000.0,
            0.5,
        );

        let report = engine.run_scan_report();

        assert!(report.opportunities.is_empty());
        assert_eq!(report.meta.market_data_problem_count, 3);
        assert_eq!(
            report.meta.degraded_venues,
            vec!["funding", "perp_tickers", "spot_ticks"]
        );
        assert_eq!(report.meta.scan_outcome, OpportunityScanOutcome::Warming);
    }

    #[test]
    fn scan_report_prefers_snapshot_status_for_market_data_problems() {
        let engine = ArbitrageEngineV3::new(
            ArbitrageConfig::default(),
            Arc::new(DegradedMarketStatusSource),
            100_000.0,
            0.5,
        );

        let report = engine.run_scan_report();

        assert_eq!(report.meta.market_data_problem_count, 1);
        assert_eq!(
            report.meta.degraded_venues,
            vec!["bybit:perp_tickers=rate_limited 0/1"]
        );
        assert!(report.meta.market_data_status.is_some());
        assert_eq!(
            report.meta.scan_outcome,
            OpportunityScanOutcome::PartialUpstream
        );
    }

    #[test]
    fn engine_filters_below_min_net_yield() {
        let config = ArbitrageConfig {
            min_net_yield: 0.999, // 设极高阈值，应过滤所有
            ..Default::default()
        };

        let engine = ArbitrageEngineV3::new(config, Arc::new(MockSource), 100_000.0, 0.5);
        let opps = engine.run_scan();
        assert!(opps.is_empty());
    }

    #[test]
    fn engine_drops_unproven_convergence_observations() {
        let engine = ArbitrageEngineV3::new(
            ArbitrageConfig {
                min_net_yield: 0.0,
                ..Default::default()
            },
            Arc::new(MockSource),
            100_000.0,
            0.5,
        );
        let constraints = engine.position_constraints();
        let mut candidate = raw("BTC", shared_types::ArbitrageType::CrossExchange);
        candidate.extra.strategy_kind = Some(StrategyKind::PerpPriceSpread);
        candidate.single_yield = 0.01;

        assert!(matches!(
            engine.evaluate_candidate(candidate, &constraints, chrono::Utc::now()),
            ScanCandidate::BelowMinNetYield
        ));
    }

    #[test]
    fn extreme_yield_guard_uses_strategy_edge_not_legacy_eight_hour_field() {
        let engine = ArbitrageEngineV3::new(
            ArbitrageConfig::default(),
            Arc::new(MockSource),
            100_000.0,
            0.5,
        );
        let constraints = engine.position_constraints();
        let mut misleading_legacy = raw("BTC", shared_types::ArbitrageType::CrossExchange);
        misleading_legacy.extra.strategy_kind = Some(StrategyKind::PerpCross);
        misleading_legacy.single_yield = 0.001;
        misleading_legacy.spread_8h = 10.0;

        assert!(!matches!(
            engine.evaluate_candidate(misleading_legacy, &constraints, chrono::Utc::now()),
            ScanCandidate::ExtremeYield
        ));

        let mut real_extreme = raw("BTC", shared_types::ArbitrageType::CrossExchange);
        real_extreme.extra.strategy_kind = Some(StrategyKind::PerpCross);
        real_extreme.single_yield = 10.0;
        real_extreme.spread_8h = 0.001;

        assert!(matches!(
            engine.evaluate_candidate(real_extreme, &constraints, chrono::Utc::now()),
            ScanCandidate::ExtremeYield
        ));
    }

    #[test]
    fn perp_cross_frontier_promotes_the_aligned_pair_before_truncation() {
        let mut rows = (0..7)
            .map(|index| {
                let mut row = raw("MU", shared_types::ArbitrageType::CrossExchange);
                row.extra.strategy_kind = Some(StrategyKind::PerpCross);
                row.single_yield = if index == 6 { 0.02 } else { 0.03 };
                row.extra.long_price = Some(100.0);
                row.extra.short_price = Some(if index == 6 { 100.01 } else { 101.0 });
                row.extra.long_leg_market_evidence = Some(fresh_ws_leg_evidence("binance"));
                row.extra.short_leg_market_evidence = Some(fresh_ws_leg_evidence("okx"));
                row
            })
            .collect::<Vec<_>>();

        retain_perp_cross_frontier(&mut rows, 0.0, 0.0);

        assert_eq!(rows.len(), MAX_PERP_CROSS_PAIRS_PER_SYMBOL);
        assert!(rows.iter().any(|row| row.extra.short_price == Some(100.01)));
    }

    #[test]
    fn frontier_numeric_fee_cost_matches_full_evidence_cost() {
        let row = raw("MU", shared_types::ArbitrageType::CrossExchange);
        let numeric =
            standard_round_trip_cost(&row, 0.000_2, StrategyExecutionCycle::PairedOpenClose)
                .expect("standard fee cost");
        let (long, short) = row_fee_snapshots(&row, 1).expect("standard fee evidence");
        let evidenced = cost_from_fee_snapshots(
            &long,
            &short,
            0.000_2,
            StrategyExecutionCycle::PairedOpenClose,
        );

        assert!((numeric - evidenced.round_trip_cost).abs() < f64::EPSILON);
    }

    #[test]
    fn scan_report_marks_healthy_zero_candidates_as_true_empty() {
        let engine = ArbitrageEngineV3::new(
            ArbitrageConfig::default(),
            Arc::new(HealthyFlatMarketSource),
            100_000.0,
            0.5,
        );

        let report = engine.run_scan_report();

        assert!(report.opportunities.is_empty());
        assert_eq!(report.meta.market_data_problem_count, 0);
        assert_eq!(report.meta.candidate_count, 0);
        assert_eq!(report.meta.scan_outcome, OpportunityScanOutcome::TrueEmpty);
    }

    #[test]
    fn scan_report_marks_clean_dropped_candidates_as_filtered_empty() {
        let config = ArbitrageConfig {
            min_net_yield: 0.999,
            ..Default::default()
        };
        let engine =
            ArbitrageEngineV3::new(config, Arc::new(HealthyOpportunitySource), 100_000.0, 0.5);

        let report = engine.run_scan_report();

        assert!(report.opportunities.is_empty());
        assert_eq!(report.meta.market_data_problem_count, 0);
        assert!(report.meta.candidate_count > 0);
        assert_eq!(
            report.meta.scan_outcome,
            OpportunityScanOutcome::FilteredEmpty
        );
    }

    #[test]
    fn engine_includes_market_strategy_inputs() {
        let engine = ArbitrageEngineV3::new(
            ArbitrageConfig::default(),
            Arc::new(MockMarketSource),
            100_000.0,
            0.5,
        );
        let opps = engine.run_scan();
        let kinds: Vec<_> = opps.iter().filter_map(|opp| opp.strategy_kind).collect();
        assert!(kinds.contains(&shared_types::StrategyKind::SpotPerp));
        assert!(kinds.contains(&shared_types::StrategyKind::CrossSpotPerp));
        assert!(opps
            .iter()
            .filter(|opp| matches!(
                opp.strategy_kind,
                Some(shared_types::StrategyKind::SpotPerp)
                    | Some(shared_types::StrategyKind::CrossSpotPerp)
            ))
            .all(|opp| {
                opp.long_leg_market_evidence.is_some() && opp.short_leg_market_evidence.is_some()
            }));
    }

    #[test]
    fn engine_keeps_mu_km_and_xyz_against_kucoin_after_costs() {
        let engine = ArbitrageEngineV3::new(
            ArbitrageConfig::default(),
            Arc::new(MockMuSource),
            100_000.0,
            0.5,
        );
        let opps = engine.run_scan();

        assert!(opps.iter().any(|opp| {
            opp.symbol == "MU"
                && opp.long_exchange == "hyperliquid:km"
                && opp.short_exchange == "kucoin"
        }));
        assert!(opps.iter().any(|opp| {
            opp.symbol == "MU"
                && opp.long_exchange == "hyperliquid:xyz"
                && opp.short_exchange == "kucoin"
        }));
    }

    #[test]
    fn one_scan_projects_every_row_and_fee_snapshot_at_one_time() {
        let engine = ArbitrageEngineV3::new(
            ArbitrageConfig::default(),
            Arc::new(MockMuSource),
            100_000.0,
            0.5,
        );

        let rows = engine.run_scan();
        let observed_at = rows.first().expect("scan rows").updated_at;
        let observed_at_ms = observed_at.timestamp_millis();

        assert!(rows.iter().all(|row| row.updated_at == observed_at));
        assert!(rows.iter().all(|row| {
            row.execution_cost
                .as_ref()
                .and_then(|cost| cost.round_trip.as_ref())
                .and_then(|cost| cost.long_leg.fee_snapshot.as_ref())
                .is_some_and(|fee| fee.fetched_at_ms == observed_at_ms)
        }));
    }

    #[test]
    fn divergent_perp_cross_keeps_public_market_evidence_until_ticket_stage() {
        let engine = ArbitrageEngineV3::new(
            ArbitrageConfig::default(),
            Arc::new(MockPerpPriceSource),
            100_000.0,
            0.5,
        );

        let opps = engine.run_scan();
        let Some(opp) = opps.iter().find(|opp| {
            opp.symbol == "MU" && opp.long_exchange == "binance" && opp.short_exchange == "kucoin"
        }) else {
            panic!("expected MU perp-cross opportunity");
        };

        assert_perp_cross_market_evidence(opp);
        assert_perp_cross_cost_evidence(opp);
    }

    fn assert_perp_cross_market_evidence(opp: &ArbitrageOpportunityDto) {
        assert_eq!(opp.long_price, Some(100.0));
        assert_eq!(opp.short_price, Some(101.0));
        assert_eq!(
            opp.long_leg_market_evidence
                .as_ref()
                .map(|evidence| evidence.health.quality),
            Some(MarketDataQuality::Fresh)
        );
        assert_eq!(
            opp.short_leg_market_evidence
                .as_ref()
                .map(|evidence| evidence.health.quality),
            Some(MarketDataQuality::Fresh)
        );
        assert_eq!(
            opp.short_leg_market_evidence
                .as_ref()
                .map(|evidence| evidence.health.source),
            Some(MarketDataSourceKind::WsPush)
        );
        assert!(opp
            .execution_blockers
            .iter()
            .any(|blocker| blocker.contains("双边可成交价格差")));
        assert_eq!(
            opp.funding_diff_window
                .as_ref()
                .map(|window| window.current_percentile),
            Some(95)
        );
        assert_eq!(opp.funding_diff_windows.len(), 3);
    }

    fn assert_perp_cross_cost_evidence(opp: &ArbitrageOpportunityDto) {
        let round_trip = opp
            .execution_cost
            .as_ref()
            .and_then(|cost| cost.round_trip.as_ref())
            .expect("verified round trip cost");
        assert!(round_trip
            .long_leg
            .fee_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.evidence.as_ref())
            .is_some_and(|evidence| evidence.evidence_id.contains("binance")));
        assert!(round_trip
            .short_leg
            .fee_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.evidence.as_ref())
            .is_some_and(|evidence| evidence.evidence_id.contains("kucoin")));
        assert_eq!(
            round_trip.profitability_evidence.status,
            shared_types::ProfitabilityEvidenceStatus::Verified
        );
        assert!(round_trip.profitability_evidence.is_cost_verified());
        assert_eq!(
            round_trip
                .profitability_evidence
                .verified_fee_snapshot_count,
            2
        );
        assert_eq!(round_trip.profitability_evidence.fee_evidence_ids.len(), 2);
        assert!(round_trip
            .profitability_evidence
            .funding_history
            .as_ref()
            .is_some_and(shared_types::FundingHistoryEvidence::is_usable));
    }

    #[test]
    fn spot_perp_scan_leaves_ticket_bound_checks_for_preview() {
        let engine = ArbitrageEngineV3::new(
            ArbitrageConfig::default(),
            Arc::new(MockSpotPerpSource),
            100_000.0,
            0.5,
        );

        let opps = engine.run_scan();
        let Some(opp) = opps
            .iter()
            .find(|opp| opp.strategy_kind == Some(shared_types::StrategyKind::SpotPerp))
        else {
            panic!("expected spot-perp opportunity");
        };

        assert!(opp
            .execution_blockers
            .iter()
            .any(|blocker| blocker == shared_types::DEFERRED_SPOT_PERP_TICKET_BLOCKER));
    }

    #[test]
    fn scan_cost_uses_spot_fee_for_forward_spot_perp_leg() {
        let mut row = raw("MU", shared_types::ArbitrageType::SpotFutures);
        row.extra.strategy_kind = Some(StrategyKind::SpotPerp);
        row.extra.spot_leg_mode = Some(shared_types::SpotLegMode::BuySpot);
        row.extra.long_depth_symbol = Some("MU/USDT".into());

        let cost = row_round_trip_cost(&row, 0.0);

        assert!((cost.round_trip_cost - 0.002_7).abs() < 1e-9);
    }

    #[test]
    fn cross_spot_perp_short_leg_uses_perp_fee_product() {
        let mut row = raw("MU", shared_types::ArbitrageType::CrossSpotFutures);
        row.extra.strategy_kind = Some(StrategyKind::CrossSpotPerp);
        row.extra.spot_leg_mode = Some(shared_types::SpotLegMode::BuySpot);
        row.extra.long_depth_symbol = Some("MU/USDT".into());
        row.extra.short_depth_symbol = Some("MUUSDT".into());

        assert_eq!(row_fee_product(&row, OrderSide::Buy), FeeProduct::Spot);
        assert_eq!(row_fee_product(&row, OrderSide::Sell), FeeProduct::Perp);
    }

    #[test]
    fn spot_cross_scan_cost_charges_the_entry_pair_and_transfer_cycle() {
        let mut row = raw("MU", shared_types::ArbitrageType::SpotCross);
        row.extra.strategy_kind = Some(StrategyKind::SpotCross);
        row.extra.spot_leg_mode = Some(shared_types::SpotLegMode::SellInventory);
        row.extra.long_depth_symbol = Some("MU/USDT".into());
        row.extra.short_depth_symbol = Some("MU/USDT".into());

        let estimated = row_round_trip_cost(&row, 0.000_2);
        assert!((estimated.round_trip_cost - 0.002_4).abs() < 1e-9);

        let verified = verified_row_round_trip_cost(&mut row, 0.000_2, 1);
        let evidence = row.extra.cost_round_trip.as_ref().expect("fee evidence");
        assert_eq!(evidence.close_fee_bps, 0.0);
        assert_eq!(evidence.close_slippage_bps, 0.0);
        assert_eq!(evidence.long_leg.close_fee_bps, 0.0);
        assert_eq!(evidence.short_leg.close_fee_bps, 0.0);
        assert!((verified.round_trip_cost - 0.002_4).abs() < 1e-9);

        let profile = crate::calculator::execution_cost_profile(&row, &verified, 1);
        assert!((profile.fee_bps - 20.0).abs() < 1e-9);
        assert!((profile.wear_bps - 4.0).abs() < 1e-9);
        assert!((profile.total_cost_bps - 24.0).abs() < 1e-9);
    }

    #[test]
    fn scan_cost_uses_spot_fee_for_reverse_spot_perp_leg() {
        let mut row = raw("MU", shared_types::ArbitrageType::CrossSpotFutures);
        row.extra.short_depth_symbol = Some("MU/USDT".into());

        let cost = row_round_trip_cost(&row, 0.0);

        assert!((cost.round_trip_cost - 0.002_6).abs() < 1e-9);
    }

    #[test]
    fn engine_blocks_low_index_composition_overlap() {
        let engine = ArbitrageEngineV3::new(
            ArbitrageConfig::default(),
            Arc::new(MockIndexCompositionSource),
            100_000.0,
            0.5,
        );

        let opps = engine.run_scan();
        let Some(opp) = opps.iter().find(|opp| {
            opp.symbol == "MU" && opp.long_exchange == "binance" && opp.short_exchange == "kucoin"
        }) else {
            panic!("expected MU perp-cross opportunity");
        };

        assert!(!opp.execution_eligible);
        assert!(opp
            .execution_blockers
            .iter()
            .any(|item| item.contains("指数成分重合度")));
        assert!(opp
            .risk_warnings
            .iter()
            .any(|item| item.contains("指数成分重合度")));
        let profile = opp
            .index_composition
            .as_ref()
            .expect("index composition profile");
        let long_evidence = profile.long_evidence.as_ref().expect("long evidence");
        assert_eq!(long_evidence.source, "test");
        assert_eq!(long_evidence.received_at_ms, 1);
        let short_evidence = profile.short_evidence.as_ref().expect("short evidence");
        assert_eq!(short_evidence.source, "test");
    }

    #[test]
    fn composition_hidden_price_blocks_execution_even_when_verified() {
        let row = raw("MU", shared_types::ArbitrageType::CrossExchange);
        let hidden_long = index_composition("binance", "MU", "NASDAQ", 1.0);
        let hidden_short = index_composition("kucoin", "MU", "NASDAQ", 1.0);

        let risk = composition_risk(&row, Some(&hidden_long), Some(&hidden_short))
            .expect("composition risk");
        assert!(risk.hidden_price);
        assert!(risk
            .blocker
            .as_deref()
            .is_some_and(|blocker| blocker.contains("隐藏价格")));

        let mut priced_long = hidden_long;
        let mut priced_short = hidden_short;
        priced_long.components[0].price = Some(100.0);
        priced_short.components[0].price = Some(100.0);
        let risk = composition_risk(&row, Some(&priced_long), Some(&priced_short))
            .expect("composition risk");
        assert!(!risk.hidden_price);
        assert_eq!(risk.blocker, None);
    }

    #[test]
    fn composition_missing_payload_evidence_blocks_execution() {
        let row = raw("MU", shared_types::ArbitrageType::CrossExchange);
        let mut long = index_composition("binance", "MU", "NASDAQ", 1.0);
        let mut short_priced = index_composition("kucoin", "MU", "NASDAQ", 1.0);
        long.components[0].price = Some(100.0);
        short_priced.components[0].price = Some(100.0);
        long.source_url = None;

        let risk =
            composition_risk(&row, Some(&long), Some(&short_priced)).expect("composition risk");
        assert!(risk
            .blocker
            .as_deref()
            .is_some_and(|blocker| blocker.contains("缺官方 payload 证据")));
    }

    fn group_rates(
        rates: Vec<FundingRateData>,
    ) -> HashMap<String, HashMap<String, FundingRateData>> {
        let mut out: HashMap<String, HashMap<String, FundingRateData>> = HashMap::new();
        for rate in rates {
            out.entry(rate.symbol.clone())
                .or_default()
                .insert(rate.exchange.clone(), rate);
        }
        out
    }

    fn rate(exchange: &str, symbol: &str, value: f64) -> FundingRateData {
        rate_with_volume(exchange, symbol, value, 2_000_000.0)
    }

    fn raw(symbol: &str, arb_type: shared_types::ArbitrageType) -> RawOpportunity {
        RawOpportunity {
            symbol: symbol.into(),
            arb_type,
            long_exchange: "binance".into(),
            short_exchange: "okx".into(),
            long_rate: rate("binance", symbol, 0.0001),
            short_rate: rate("okx", symbol, 0.0002),
            spread_8h: 0.0001,
            single_yield: 0.0001,
            extra: Default::default(),
        }
    }

    fn rate_with_volume(
        exchange: &str,
        symbol: &str,
        value: f64,
        volume_24h: f64,
    ) -> FundingRateData {
        let timestamp = 1;
        FundingRateData {
            symbol: symbol.into(),
            exchange: exchange.into(),
            rate: value,
            rate_8h: value,
            predicted_rate: None,
            next_funding_time: timestamp + 8 * 3_600_000,
            funding_interval: 8,
            volume_24h,
            timestamp,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        }
    }

    fn perp(exchange: &str, symbol: &str, bid: f64) -> TickerInfo {
        perp_with_spread(exchange, symbol, bid, bid + 0.2)
    }

    fn perp_with_spread(exchange: &str, symbol: &str, bid: f64, ask: f64) -> TickerInfo {
        TickerInfo {
            symbol: symbol.into(),
            exchange: exchange.into(),
            bid,
            ask,
            last: (bid + ask) * 0.5,
            volume_24h: 2_000_000.0,
            timestamp: 1,
        }
    }

    fn spot(venue: &str, symbol: &str, ask: f64) -> SpotTick {
        SpotTick {
            venue: venue.into(),
            symbol: symbol.into(),
            bid: dec!(98),
            ask: dec_from_f64(ask),
            last: dec_from_f64(ask),
            bid_size: Some(dec!(1)),
            ask_size: Some(dec!(1)),
            volume_24h: dec!(2000000),
            exchange_ts_ms: Some(1),
            received_at_ms: 1,
        }
    }

    fn diff_stats_row(
        symbol: &str,
        long_exchange: &str,
        short_exchange: &str,
        percentile: u8,
    ) -> FundingDiffStatsRow {
        FundingDiffStatsRow {
            symbol: symbol.into(),
            long_exchange: long_exchange.into(),
            short_exchange: short_exchange.into(),
            computed_at_ms: 1,
            latest_at_ms: 1,
            latest_diff_bps: 10.0,
            base_interval_hours: 8,
            source: "test".into(),
            freshness_ms: Some(0),
            problem: None,
            problem_detail: None,
            retry_after_ms: None,
            evidence: Default::default(),
            windows: vec![
                diff_window(1, 8, 6.0, percentile.saturating_sub(10)),
                diff_window(3, 24, 7.0, percentile.saturating_sub(4)),
                diff_window(9, 72, 8.0, percentile),
            ],
        }
    }

    fn diff_window(
        cycles: u32,
        window_hours: u32,
        mean_diff_bps: f64,
        percentile: u8,
    ) -> FundingDiffWindowStats {
        FundingDiffWindowStats {
            cycles,
            window_hours,
            sample_count: cycles as usize,
            mean_diff_bps,
            p50_diff_bps: mean_diff_bps,
            p75_diff_bps: mean_diff_bps + 1.0,
            p90_diff_bps: mean_diff_bps + 2.0,
            p95_diff_bps: mean_diff_bps + 2.0,
            stddev_diff_bps: 1.0,
            positive_ratio: 1.0,
            reversal_count: 0,
            current_percentile: percentile,
            source: "test".into(),
            freshness_ms: Some(0),
            sample_health: FundingDiffSampleHealth::Ok,
            problem: None,
            problem_detail: None,
            retry_after_ms: None,
            evidence: shared_types::FundingHistoryEvidence {
                source: "test".into(),
                observed_at_ms: 1,
                latest_at_ms: 1,
                freshness_ms: Some(0),
                sample_count: cycles as usize,
                sample_health: FundingDiffSampleHealth::Ok,
                problem: None,
                retry_after_ms: None,
            },
        }
    }

    fn index_composition(
        venue: &str,
        symbol: &str,
        component_name: &str,
        weight: f64,
    ) -> IndexCompositionSnapshot {
        IndexCompositionSnapshot {
            venue: venue.into(),
            symbol: symbol.into(),
            index_id: format!("{symbol}-INDEX"),
            components: vec![IndexComponent {
                symbol: format!("{symbol}/USDT"),
                name: component_name.into(),
                weight,
                price: None,
            }],
            quality: IndexCompositionQuality::Verified,
            source: "test".into(),
            received_at_ms: 1,
            freshness_ms: Some(0),
            error: None,
            retry_after_ms: None,
            source_url: Some(format!("https://{venue}.test/constituents?symbol={symbol}")),
            payload_sha256: Some("deadbeef".repeat(8)),
            schema_version: Some("test-index-constituents/1".to_owned()),
        }
    }

    fn status_row(
        venue: &str,
        operation: MarketDataSnapshotOperation,
        quality: MarketDataQuality,
    ) -> MarketDataSnapshotStatusRow {
        MarketDataSnapshotStatusRow {
            venue: venue.into(),
            operation,
            health: MarketDataHealth {
                quality,
                source: MarketDataSourceKind::RestBaseline,
                freshness_ms: None,
                retry_after_ms: (quality == MarketDataQuality::RateLimited).then_some(2_000),
                last_error: (quality != MarketDataQuality::Fresh).then(|| "degraded".into()),
                observed_at_ms: 1,
                coverage: Some(MarketDataCoverage::new(1, 0)),
                problem: None,
            },
        }
    }

    fn fresh_scan_status() -> MarketDataSnapshotStatus {
        MarketDataSnapshotStatus {
            observed_at_ms: 1,
            rows: vec![
                fresh_status_row(MarketDataSnapshotOperation::FundingRates, 2),
                fresh_status_row(MarketDataSnapshotOperation::PerpTickers, 2),
                fresh_status_row(MarketDataSnapshotOperation::SpotTicks, 2),
            ],
        }
    }

    fn fresh_status_row(
        operation: MarketDataSnapshotOperation,
        received: u64,
    ) -> MarketDataSnapshotStatusRow {
        MarketDataSnapshotStatusRow {
            venue: "all".into(),
            operation,
            health: MarketDataHealth {
                quality: MarketDataQuality::Fresh,
                source: MarketDataSourceKind::LocalCache,
                freshness_ms: Some(10),
                retry_after_ms: None,
                last_error: None,
                observed_at_ms: 1,
                coverage: Some(MarketDataCoverage::new(received, received)),
                problem: None,
            },
        }
    }

    fn row_evidence(
        venue: &str,
        symbol: &str,
        operation: MarketDataSnapshotOperation,
        quality: MarketDataQuality,
        source: MarketDataSourceKind,
    ) -> MarketDataRowEvidence {
        MarketDataRowEvidence {
            venue: venue.into(),
            symbol: symbol.into(),
            operation,
            health: MarketDataHealth {
                quality,
                source,
                freshness_ms: Some(10),
                retry_after_ms: None,
                last_error: None,
                observed_at_ms: 1,
                coverage: Some(MarketDataCoverage::new(1, 1)),
                problem: None,
            },
        }
    }

    fn fresh_ws_leg_evidence(venue: &str) -> OpportunityLegMarketEvidence {
        OpportunityLegMarketEvidence {
            venue: venue.into(),
            symbol: "MUUSDT".into(),
            price: Some(100.0),
            health: MarketDataHealth {
                quality: MarketDataQuality::Fresh,
                source: MarketDataSourceKind::WsPush,
                freshness_ms: Some(10),
                retry_after_ms: None,
                last_error: None,
                observed_at_ms: 1,
                coverage: Some(MarketDataCoverage::new(1, 1)),
                problem: None,
            },
        }
    }

    fn dec_from_f64(value: f64) -> rust_decimal::Decimal {
        rust_decimal::Decimal::from_f64_retain(value).unwrap_or(rust_decimal::Decimal::ZERO)
    }
}
