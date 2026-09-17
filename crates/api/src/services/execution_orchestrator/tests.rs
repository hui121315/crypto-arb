use super::evidence::orderbook_depth_input;
use super::*;
use shared_types::{
    ExecutionLedgerQuality, ExecutionMode, HedgeLegQuote, MarketDataHealth, MarketDataQuality,
    MarketDataSourceKind, OrderSource,
};

mod dry_run;
mod leg_record;
mod run_state;
mod unwind;

#[test]
fn orderbook_depth_input_preserves_ticket_depth_evidence() {
    let leg = HedgeLegQuote {
        role: HedgeLegRole::Long,
        exchange: "okx".into(),
        symbol: "BTCUSDT".into(),
        side: OrderSide::Buy,
        reference_price: Some(100.0),
        bid: Some(99.9),
        ask: Some(100.1),
        mid: Some(100.0),
        open_vwap_price: Some(100.2),
        open_slippage_bps: Some(2.0),
        close_vwap_price: Some(99.8),
        close_slippage_bps: Some(2.0),
        depth_usd_5bps: Some(500.0),
        depth_usd_10bps: Some(1000.0),
        depth_usd_20bps: Some(1500.0),
        max_notional_usd: Some(1500.0),
        market_evidence: None,
        depth_health: Some(MarketDataHealth {
            quality: MarketDataQuality::Fresh,
            source: MarketDataSourceKind::WsPush,
            freshness_ms: Some(3),
            retry_after_ms: None,
            last_error: None,
            observed_at_ms: 42,
            coverage: None,
            problem: None,
        }),
        depth_reason: None,
        funding_bps: None,
        next_funding_time: 0,
        funding_interval_hours: 0,
        market_timestamp_ms: Some(42),
        blockers: Vec::new(),
    };

    let input = orderbook_depth_input(&leg);

    assert_eq!(input.max_notional_usd, Some(1500.0));
    assert_eq!(input.market_timestamp_ms, Some(42));
    assert_eq!(input.quality, ExecutionLedgerQuality::Actual);
}

pub(super) fn order_record(state: LiveOrderState) -> OrderRecord {
    OrderRecord {
        intent: OrderIntent {
            id: "open".into(),
            source: OrderSource::ArbitragePreview,
            strategy: None,
            mode: ExecutionMode::DryRun,
            exchange: "okx".into(),
            symbol: "BTC".into(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            quantity: 1.0,
            price: Some(100.0),
            slippage_tolerance_bps: None,
            reduce_only: false,
            time_in_force: shared_types::TimeInForce::Ioc,
            post_only: false,
            margin_mode: shared_types::MarginMode::Cross,
            leverage: 1.0,
            client_order_id: "open-client".into(),
            client_order_id_policy: None,
            created_at_ms: 1,
        },
        state,
        risk: None,
        identity: Default::default(),
        last_update_source: Default::default(),
        exchange_order_id: None,
        message: None,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
        updated_at_ms: 1,
    }
}

pub(super) fn leg_with_fee(role: HedgeLegRole, filled_fee: Option<f64>) -> ExecutionRunLeg {
    ExecutionRunLeg {
        role,
        exchange: "okx".into(),
        symbol: "BTC".into(),
        order_ids: vec!["order-1".into()],
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        state: LiveOrderState::Filled,
        target_quantity: 1.0,
        filled_quantity: Some(1.0),
        target_notional_usd: 100.0,
        filled_notional_usd: Some(100.0),
        filled_fee,
    }
}

pub(super) fn assert_close_option(actual: Option<f64>, expected: f64) -> Result<(), &'static str> {
    let actual = actual.ok_or("missing actual value")?;
    if (actual - expected).abs() < 1e-9 {
        Ok(())
    } else {
        Err("actual value differs from expected")
    }
}
