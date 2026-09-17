use super::*;
use shared_types::{ExecutionMode, MarginMode, OrderSource, TimeInForce};

fn intent(side: OrderSide, reduce_only: bool) -> OrderIntent {
    OrderIntent {
        id: "intent-1".to_owned(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: "bitget".to_owned(),
        symbol: "BTCUSDT".to_owned(),
        side,
        order_type: OrderType::Limit,
        quantity: 0.002,
        price: Some(50_000.0),
        slippage_tolerance_bps: None,
        reduce_only,
        time_in_force: TimeInForce::Gtc,
        post_only: false,
        margin_mode: MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "client-1".to_owned(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

fn spec(category: BitgetUtaCategory) -> BitgetInstrumentSpec {
    BitgetInstrumentSpec {
        category,
        native_symbol: "BTCUSDT".to_owned(),
        price_tick: 0.1,
        qty_step: 0.001,
        min_qty: 0.001,
        min_notional: Some(5.0),
        execution_supported: true,
        listing_status: InstrumentListingStatus::Trading,
    }
}

#[test]
fn hedge_mode_derives_all_open_close_pos_side_combinations() {
    for (side, reduce_only, expected) in [
        (OrderSide::Buy, false, "long"),
        (OrderSide::Sell, false, "short"),
        (OrderSide::Sell, true, "long"),
        (OrderSide::Buy, true, "short"),
    ] {
        let compiled = compile_order(
            &intent(side, reduce_only),
            &spec(BitgetUtaCategory::UsdtFutures),
            BitgetPositionMode::Hedge,
        )
        .expect("hedge compile");
        assert_eq!(compiled.pos_side, Some(expected));
        assert!(!compiled.reduce_only);
    }
}

#[test]
fn one_way_close_uses_reduce_only_without_pos_side() {
    let compiled = compile_order(
        &intent(OrderSide::Sell, true),
        &spec(BitgetUtaCategory::UsdcFutures),
        BitgetPositionMode::OneWay,
    )
    .expect("one-way compile");
    assert_eq!(compiled.pos_side, None);
    assert!(compiled.reduce_only);
    assert_eq!(compiled.category, BitgetUtaCategory::UsdcFutures);
}

#[test]
fn inverse_reality_and_misaligned_orders_fail_closed() {
    let mut inverse = spec(BitgetUtaCategory::CoinFutures);
    inverse.execution_supported = false;
    assert!(compile_order(
        &intent(OrderSide::Buy, false),
        &inverse,
        BitgetPositionMode::OneWay
    )
    .is_err());

    let mut bad = intent(OrderSide::Buy, false);
    bad.quantity = 0.0015;
    assert!(compile_order(
        &bad,
        &spec(BitgetUtaCategory::UsdtFutures),
        BitgetPositionMode::OneWay
    )
    .is_err());
}
