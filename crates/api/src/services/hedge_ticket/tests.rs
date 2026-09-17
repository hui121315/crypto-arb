use super::*;
use arbitrage::models::{RawOpportunity, RawOpportunityExtra};
use arbitrage::{CostBreakdown, OpportunityBuilder, PositionSizing, RiskMetrics};
use shared_types::TradeFeeSource;
use std::collections::BTreeSet;

mod cost_evidence;
mod depth;
mod evidence;
mod execution_order;
mod pr_ak;

fn book(bids: Vec<[f64; 2]>, asks: Vec<[f64; 2]>) -> OrderBookInfo {
    OrderBookInfo {
        symbol: "BTC".into(),
        exchange: "mock".into(),
        bids,
        asks,
        timestamp: 1,
    }
}

fn leg_spec(exchange: &str, symbol: &str) -> LegSpec {
    LegSpec {
        exchange: exchange.into(),
        symbol: symbol.into(),
        book_kind: LegBookKind::Perp,
        side: OrderSide::Buy,
        fallback_price: Some(1.0),
        funding_rate: 0.0,
        next_funding_time: 0,
        funding_interval_hours: 0,
    }
}

fn empty_leg() -> HedgeLegQuote {
    HedgeLegQuote {
        role: HedgeLegRole::Long,
        exchange: "hyperliquid:xyz".into(),
        symbol: "SNDK".into(),
        side: OrderSide::Buy,
        reference_price: None,
        bid: None,
        ask: None,
        mid: None,
        open_vwap_price: None,
        open_slippage_bps: None,
        close_vwap_price: None,
        close_slippage_bps: None,
        depth_usd_5bps: None,
        depth_usd_10bps: None,
        depth_usd_20bps: None,
        max_notional_usd: None,
        market_evidence: None,
        depth_health: None,
        depth_reason: None,
        funding_bps: None,
        next_funding_time: 0,
        funding_interval_hours: 0,
        market_timestamp_ms: None,
        blockers: Vec::new(),
    }
}

fn base_cost_profile() -> ExecutionCostProfile {
    ExecutionCostProfile {
        gross_edge_bps: 30.0,
        fee_bps: 0.0,
        wear_bps: 8.0,
        total_cost_bps: 8.0,
        one_cycle: OneCycleCostProfile {
            gross_edge_bps: 30.0,
            open_fee_bps: 0.0,
            close_fee_bps: 0.0,
            open_slippage_bps: 4.0,
            close_slippage_bps: 4.0,
            funding_window_mismatch_buffer_bps: 0.0,
            yield_basis: None,
            long_next_settlement_ms: None,
            short_next_settlement_ms: None,
            target_buffer_bps: 0.0,
            net_bps: 22.0,
            covers_round_trip_cost: true,
        },
        breakeven_periods: 1,
        breakeven_hours: 8.0,
        recommended_hold_periods: 2,
        recommended_hold_hours: 16.0,
        net_bps_at_recommended_hold: 52.0,
        round_trip: None,
    }
}

fn opportunity_with_prices(
    long_price: Option<f64>,
    short_price: Option<f64>,
) -> ArbitrageOpportunityDto {
    let raw = RawOpportunity {
        symbol: "BTC".into(),
        arb_type: ArbitrageType::CrossExchange,
        long_exchange: "binance".into(),
        short_exchange: "okx".into(),
        long_rate: funding("BTC", "binance"),
        short_rate: funding("BTC", "okx"),
        spread_8h: 0.001,
        single_yield: 0.001,
        extra: RawOpportunityExtra {
            strategy_kind: Some(StrategyKind::PerpCross),
            long_price,
            short_price,
            ..Default::default()
        },
    };
    OpportunityBuilder {
        raw: &raw,
        metrics: &RiskMetrics::default(),
        position: &PositionSizing::default(),
        cost: &CostBreakdown::default(),
        min_holding_periods: 1,
        net_single_yield: raw.single_yield,
        data_source: "test",
        confidence: 0.8,
    }
    .build()
}

#[test]
fn price_spread_strategies_use_their_actual_fee_products() {
    let mut opportunity = opportunity_with_prices(Some(100.0), Some(101.0));
    opportunity.strategy_kind = Some(StrategyKind::PerpPriceSpread);
    assert_eq!(
        fee_product_for(&opportunity, HedgeLegRole::Long),
        FeeProduct::Perp
    );
    assert_eq!(
        fee_product_for(&opportunity, HedgeLegRole::Short),
        FeeProduct::Perp
    );

    opportunity.strategy_kind = Some(StrategyKind::SpotCross);
    opportunity.arb_type = ArbitrageType::SpotCross;
    assert_eq!(
        fee_product_for(&opportunity, HedgeLegRole::Long),
        FeeProduct::Spot
    );
    assert_eq!(
        fee_product_for(&opportunity, HedgeLegRole::Short),
        FeeProduct::Spot
    );
}

#[test]
fn spot_cross_gross_edge_uses_target_size_executable_prices() {
    let mut cost = base_cost_profile();
    let mut long = empty_leg();
    long.open_vwap_price = Some(100.0);
    let mut short = empty_leg();
    short.role = HedgeLegRole::Short;
    short.side = OrderSide::Sell;
    short.open_vwap_price = Some(100.8);

    bind_executable_gross_edge(&mut cost, Some(StrategyKind::SpotCross), &long, &short, 1);

    assert!((cost.gross_edge_bps - 80.0).abs() < 1e-9);
    assert!((cost.one_cycle.gross_edge_bps - 80.0).abs() < 1e-9);
}

#[test]
fn leg_specs_bind_native_rates_to_native_settlement_events() {
    let mut opportunity = opportunity_with_prices(Some(100.0), Some(101.0));
    opportunity.long_rate = 0.000_2;
    opportunity.long_rate_8h = 0.001_6;
    opportunity.short_rate = 0.000_7;
    opportunity.short_rate_8h = 0.000_7;

    let long = LegSpec::from_opp(&opportunity, HedgeLegRole::Long);
    let short = LegSpec::from_opp(&opportunity, HedgeLegRole::Short);

    assert_eq!(long.funding_rate, 0.000_2);
    assert_eq!(short.funding_rate, 0.000_7);
}

fn funding(symbol: &str, exchange: &str) -> shared_types::FundingRateData {
    shared_types::FundingRateData {
        symbol: symbol.into(),
        exchange: exchange.into(),
        rate: 0.0,
        rate_8h: 0.0,
        predicted_rate: None,
        next_funding_time: 0,
        funding_interval: 8,
        volume_24h: 100_000.0,
        timestamp: 1,
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    }
}

fn fee_snapshot(
    venue: &str,
    product: FeeProduct,
    open_fee_bps: f64,
    close_fee_bps: f64,
) -> TradeFeeSnapshot {
    let now_ms = chrono::Utc::now().timestamp_millis();
    TradeFeeSnapshot {
        venue: venue.into(),
        symbol: "BTC".into(),
        product,
        account_id: None,
        maker_fee_bps: 1.0,
        taker_fee_bps: open_fee_bps.max(close_fee_bps),
        open_fee_bps,
        close_fee_bps,
        source: TradeFeeSource::AccountApi,
        fetched_at_ms: now_ms.saturating_sub(1_000),
        valid_until_ms: now_ms.saturating_add(60_000),
        freshness_ms: Some(0),
        evidence: None,
        verification_problem: None,
        note: None,
    }
}
