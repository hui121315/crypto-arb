use crate::algorithms::market_index::MarketScanIndex;
use crate::algorithms::{
    cross_exchange, cross_spot_perp, perp_price_spread, spot_cross, spot_perp,
};
use crate::interfaces::MarketDataSnapshot;
use crate::models::{CostBreakdown, RawOpportunity};
use shared_types::{
    is_p0_executable_strategy, ArbitrageConfig, ArbitrageType, RiskLevel, StrategyKind,
};

pub(crate) trait MarketStrategy: Sync {
    fn kind(&self) -> StrategyKind;
    fn arb_type(&self) -> ArbitrageType;
    fn scan(
        &self,
        market: &MarketDataSnapshot,
        index: &MarketScanIndex<'_>,
        config: &ArbitrageConfig,
    ) -> Vec<RawOpportunity>;

    fn max_single_yield(&self, default_limit: f64) -> f64 {
        default_limit
    }
}

struct PerpCrossStrategy;
struct PerpPriceSpreadStrategy;
struct SpotPerpStrategy;
struct CrossSpotPerpStrategy;
struct SpotCrossStrategy;

static PERP_CROSS: PerpCrossStrategy = PerpCrossStrategy;
static PERP_PRICE_SPREAD: PerpPriceSpreadStrategy = PerpPriceSpreadStrategy;
static SPOT_PERP: SpotPerpStrategy = SpotPerpStrategy;
static CROSS_SPOT_PERP: CrossSpotPerpStrategy = CrossSpotPerpStrategy;
static SPOT_CROSS: SpotCrossStrategy = SpotCrossStrategy;

pub(crate) fn p0_market_strategies() -> [&'static dyn MarketStrategy; 5] {
    [
        &PERP_CROSS,
        &PERP_PRICE_SPREAD,
        &SPOT_PERP,
        &CROSS_SPOT_PERP,
        &SPOT_CROSS,
    ]
}

pub(crate) fn strategy_for_raw(raw: &RawOpportunity) -> Option<&'static dyn MarketStrategy> {
    match raw.extra.strategy_kind {
        Some(kind) if !is_p0_executable_strategy(kind) => None,
        Some(StrategyKind::PerpCross) => Some(&PERP_CROSS),
        Some(StrategyKind::PerpPriceSpread) => Some(&PERP_PRICE_SPREAD),
        Some(StrategyKind::SpotPerp) => Some(&SPOT_PERP),
        Some(StrategyKind::CrossSpotPerp) => Some(&CROSS_SPOT_PERP),
        Some(StrategyKind::SpotCross) => Some(&SPOT_CROSS),
        Some(_) => None,
        None => match raw.arb_type {
            ArbitrageType::CrossExchange => Some(&PERP_CROSS),
            ArbitrageType::SpotFutures => Some(&SPOT_PERP),
            ArbitrageType::CrossSpotFutures => Some(&CROSS_SPOT_PERP),
            ArbitrageType::SpotCross => Some(&SPOT_CROSS),
            _ => None,
        },
    }
}

pub(crate) fn max_single_yield_for(raw: &RawOpportunity, default_limit: f64) -> f64 {
    strategy_for_raw(raw).map_or(default_limit, |strategy| {
        strategy.max_single_yield(default_limit)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum YieldRealization {
    Recurring,
    InstantSpread,
    ConvergenceSpread,
    ProjectedBasisCarry,
}

pub(crate) fn yield_realization(raw: &RawOpportunity) -> YieldRealization {
    match raw.extra.strategy_kind {
        Some(StrategyKind::SpotCross) => YieldRealization::InstantSpread,
        Some(StrategyKind::PerpPriceSpread) => YieldRealization::ConvergenceSpread,
        Some(StrategyKind::SpotPerp | StrategyKind::CrossSpotPerp) => {
            YieldRealization::ProjectedBasisCarry
        }
        _ => YieldRealization::Recurring,
    }
}

/// List-stage risk is structural strategy risk, not a synthetic annualized scan score.
pub(crate) fn list_risk_level(raw: &RawOpportunity) -> RiskLevel {
    match raw.extra.strategy_kind {
        Some(StrategyKind::PerpCross | StrategyKind::SpotCross) => RiskLevel::Medium,
        Some(
            StrategyKind::PerpPriceSpread | StrategyKind::SpotPerp | StrategyKind::CrossSpotPerp,
        ) => RiskLevel::High,
        Some(_) => RiskLevel::High,
        None => match raw.arb_type {
            ArbitrageType::CrossExchange | ArbitrageType::SpotCross => RiskLevel::Medium,
            _ => RiskLevel::High,
        },
    }
}

pub(crate) fn expected_gross_yield(raw: &RawOpportunity) -> f64 {
    if is_perp_cross(raw) {
        return raw.single_yield;
    }
    match yield_realization(raw) {
        YieldRealization::ConvergenceSpread => raw
            .extra
            .price_spread_convergence
            .filter(|evidence| evidence.evidence_ready)
            .map(|evidence| evidence.expected_gross_bps / 10_000.0)
            .unwrap_or(0.0),
        YieldRealization::InstantSpread
        | YieldRealization::ProjectedBasisCarry
        | YieldRealization::Recurring => raw.single_yield,
    }
}

pub(crate) fn scan_edge(raw: &RawOpportunity) -> f64 {
    raw.single_yield
}

pub(crate) fn gross_yield_at_horizon(raw: &RawOpportunity, periods: u32) -> f64 {
    if is_perp_cross(raw) {
        return raw.single_yield;
    }
    match yield_realization(raw) {
        YieldRealization::Recurring => raw.single_yield * f64::from(periods.max(1)),
        YieldRealization::InstantSpread
        | YieldRealization::ConvergenceSpread
        | YieldRealization::ProjectedBasisCarry => expected_gross_yield(raw),
    }
}

pub(crate) fn min_holding_periods(raw: &RawOpportunity, cost: &CostBreakdown) -> u32 {
    if is_perp_cross(raw) {
        return if raw.single_yield > cost.round_trip_cost.max(0.0) {
            1
        } else {
            u32::MAX
        };
    }
    match yield_realization(raw) {
        YieldRealization::InstantSpread => {
            return if raw.single_yield > cost.round_trip_cost {
                1
            } else {
                u32::MAX
            };
        }
        YieldRealization::ConvergenceSpread => return 1,
        YieldRealization::ProjectedBasisCarry => {
            return if raw.single_yield > cost.round_trip_cost.max(0.0) {
                1
            } else {
                u32::MAX
            };
        }
        YieldRealization::Recurring => {}
    }
    crate::algorithms::cost_model::min_holding_periods(raw.single_yield, cost)
}

pub(crate) fn net_single_yield(
    raw: &RawOpportunity,
    cost: &CostBreakdown,
    min_holding_periods: u32,
) -> f64 {
    if is_perp_cross(raw) {
        return raw.single_yield - cost.round_trip_cost.max(0.0);
    }
    match yield_realization(raw) {
        YieldRealization::InstantSpread => return raw.single_yield - cost.round_trip_cost,
        YieldRealization::ConvergenceSpread => {
            return raw
                .extra
                .price_spread_convergence
                .filter(|evidence| evidence.evidence_ready)
                .map(|evidence| evidence.projected_net_bps / 10_000.0)
                .unwrap_or(-cost.round_trip_cost.max(0.0));
        }
        YieldRealization::ProjectedBasisCarry => {
            return raw.single_yield - cost.round_trip_cost.max(0.0);
        }
        YieldRealization::Recurring => {}
    }
    crate::algorithms::cost_model::net_single_yield(raw.single_yield, cost, min_holding_periods)
}

fn is_perp_cross(raw: &RawOpportunity) -> bool {
    raw.extra.strategy_kind == Some(StrategyKind::PerpCross)
        || raw.extra.strategy_kind.is_none() && raw.arb_type == ArbitrageType::CrossExchange
}

impl MarketStrategy for PerpCrossStrategy {
    fn kind(&self) -> StrategyKind {
        StrategyKind::PerpCross
    }

    fn arb_type(&self) -> ArbitrageType {
        ArbitrageType::CrossExchange
    }

    fn scan(
        &self,
        _market: &MarketDataSnapshot,
        index: &MarketScanIndex<'_>,
        config: &ArbitrageConfig,
    ) -> Vec<RawOpportunity> {
        cross_exchange::scan_indexed(index, config.min_spread, config.min_volume_24h)
    }
}

impl MarketStrategy for PerpPriceSpreadStrategy {
    fn kind(&self) -> StrategyKind {
        StrategyKind::PerpPriceSpread
    }

    fn arb_type(&self) -> ArbitrageType {
        ArbitrageType::CrossExchange
    }

    fn scan(
        &self,
        _market: &MarketDataSnapshot,
        index: &MarketScanIndex<'_>,
        config: &ArbitrageConfig,
    ) -> Vec<RawOpportunity> {
        perp_price_spread::scan_indexed(
            index,
            perp_price_spread::PerpPriceSpreadConfig {
                min_spread_bps: min_edge_bps(config.min_spread),
                min_volume_24h: config.min_volume_24h,
            },
        )
    }

    fn max_single_yield(&self, default_limit: f64) -> f64 {
        default_limit.max(0.20)
    }
}

impl MarketStrategy for SpotPerpStrategy {
    fn kind(&self) -> StrategyKind {
        StrategyKind::SpotPerp
    }

    fn arb_type(&self) -> ArbitrageType {
        ArbitrageType::SpotFutures
    }

    fn scan(
        &self,
        _market: &MarketDataSnapshot,
        index: &MarketScanIndex<'_>,
        config: &ArbitrageConfig,
    ) -> Vec<RawOpportunity> {
        spot_perp::scan_indexed(
            index,
            spot_perp::SpotPerpConfig {
                min_basis_bps: min_edge_bps(config.min_spread),
                min_volume_24h: config.min_volume_24h,
            },
        )
    }

    fn max_single_yield(&self, default_limit: f64) -> f64 {
        default_limit.max(0.05)
    }
}

impl MarketStrategy for CrossSpotPerpStrategy {
    fn kind(&self) -> StrategyKind {
        StrategyKind::CrossSpotPerp
    }

    fn arb_type(&self) -> ArbitrageType {
        ArbitrageType::CrossSpotFutures
    }

    fn scan(
        &self,
        _market: &MarketDataSnapshot,
        index: &MarketScanIndex<'_>,
        config: &ArbitrageConfig,
    ) -> Vec<RawOpportunity> {
        cross_spot_perp::scan_indexed(
            index,
            cross_spot_perp::CrossSpotPerpConfig {
                min_basis_bps: min_edge_bps(config.min_spread),
                min_volume_24h: config.min_volume_24h,
                min_net_yield: config.min_net_yield,
                default_slippage: config.default_slippage,
            },
        )
    }

    fn max_single_yield(&self, default_limit: f64) -> f64 {
        default_limit.max(0.05)
    }
}

impl MarketStrategy for SpotCrossStrategy {
    fn kind(&self) -> StrategyKind {
        StrategyKind::SpotCross
    }

    fn arb_type(&self) -> ArbitrageType {
        ArbitrageType::SpotCross
    }

    fn scan(
        &self,
        _market: &MarketDataSnapshot,
        index: &MarketScanIndex<'_>,
        config: &ArbitrageConfig,
    ) -> Vec<RawOpportunity> {
        spot_cross::scan_indexed(
            index,
            spot_cross::SpotCrossConfig {
                min_spread_bps: min_edge_bps(config.min_spread),
                min_volume_24h: config.min_volume_24h,
            },
        )
    }

    fn max_single_yield(&self, default_limit: f64) -> f64 {
        default_limit.max(0.20)
    }
}

fn min_edge_bps(min_spread: f64) -> f64 {
    (min_spread.max(0.0) * 10_000.0).max(5.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::FundingRateData;

    #[test]
    fn p0_market_registry_contains_all_five_product_strategies() {
        let strategies = p0_market_strategies();

        assert_eq!(strategies.len(), 5);
        assert_eq!(strategies[0].kind(), StrategyKind::PerpCross);
        assert_eq!(strategies[1].kind(), StrategyKind::PerpPriceSpread);
        assert_eq!(strategies[2].kind(), StrategyKind::SpotPerp);
        assert_eq!(strategies[3].kind(), StrategyKind::CrossSpotPerp);
        assert_eq!(strategies[4].kind(), StrategyKind::SpotCross);
    }

    #[test]
    fn metadata_maps_raw_opportunities_to_p0_strategy_rules() {
        let spot = raw("BTC", ArbitrageType::SpotFutures, StrategyKind::SpotPerp);
        let perp = raw("MU", ArbitrageType::CrossExchange, StrategyKind::PerpCross);

        assert_eq!(max_single_yield_for(&spot, 0.01), 0.05);
        assert_eq!(list_risk_level(&spot), RiskLevel::High);
        assert_eq!(list_risk_level(&perp), RiskLevel::Medium);
    }

    #[test]
    fn options_perp_basis_cannot_fall_back_to_a_p0_market_rule() {
        let raw = raw(
            "BTC",
            ArbitrageType::SpotFutures,
            StrategyKind::OptionsPerpBasis,
        );

        assert!(strategy_for_raw(&raw).is_none());
        assert_eq!(max_single_yield_for(&raw, 0.01), 0.01);
        assert_eq!(list_risk_level(&raw), RiskLevel::High);
    }

    #[test]
    fn diagnostic_non_p0_kinds_cannot_fall_back_to_p0_market_rules() {
        for kind in [StrategyKind::Triangular, StrategyKind::OnchainDepeg] {
            let raw = raw("BTC", ArbitrageType::SpotFutures, kind);

            assert!(
                strategy_for_raw(&raw).is_none(),
                "{kind:?} must stay diagnostic"
            );
            assert_eq!(max_single_yield_for(&raw, 0.01), 0.01);
            assert_eq!(list_risk_level(&raw), RiskLevel::High);
        }
    }

    #[test]
    fn one_shot_price_spreads_must_cover_the_full_cycle_cost_once() {
        let mut spot = raw("BTC", ArbitrageType::SpotCross, StrategyKind::SpotCross);
        spot.single_yield = 0.01;
        let cost = CostBreakdown {
            round_trip_cost: 0.004,
            ..CostBreakdown::default()
        };

        assert_eq!(min_holding_periods(&spot, &cost), 1);
        assert!((net_single_yield(&spot, &cost, 1) - 0.006).abs() < 1e-12);

        spot.single_yield = 0.003;
        assert_eq!(min_holding_periods(&spot, &cost), u32::MAX);
    }

    #[test]
    fn perp_price_spread_uses_only_the_proven_convergence_projection() {
        let mut perp = raw(
            "BTC",
            ArbitrageType::CrossExchange,
            StrategyKind::PerpPriceSpread,
        );
        perp.single_yield = 0.01;
        let cost = CostBreakdown {
            round_trip_cost: 0.0028,
            ..CostBreakdown::default()
        };

        assert_eq!(
            yield_realization(&perp),
            YieldRealization::ConvergenceSpread
        );
        assert_eq!(expected_gross_yield(&perp), 0.0);
        assert_eq!(min_holding_periods(&perp, &cost), 1);
        assert!((net_single_yield(&perp, &cost, 1) + 0.0028).abs() < 1e-12);

        perp.extra.price_spread_convergence = Some(crate::models::PriceSpreadConvergence {
            expected_gross_bps: 60.0,
            projected_net_bps: 32.0,
            evidence_ready: true,
            ..Default::default()
        });
        assert!((expected_gross_yield(&perp) - 0.006).abs() < 1e-12);
        assert!((net_single_yield(&perp, &cost, 1) - 0.0032).abs() < 1e-12);

        perp.extra.price_spread_convergence = Some(crate::models::PriceSpreadConvergence {
            expected_gross_bps: 80.0,
            projected_net_bps: 52.0,
            evidence_ready: false,
            ..Default::default()
        });
        assert_eq!(expected_gross_yield(&perp), 0.0);
        assert!((net_single_yield(&perp, &cost, 1) + 0.0028).abs() < 1e-12);
    }

    #[test]
    fn spot_perp_counts_basis_and_current_native_funding_once() {
        let mut spot = raw("BTC", ArbitrageType::SpotFutures, StrategyKind::SpotPerp);
        spot.extra.basis_spread = Some(0.01);
        spot.short_rate.rate = 0.001;
        spot.short_rate.rate_8h = 0.008;
        spot.single_yield = 0.011;
        let cost = CostBreakdown {
            round_trip_cost: 0.005,
            ..CostBreakdown::default()
        };

        assert_eq!(
            yield_realization(&spot),
            YieldRealization::ProjectedBasisCarry
        );
        assert!((gross_yield_at_horizon(&spot, 1) - 0.011).abs() < 1e-12);
        assert!((gross_yield_at_horizon(&spot, 8) - 0.011).abs() < 1e-12);
        assert_eq!(min_holding_periods(&spot, &cost), 1);
        assert!((net_single_yield(&spot, &cost, 1) - 0.006).abs() < 1e-12);
    }

    #[test]
    fn cross_spot_perp_never_repeats_the_opening_basis() {
        let mut cross = raw(
            "BTC",
            ArbitrageType::CrossSpotFutures,
            StrategyKind::CrossSpotPerp,
        );
        cross.extra.basis_spread = Some(0.01);
        cross.short_rate.rate = 0.001;
        cross.short_rate.rate_8h = 0.008;
        cross.single_yield = 0.011;
        let cost = CostBreakdown {
            round_trip_cost: 0.005,
            ..CostBreakdown::default()
        };

        assert_eq!(
            yield_realization(&cross),
            YieldRealization::ProjectedBasisCarry
        );
        assert!((gross_yield_at_horizon(&cross, 8) - 0.011).abs() < 1e-12);
        assert!((net_single_yield(&cross, &cost, 8) - 0.006).abs() < 1e-12);
    }

    #[test]
    fn perp_cross_ignores_normalized_rate_and_uses_native_joint_event_edge() {
        let mut perp = raw("BTC", ArbitrageType::CrossExchange, StrategyKind::PerpCross);
        perp.long_rate.rate = 0.0001;
        perp.long_rate.rate_8h = 0.0008;
        perp.long_rate.funding_interval = 1;
        perp.long_rate.next_funding_time = 3_600_000;
        perp.short_rate.rate = 0.001;
        perp.short_rate.rate_8h = 0.001;
        perp.short_rate.funding_interval = 8;
        perp.short_rate.next_funding_time = 28_800_000;
        perp.spread_8h = 0.0002;
        perp.single_yield = 0.0009;

        assert!((scan_edge(&perp) - 0.0009).abs() < 1e-12);
        assert!((expected_gross_yield(&perp) - 0.0009).abs() < 1e-12);
        assert_eq!(min_holding_periods(&perp, &CostBreakdown::default()), 1);
    }

    fn raw(symbol: &str, arb_type: ArbitrageType, kind: StrategyKind) -> RawOpportunity {
        RawOpportunity {
            symbol: symbol.into(),
            arb_type,
            long_exchange: "a".into(),
            short_exchange: "b".into(),
            long_rate: rate("a", symbol),
            short_rate: rate("b", symbol),
            spread_8h: 0.0,
            single_yield: 0.0,
            extra: crate::models::RawOpportunityExtra {
                strategy_kind: Some(kind),
                ..Default::default()
            },
        }
    }

    fn rate(exchange: &str, symbol: &str) -> FundingRateData {
        FundingRateData {
            symbol: symbol.into(),
            exchange: exchange.into(),
            rate: 0.0,
            rate_8h: 0.0,
            predicted_rate: None,
            next_funding_time: 0,
            funding_interval: 8,
            volume_24h: 0.0,
            timestamp: 0,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        }
    }
}
