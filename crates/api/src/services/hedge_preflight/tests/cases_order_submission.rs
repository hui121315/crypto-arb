use super::super::*;
use super::fixtures::plan;
use shared_types::{
    ExecutionMode, MarginMode, OrderIntent, OrderSide, OrderSource, OrderType, TimeInForce,
};

#[test]
fn reduce_only_close_defers_remote_preflight_to_the_submission_adapter() {
    let mut intent = intent();
    intent.reduce_only = true;

    assert!(!requires_remote_order_preflight(&intent));
}

#[test]
fn exposure_increasing_order_keeps_remote_preflight() {
    assert!(requires_remote_order_preflight(&intent()));
}

#[test]
fn account_mode_preflight_matches_verified_new_venue_boundaries() {
    assert!(account_mode_plan(
        ExecutionMode::Live,
        &plan(
            "gate_crossex:binance",
            "BINANCE_FUTURE_BTC_USDT",
            Vec::new()
        )
    ));
    assert!(!account_mode_plan(
        ExecutionMode::Live,
        &plan("kraken", "PF_XBTUSD", Vec::new())
    ));
}

fn intent() -> OrderIntent {
    OrderIntent {
        id: "preflight-1".to_owned(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: "binance".to_owned(),
        symbol: "SOL".to_owned(),
        side: OrderSide::Sell,
        order_type: OrderType::Market,
        quantity: 0.1,
        price: Some(100.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        margin_mode: MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "preflight-1".to_owned(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}
