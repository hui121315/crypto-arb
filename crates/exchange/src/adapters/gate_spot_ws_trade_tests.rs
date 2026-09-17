use super::*;
use shared_types::{ExecutionMode, MarginMode, OrderSide, OrderSource, OrderType, TimeInForce};

#[test]
fn spot_place_cancel_and_query_use_official_api_channels() {
    let intent = intent();
    let compiled = compiled();
    let place = api_request(
        CHANNEL_ORDER_PLACE,
        &intent.id,
        place_params(&intent, &compiled).expect("place params"),
    );
    assert_eq!(place.channel, CHANNEL_ORDER_PLACE);
    assert_eq!(
        place.payload.req_param.as_ref().expect("params")["account"],
        "spot"
    );
    assert_eq!(
        place.payload.req_param.as_ref().expect("params")["currency_pair"],
        "SOL_USDT"
    );

    let cancel = CancelOrderRequest {
        exchange: EXCHANGE.into(),
        symbol: "SOL".into(),
        internal_order_id: "cancel-1".into(),
        exchange_order_id: Some("7".into()),
        client_order_id: "cid-1".into(),
    };
    let cancel = api_request(
        CHANNEL_ORDER_CANCEL,
        &cancel.internal_order_id,
        cancel_params(&cancel, "SOL_USDT").expect("cancel params"),
    );
    assert_eq!(cancel.channel, CHANNEL_ORDER_CANCEL);
    assert_eq!(
        cancel.payload.req_param.as_ref().expect("params")["order_id"],
        "7"
    );

    let query = api_request(
        CHANNEL_ORDER_STATUS,
        "query-1",
        status_params("7", "SOL_USDT").expect("status params"),
    );
    assert_eq!(query.channel, CHANNEL_ORDER_STATUS);
    assert_eq!(
        query.payload.req_param.as_ref().expect("params")["currency_pair"],
        "SOL_USDT"
    );
}

fn compiled() -> CompiledSpotOrder {
    CompiledSpotOrder {
        native_symbol: "SOL_USDT".into(),
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
