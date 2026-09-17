use super::*;
use crate::adapters::binance_exchange_info::{
    BinanceInstrumentIdentity, BinanceOrderCapabilities, BinancePriceConstraints,
};
use pretty_assertions::assert_eq;
use shared_types::{ExecutionMode, OrderSide, OrderSource, TimeInForce};

#[test]
fn rest_limit_order_params_match_binance_schema() {
    assert_eq!(
        crate::adapters::binance_private_rest::ORDER_PATH,
        "/fapi/v1/order"
    );

    let params = rest_place_order_params(&intent(OrderType::Limit), "BTCUSDT", "BOTH")
        .expect("params build");

    assert!(contains(&params, "symbol", "BTCUSDT"));
    assert!(contains(&params, "side", "BUY"));
    assert!(contains(&params, "positionSide", "BOTH"));
    assert!(contains(&params, "type", "LIMIT"));
    assert!(contains(&params, "quantity", "0.01"));
    assert!(contains(&params, "price", "50000"));
    assert!(contains(&params, "timeInForce", "GTC"));
    assert!(contains(&params, "newClientOrderId", "cid-1"));
    assert!(contains(&params, "newOrderRespType", "RESULT"));
}

#[test]
fn rest_place_order_validates_official_client_order_id_rule() {
    let mut order = intent(OrderType::Limit);
    order.client_order_id = "ABC.def:/xyz_09-1".into();
    let params = rest_place_order_params(&order, "BTCUSDT", "BOTH").expect("official chars pass");
    assert!(contains(&params, "newClientOrderId", "ABC.def:/xyz_09-1"));

    order.client_order_id = String::new();
    assert!(matches!(
        rest_place_order_params(&order, "BTCUSDT", "BOTH"),
        Err(ExchangeError::Api { .. })
    ));

    order.client_order_id = "a".repeat(37);
    assert!(matches!(
        rest_place_order_params(&order, "BTCUSDT", "BOTH"),
        Err(ExchangeError::Api { .. })
    ));

    order.client_order_id = "bad#id".into();
    assert!(matches!(
        rest_place_order_params(&order, "BTCUSDT", "BOTH"),
        Err(ExchangeError::Api { .. })
    ));
}

#[test]
fn rest_market_order_omits_price_and_tif() {
    let params = rest_place_order_params(&intent(OrderType::Market), "ETHUSDT", "BOTH")
        .expect("params build");

    assert!(contains(&params, "type", "MARKET"));
    assert!(!params.iter().any(|(key, _)| key == "price"));
    assert!(!params.iter().any(|(key, _)| key == "timeInForce"));
}

#[test]
fn post_only_uses_gtx() {
    let params = rest_place_order_params(&intent(OrderType::PostOnly), "BTCUSDT", "BOTH")
        .expect("params build");

    assert!(contains(&params, "timeInForce", "GTX"));
}

#[test]
fn limit_ioc_and_fok_preserve_time_in_force() {
    let mut ioc = intent(OrderType::Limit);
    ioc.time_in_force = TimeInForce::Ioc;
    let ioc_params = rest_place_order_params(&ioc, "BTCUSDT", "BOTH").expect("ioc params");
    assert!(contains(&ioc_params, "timeInForce", "IOC"));

    let mut fok = intent(OrderType::Limit);
    fok.time_in_force = TimeInForce::Fok;
    let fok_params = rest_place_order_params(&fok, "BTCUSDT", "BOTH").expect("fok params");
    assert!(contains(&fok_params, "timeInForce", "FOK"));
}

#[test]
fn limit_gtx_uses_official_gtx_time_in_force() {
    let mut order = intent(OrderType::Limit);
    order.time_in_force = TimeInForce::Gtx;
    let params = rest_place_order_params(&order, "BTCUSDT", "BOTH").expect("gtx params");

    assert!(contains(&params, "type", "LIMIT"));
    assert!(contains(&params, "timeInForce", "GTX"));
}

#[test]
fn reduce_only_is_serialized() {
    let mut order = intent(OrderType::Limit);
    order.reduce_only = true;
    let params = rest_place_order_params(&order, "BTCUSDT", "BOTH").expect("params build");

    assert!(contains(&params, "reduceOnly", "true"));
}

#[test]
fn position_side_is_serialized_from_verified_mode() {
    let buy = intent(OrderType::Limit);
    let buy_params = rest_place_order_params(&buy, "BTCUSDT", "LONG").expect("long side");
    assert!(contains(&buy_params, "positionSide", "LONG"));

    let mut sell = intent(OrderType::Limit);
    sell.side = OrderSide::Sell;
    let sell_params = rest_place_order_params(&sell, "BTCUSDT", "SHORT").expect("short side");
    assert!(contains(&sell_params, "positionSide", "SHORT"));
}

#[test]
fn hedge_mode_close_uses_position_side_and_omits_reduce_only() {
    let mut order = intent(OrderType::Limit);
    order.reduce_only = true;
    order.side = OrderSide::Sell;
    let params =
        rest_place_order_params(&order, "BTCUSDT", "LONG").expect("verified hedge long close");

    assert!(contains(&params, "positionSide", "LONG"));
    assert!(!params.iter().any(|(key, _)| key == "reduceOnly"));
}

#[test]
fn hedge_mode_close_rejects_side_mismatch() {
    let mut order = intent(OrderType::Limit);
    order.reduce_only = true;
    let err = rest_place_order_params(&order, "BTCUSDT", "LONG")
        .expect_err("buy cannot close a hedge-mode long");

    assert!(matches!(
        err,
        ExchangeError::Api { message, .. } if message.contains("side mismatch")
    ));
}

#[test]
fn limit_without_positive_price_is_rejected() {
    let mut order = intent(OrderType::Limit);
    order.price = None;
    assert!(matches!(
        rest_place_order_params(&order, "BTCUSDT", "BOTH"),
        Err(ExchangeError::Api { .. })
    ));

    order.price = Some(0.0);
    assert!(matches!(
        rest_place_order_params(&order, "BTCUSDT", "BOTH"),
        Err(ExchangeError::Api { .. })
    ));
}

#[test]
fn cancel_params_use_orig_client_order_id() {
    assert_eq!(
        crate::adapters::binance_private_rest::ORDER_PATH,
        "/fapi/v1/order"
    );

    let params = rest_cancel_order_params("cid-1", "BTCUSDT");

    assert!(contains(&params, "symbol", "BTCUSDT"));
    assert!(contains(&params, "origClientOrderId", "cid-1"));
    assert!(contains(&params, "recvWindow", "5000"));
}

#[test]
fn order_constraints_validate_min_notional() {
    let mut order = intent(OrderType::Limit);
    order.quantity = 0.001;
    order.price = Some(50.0);
    let constraints = BinanceOrderConstraints {
        min_notional: Some(1.0),
        ..tradable_constraints()
    };

    assert!(matches!(
        validate_order_constraints("BTCUSDT", &order, &constraints),
        Err(ExchangeError::Api { .. })
    ));
}

#[test]
fn order_constraints_reject_non_trading_symbol() {
    let constraints = BinanceOrderConstraints {
        capabilities: BinanceOrderCapabilities {
            is_trading: false,
            ..tradable_constraints().capabilities
        },
        ..tradable_constraints()
    };

    let err = validate_order_constraints("BTCUSDT", &intent(OrderType::Limit), &constraints)
        .expect_err("non-trading symbols must be blocked");

    assert!(matches!(
        err,
        ExchangeError::Api { message, .. } if message.contains("not TRADING")
    ));
}

#[test]
fn order_constraints_reject_non_perpetual_contract() {
    let constraints = BinanceOrderConstraints {
        capabilities: BinanceOrderCapabilities {
            is_perpetual: false,
            ..tradable_constraints().capabilities
        },
        ..tradable_constraints()
    };

    let err = validate_order_constraints("BTCUSDT", &intent(OrderType::Limit), &constraints)
        .expect_err("non-perpetual contracts must be blocked");

    assert!(matches!(
        err,
        ExchangeError::Api { message, .. } if message.contains("not PERPETUAL")
    ));
}

#[test]
fn order_constraints_reject_missing_asset_identity() {
    let constraints = BinanceOrderConstraints {
        identity: BinanceInstrumentIdentity {
            base_asset: String::new(),
            ..tradable_constraints().identity
        },
        ..tradable_constraints()
    };

    let err = validate_order_constraints("BTCUSDT", &intent(OrderType::Limit), &constraints)
        .expect_err("missing asset identity must be blocked");

    assert!(matches!(
        err,
        ExchangeError::Api { message, .. } if message.contains("baseAsset/quoteAsset/marginAsset")
    ));
}

#[test]
fn order_constraints_reject_asset_symbol_mismatch() {
    let constraints = BinanceOrderConstraints {
        identity: BinanceInstrumentIdentity {
            base_asset: "ETH".to_owned(),
            ..tradable_constraints().identity
        },
        ..tradable_constraints()
    };

    let err = validate_order_constraints("BTCUSDT", &intent(OrderType::Limit), &constraints)
        .expect_err("asset mismatch must be blocked");

    assert!(matches!(
        err,
        ExchangeError::Api { message, .. } if message.contains("baseAsset/quoteAsset")
    ));
}

#[test]
fn order_constraints_reject_unsupported_order_type_and_tif() {
    let market_constraints = BinanceOrderConstraints {
        capabilities: BinanceOrderCapabilities {
            supports_market: false,
            ..tradable_constraints().capabilities
        },
        ..tradable_constraints()
    };
    let market_err =
        validate_order_constraints("BTCUSDT", &intent(OrderType::Market), &market_constraints)
            .expect_err("unsupported market order must be blocked");
    assert!(matches!(
        market_err,
        ExchangeError::Api { message, .. } if message.contains("MARKET")
    ));

    let ioc_constraints = BinanceOrderConstraints {
        capabilities: BinanceOrderCapabilities {
            supports_ioc: false,
            ..tradable_constraints().capabilities
        },
        ..tradable_constraints()
    };
    let mut ioc = intent(OrderType::Limit);
    ioc.time_in_force = TimeInForce::Ioc;
    let ioc_err = validate_order_constraints("BTCUSDT", &ioc, &ioc_constraints)
        .expect_err("unsupported IOC must be blocked");
    assert!(matches!(
        ioc_err,
        ExchangeError::Api { message, .. } if message.contains("IOC")
    ));
}

#[test]
fn limit_order_uses_lot_size_quantity_filter() {
    let constraints = BinanceOrderConstraints {
        limit_qty: qty_constraints(Some(0.001), Some(100.0), Some(0.001)),
        market_qty: qty_constraints(Some(0.001), Some(0.005), Some(0.001)),
        ..tradable_constraints()
    };

    validate_order_constraints("BTCUSDT", &intent(OrderType::Limit), &constraints)
        .expect("limit order must use LOT_SIZE, not MARKET_LOT_SIZE");
}

#[test]
fn market_order_uses_market_lot_size_quantity_filter() {
    let constraints = BinanceOrderConstraints {
        limit_qty: qty_constraints(Some(0.001), Some(100.0), Some(0.001)),
        market_qty: qty_constraints(Some(0.001), Some(0.005), Some(0.001)),
        ..tradable_constraints()
    };

    let err = validate_order_constraints("BTCUSDT", &intent(OrderType::Market), &constraints)
        .expect_err("market order must use MARKET_LOT_SIZE");

    assert!(matches!(
        err,
        ExchangeError::Api { message, .. } if message.contains("MARKET_LOT_SIZE quantity")
    ));
}

#[test]
fn market_order_rejects_missing_market_lot_size_filter() {
    let constraints = BinanceOrderConstraints {
        market_qty: BinanceQuantityConstraints::default(),
        ..tradable_constraints()
    };

    let err = validate_order_constraints("BTCUSDT", &intent(OrderType::Market), &constraints)
        .expect_err("missing MARKET_LOT_SIZE must be fail-closed");

    assert!(matches!(
        err,
        ExchangeError::Api { message, .. } if message.contains("missing MARKET_LOT_SIZE")
    ));
}

#[test]
fn order_spec_requires_complete_official_sizing_metadata() {
    let mut constraints = tradable_constraints();
    constraints.min_notional = Some(5.0);
    constraints.price.tick_size = None;
    let spec = instrument_spec(constraints);

    let err = validate_order_spec(&intent(OrderType::Limit), &spec)
        .expect_err("missing price tick must be blocked");

    assert!(matches!(
        err,
        ExchangeError::Api { message, .. } if message.contains("missing valid price tick")
    ));
}

#[test]
fn order_spec_requires_quantity_step_and_min_notional() {
    let mut missing_step = tradable_constraints();
    missing_step.min_notional = Some(5.0);
    missing_step.limit_qty.step_size = None;
    let step_err = validate_order_spec(&intent(OrderType::Limit), &instrument_spec(missing_step))
        .expect_err("missing quantity step must be blocked");
    assert!(matches!(
        step_err,
        ExchangeError::Api { message, .. } if message.contains("missing valid quantity step")
    ));

    let missing_notional = instrument_spec(tradable_constraints());
    let notional_err = validate_order_spec(&intent(OrderType::Limit), &missing_notional)
        .expect_err("missing min notional must be blocked");
    assert!(matches!(
        notional_err,
        ExchangeError::Api { message, .. } if message.contains("missing valid min notional")
    ));
}

#[test]
fn order_spec_rejects_quote_margin_mismatch() {
    let mut constraints = tradable_constraints();
    constraints.identity.quote_asset = "USDC".to_owned();
    constraints.min_notional = Some(5.0);
    let mut spec = instrument_spec(constraints);
    spec.native_symbol = "BTCUSDC".to_owned();

    let err = validate_order_spec(&intent(OrderType::Limit), &spec)
        .expect_err("quote and margin mismatch must be blocked");

    assert!(matches!(
        err,
        ExchangeError::Api { message, .. } if message.contains("does not match quoteAsset")
    ));
}

#[test]
fn ack_maps_new_status_to_accepted() {
    let item = OpenOrderItem {
        order_id: 7,
        symbol: "BTCUSDT".into(),
        status: "NEW".into(),
        order_type: "LIMIT".into(),
        side: "BUY".into(),
        price: "50000".into(),
        orig_qty: "0.01".into(),
        executed_qty: "0".into(),
        avg_price: "0".into(),
        time: Some(1),
        update_time: None,
        time_in_force: "GTC".into(),
        client_order_id: String::new(),
        reduce_only: false,
    };

    let ack = ack_from_order_item("int-1".into(), "cid-1".into(), &item);

    assert_eq!(ack.exchange_order_id.as_deref(), Some("7"));
    assert_eq!(ack.client_order_id, "cid-1");
    assert_eq!(
        ack.identity_update.venue_client_order_id.as_deref(),
        Some("cid-1")
    );
    assert_eq!(ack.state, LiveOrderState::Accepted);
}

fn contains(params: &RestOrderParams, key: &str, value: &str) -> bool {
    params.iter().any(|(k, v)| k == key && v == value)
}

fn tradable_constraints() -> BinanceOrderConstraints {
    BinanceOrderConstraints {
        identity: BinanceInstrumentIdentity {
            base_asset: "BTC".to_owned(),
            quote_asset: "USDT".to_owned(),
            margin_asset: "USDT".to_owned(),
            underlying_type: "COIN".to_owned(),
        },
        capabilities: BinanceOrderCapabilities {
            is_trading: true,
            is_registry_perpetual: true,
            is_perpetual: true,
            supports_limit: true,
            supports_market: true,
            supports_gtc: true,
            supports_ioc: true,
            supports_fok: true,
            supports_gtx: true,
        },
        price: BinancePriceConstraints {
            min_price: Some(0.1),
            max_price: Some(1_000_000.0),
            tick_size: Some(0.1),
        },
        limit_qty: qty_constraints(Some(0.001), Some(100.0), Some(0.001)),
        market_qty: qty_constraints(Some(0.001), Some(50.0), Some(0.001)),
        ..Default::default()
    }
}

fn instrument_spec(constraints: BinanceOrderConstraints) -> BinanceInstrumentSpec {
    BinanceInstrumentSpec {
        native_symbol: "BTCUSDT".to_owned(),
        contract_size: 1.0,
        constraints,
    }
}

fn qty_constraints(
    min_qty: Option<f64>,
    max_qty: Option<f64>,
    step_size: Option<f64>,
) -> BinanceQuantityConstraints {
    BinanceQuantityConstraints {
        present: true,
        min_qty,
        max_qty,
        step_size,
    }
}

fn intent(order_type: OrderType) -> OrderIntent {
    OrderIntent {
        id: "int-1".into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: "binance".into(),
        symbol: "BTC".into(),
        side: OrderSide::Buy,
        order_type,
        quantity: 0.01,
        price: Some(50_000.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: shared_types::TimeInForce::Gtc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "cid-1".into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

#[test]
fn binance_place_order_ack_parses_official_fixture() {
    let fixture = include_str!("../../fixtures/binance/usdm_place_order_ack.json");
    let item: crate::adapters::binance_private_data::OpenOrderItem = serde_json::from_str(fixture)
        .expect("official binance USDM new-order RESULT fixture parses");
    let ack = ack_from_order_item(
        "internal-place-1".to_owned(),
        item.client_order_id.clone(),
        &item,
    );
    assert_eq!(ack.exchange_order_id.as_deref(), Some("283194212"));
    assert!(matches!(ack.state, LiveOrderState::Accepted));
    assert_eq!(ack.client_order_id, "x-crossline-cid-1");
}

#[test]
fn binance_cancel_order_ack_parses_official_fixture() {
    let fixture = include_str!("../../fixtures/binance/usdm_cancel_order_ack.json");
    let item: crate::adapters::binance_private_data::OpenOrderItem = serde_json::from_str(fixture)
        .expect("official binance USDM cancel-order RESULT fixture parses");
    let ack = ack_from_order_item(
        "internal-cancel-1".to_owned(),
        item.client_order_id.clone(),
        &item,
    );
    assert_eq!(ack.exchange_order_id.as_deref(), Some("283194212"));
    assert!(matches!(ack.state, LiveOrderState::Cancelled));
    assert_eq!(ack.client_order_id, "myOrder1");
}
