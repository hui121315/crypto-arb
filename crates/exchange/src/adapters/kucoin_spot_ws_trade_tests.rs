use super::*;
use shared_types::{ExecutionMode, MarginMode, OrderSide, OrderSource, OrderType, TimeInForce};

#[test]
fn pro_spot_place_and_cancel_payloads_preserve_identity() {
    let intent = intent();
    let place: Value = serde_json::from_str(
        &pro_spot_order_payload(
            &intent.id,
            place_args(&intent, &compiled()).expect("place args"),
        )
        .expect("place payload"),
    )
    .expect("place json");
    assert_eq!(place["op"], "spot.order");
    assert_eq!(place["args"]["symbol"], "SOL-USDT");
    assert_eq!(place["args"]["clientOid"], "cid-1");

    let cancel = CancelOrderRequest {
        exchange: EXCHANGE.into(),
        symbol: "SOL".into(),
        internal_order_id: "cancel-1".into(),
        exchange_order_id: Some("7".into()),
        client_order_id: "cid-1".into(),
    };
    let cancel: Value = serde_json::from_str(
        &pro_spot_cancel_payload(
            &cancel.internal_order_id,
            cancel_args(&cancel, "SOL-USDT").expect("cancel args"),
        )
        .expect("cancel payload"),
    )
    .expect("cancel json");
    assert_eq!(cancel["op"], "spot.cancel");
    assert_eq!(cancel["args"]["symbol"], "SOL-USDT");
    assert_eq!(cancel["args"]["orderId"], "7");
}

fn compiled() -> CompiledSpotOrder {
    CompiledSpotOrder {
        native_symbol: "SOL-USDT".into(),
        quantity: "0.1".into(),
        quote_notional: "15".into(),
        price: Some("150".into()),
    }
}

fn intent() -> OrderIntent {
    OrderIntent {
        id: "place-1".into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: EXCHANGE.into(),
        symbol: "SOL".into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 0.1,
        price: Some(150.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Gtc,
        post_only: false,
        margin_mode: MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "cid-1".into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}
