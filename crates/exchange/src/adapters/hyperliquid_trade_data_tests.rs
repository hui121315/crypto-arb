use super::*;
use pretty_assertions::assert_eq;

#[test]
fn order_action_matches_hyperliquid_compact_schema() {
    assert_eq!(EXCHANGE_PATH, "/exchange");

    let intent = test_intent(OrderType::PostOnly);
    let action = hyperliquid_order_action(&test_spec(0), &intent).expect("action");

    assert_eq!(action["type"], "order");
    assert_eq!(action["orders"][0]["a"], 0);
    assert_eq!(action["orders"][0]["b"], true);
    assert_eq!(action["orders"][0]["p"], "50000");
    assert_eq!(action["orders"][0]["s"], "0.01");
    assert_eq!(action["orders"][0]["r"], false);
    assert_eq!(action["orders"][0]["t"]["limit"]["tif"], "Alo");
    assert_eq!(
        action["orders"][0]["c"],
        policy_cloid("0x0000000000000000000000000000000a")
    );
}

#[test]
fn cancel_action_prefers_exchange_oid() {
    let request = test_cancel(Some("12345".into()));
    let action = hyperliquid_cancel_action(1, &request).expect("cancel");

    assert_eq!(action["type"], "cancel");
    assert_eq!(action["cancels"][0]["a"], 1);
    assert_eq!(action["cancels"][0]["o"], 12345);
}

#[test]
fn spot_order_and_cancel_preserve_spot_asset_id() {
    let spot_asset = 10_107;
    let place = hyperliquid_order_action(&test_spec(spot_asset), &test_intent(OrderType::Limit))
        .expect("spot order action");
    assert_eq!(place["orders"][0]["a"], spot_asset);
    assert_eq!(place["orders"][0]["r"], false);

    let cancel = hyperliquid_cancel_action(spot_asset, &test_cancel(Some("12345".into())))
        .expect("spot cancel action");
    assert_eq!(cancel["cancels"][0]["a"], spot_asset);
}

#[test]
fn cancel_action_falls_back_to_cloid() {
    let request = test_cancel(None);
    let action = hyperliquid_cancel_action(2, &request).expect("cancel");

    assert_eq!(action["type"], "cancelByCloid");
    assert_eq!(action["cancels"][0]["asset"], 2);
    assert_eq!(
        action["cancels"][0]["cloid"],
        policy_cloid("0x0000000000000000000000000000000a")
    );
}

#[test]
fn market_order_is_protected_ioc_limit() {
    let intent = test_intent(OrderType::Market);
    let action = hyperliquid_order_action(&test_spec(0), &intent).expect("market action");

    assert_eq!(action["orders"][0]["p"], "50000");
    assert_eq!(action["orders"][0]["t"]["limit"]["tif"], "Ioc");
}

#[test]
fn cloid_policy_passes_official_128_bit_hex_and_derives_public_ids() {
    let ok = policy_cloid("0xABCDEFabcdef12345678901234567890");
    assert_eq!(ok, "0xabcdefabcdef12345678901234567890");
    assert_eq!(ok.len(), 34);

    let derived = policy_cloid("client-order-1");
    assert_eq!(derived, "0xeb9a1d290f7f8020d7658c7985e7223c");
    assert!(is_policy_cloid(&derived));
    assert_eq!(policy_cloid("client-order-1"), derived);
    assert_ne!(policy_cloid("client-order-2"), derived);
}

#[test]
fn required_cloid_derives_format_from_public_id() {
    assert!(required_cloid("").is_err());
    assert!(required_cloid("   ").is_err());
    assert_eq!(
        required_cloid("client-order-1").unwrap(),
        "0xeb9a1d290f7f8020d7658c7985e7223c"
    );
    assert_eq!(
        required_cloid("  0xABCDEFabcdef12345678901234567890  ").unwrap(),
        "0xabcdefabcdef12345678901234567890"
    );
}

#[test]
fn order_and_cancel_actions_share_derived_cloid() {
    let mut intent = test_intent(OrderType::Limit);
    intent.client_order_id = "hedge-run-long".into();
    let action = hyperliquid_order_action(&test_spec(0), &intent).expect("order action");

    let mut cancel = test_cancel(None);
    cancel.client_order_id = "hedge-run-long".into();
    let cancel_action = hyperliquid_cancel_action(0, &cancel).expect("cancel action");

    assert_eq!(
        action["orders"][0]["c"],
        cancel_action["cancels"][0]["cloid"]
    );
    assert!(is_policy_cloid(action["orders"][0]["c"].as_str().unwrap()));
}

#[test]
fn limit_order_honors_supported_time_in_force() {
    let mut gtc = test_intent(OrderType::Limit);
    gtc.time_in_force = shared_types::TimeInForce::Gtc;
    let action = hyperliquid_order_action(&test_spec(0), &gtc).expect("gtc action");
    assert_eq!(action["orders"][0]["t"]["limit"]["tif"], "Gtc");

    let mut ioc = test_intent(OrderType::Limit);
    ioc.time_in_force = shared_types::TimeInForce::Ioc;
    let action = hyperliquid_order_action(&test_spec(0), &ioc).expect("ioc action");
    assert_eq!(action["orders"][0]["t"]["limit"]["tif"], "Ioc");
}

#[test]
fn limit_order_rejects_unsupported_time_in_force() {
    let mut fok = test_intent(OrderType::Limit);
    fok.time_in_force = shared_types::TimeInForce::Fok;
    let error =
        hyperliquid_order_action(&test_spec(0), &fok).expect_err("fok is not a hyperliquid tif");
    assert!(
        error.to_string().contains("no fok tif"),
        "unexpected error: {error}"
    );

    let mut gtx = test_intent(OrderType::Limit);
    gtx.time_in_force = shared_types::TimeInForce::Gtx;
    let error =
        hyperliquid_order_action(&test_spec(0), &gtx).expect_err("gtx is not a hyperliquid tif");
    assert!(
        error.to_string().contains("no gtx tif"),
        "unexpected error: {error}"
    );
}

#[test]
fn protected_market_rejects_non_ioc_time_in_force() {
    let mut intent = test_intent(OrderType::Market);
    intent.time_in_force = shared_types::TimeInForce::Gtc;

    let error = hyperliquid_order_action(&test_spec(0), &intent)
        .expect_err("market must remain a protected IOC limit");

    assert!(error.to_string().contains("requires ioc"));
}

#[test]
fn compiler_rejects_price_and_lot_precision_outside_official_metadata() {
    let mut price = test_intent(OrderType::Limit);
    price.price = Some(12_345.6);
    let error = hyperliquid_order_action(&test_spec(0), &price)
        .expect_err("six significant figures must fail");
    assert!(error.to_string().contains("five significant"));

    let mut lot = test_intent(OrderType::Limit);
    lot.quantity = 0.000_001;
    let error = hyperliquid_order_action(&test_spec(0), &lot)
        .expect_err("quantity finer than szDecimals must fail");
    assert!(error.to_string().contains("exceeds 5 decimal places"));
}

const TEST_CLOID: &str = "0x0000000000000000000000000000000a";

fn test_spec(asset_id: u32) -> HyperliquidInstrumentSpec {
    HyperliquidInstrumentSpec {
        asset_id,
        canonical_symbol: "BTC".into(),
        price_tick: 0.1,
        qty_step: 0.000_01,
        min_qty: 0.000_01,
        size_decimals: 5,
        price_decimals: 1,
        is_trading: true,
    }
}

fn test_intent(order_type: OrderType) -> OrderIntent {
    OrderIntent {
        id: "i1".into(),
        source: shared_types::OrderSource::Manual,
        strategy: None,
        mode: shared_types::ExecutionMode::Live,
        exchange: NAME.into(),
        symbol: "BTC".into(),
        side: OrderSide::Buy,
        order_type,
        quantity: 0.01,
        price: Some(50_000.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: shared_types::TimeInForce::Ioc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: TEST_CLOID.into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

fn test_cancel(exchange_order_id: Option<String>) -> CancelOrderRequest {
    CancelOrderRequest {
        exchange: NAME.into(),
        symbol: "BTC".into(),
        internal_order_id: "i1".into(),
        exchange_order_id,
        client_order_id: TEST_CLOID.into(),
    }
}

fn policy_cloid(client_order_id: &str) -> String {
    crate::client_order_id_policy::client_order_id_policy(NAME, client_order_id)
        .venue_client_order_id
        .expect("hyperliquid cloid")
}

fn is_policy_cloid(value: &str) -> bool {
    value
        .strip_prefix("0x")
        .is_some_and(|hex| hex.len() == 32 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}
