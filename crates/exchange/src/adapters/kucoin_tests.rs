use super::*;
use crate::adapters::kucoin_market_data::ContractActive;
use crate::adapters::kucoin_response::KucoinResponse;
use pretty_assertions::assert_eq;
use std::sync::atomic::Ordering;

fn ku() -> Kucoin {
    Kucoin::new(KucoinConfig::default()).unwrap()
}

#[test]
fn name() {
    assert_eq!(ExchangeAdapter::name(&ku()), "kucoin");
}

#[test]
fn order_margin_modes_expose_intent_driven_cross_and_isolated() {
    // 下单载荷 marginMode 直接来自 OrderIntent，两种模式均可逐单选择。
    assert_eq!(
        ku().order_margin_modes(),
        vec![MarginMode::Cross, MarginMode::Isolated]
    );
}

#[test]
fn symbol_xbt_btc_round_trip() {
    let k = ku();
    // Bare canonical symbols stay unresolved until official metadata provides a unique contract.
    assert_eq!(k.to_exchange_symbol("BTC"), "BTC");
    assert_eq!(k.to_exchange_symbol("BTC-USDT"), "BTC-USDT");
    assert_eq!(k.to_exchange_symbol("BTC-USDC"), "BTC-USDC");
    assert_eq!(k.normalize_symbol("XBTUSDTM"), "BTC");
    assert_eq!(k.normalize_symbol("XBTUSDCM"), "BTC");
    // Unquoted ordinary symbols are not assigned a settlement currency either.
    assert_eq!(k.to_exchange_symbol("ETH"), "ETH");
    assert_eq!(k.to_exchange_symbol("ETH-USDT"), "ETH-USDT");
    assert_eq!(k.normalize_symbol("ETHUSDTM"), "ETH");
    // idempotent
    assert_eq!(k.to_exchange_symbol("XBTUSDTM"), "XBTUSDTM");
}

#[test]
fn kucoin_response_propagates_non_success() {
    let body = r#"{"code":"400001","msg":"bad","data":null}"#;
    let wrap: KucoinResponse<Vec<ContractActive>> = serde_json::from_str(body).unwrap();
    let err = wrap.into_data("test").unwrap_err();
    match err {
        ExchangeError::Api { code, .. } => assert_eq!(code, "400001"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn require_credentials_errors_when_missing() {
    let k = ku();
    assert!(k.require_credentials().is_err());
}

#[tokio::test]
async fn orderbook_normalizes_contract_counts_to_base_quantity() {
    use wiremock::matchers::{method, path, query_param};

    let server = wiremock::MockServer::start().await;
    let contracts = include_str!("../../fixtures/kucoin/contracts_active_xbt_eth_usdtm.json");
    wiremock::Mock::given(method("GET"))
        .and(path("/api/v1/contracts/active"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(contracts))
        .expect(1)
        .mount(&server)
        .await;
    let depth = include_str!("../../fixtures/kucoin/futures_depth20_xbtusdtm.json");
    wiremock::Mock::given(method("GET"))
        .and(path("/api/v1/level2/depth20"))
        .and(query_param("symbol", "XBTUSDTM"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(depth))
        .expect(1)
        .mount(&server)
        .await;
    let adapter = Kucoin::new(KucoinConfig {
        base_url_override: Some(server.uri()),
        ..KucoinConfig::default()
    })
    .expect("KuCoin adapter");

    let book = ExchangeAdapter::get_orderbook(&adapter, "BTC", 20)
        .await
        .expect("normalized KuCoin orderbook");

    assert!((book.bids[0][1] - 1.856).abs() < 1e-12);
    assert!((book.asks[0][1] - 0.052).abs() < 1e-12);
}

#[tokio::test]
async fn private_order_and_position_reads_normalize_contract_counts_to_base_quantity() {
    use wiremock::matchers::{method, path, query_param};

    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(method("GET"))
        .and(path("/api/v1/contracts/active"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": "200000",
                "data": [
                    {
                        "symbol": "SOLUSDTM",
                        "baseCurrency": "SOL",
                        "quoteCurrency": "USDT",
                        "settleCurrency": "USDT",
                        "multiplier": "0.1",
                        "tickSize": "0.001",
                        "lotSize": 1,
                        "maxOrderQty": 1_000_000,
                        "marketMaxOrderQty": 1_000_000,
                        "isInverse": false,
                        "status": "Open"
                    },
                    {
                        "symbol": "SOLUSDCM",
                        "baseCurrency": "SOL",
                        "quoteCurrency": "USDC",
                        "settleCurrency": "USDC",
                        "multiplier": "0.1",
                        "tickSize": "0.001",
                        "lotSize": 1,
                        "maxOrderQty": 1_000_000,
                        "marketMaxOrderQty": 1_000_000,
                        "isInverse": false,
                        "status": "Open"
                    }
                ]
            })),
        )
        .expect(1)
        .mount(&server)
        .await;
    wiremock::Mock::given(method("GET"))
        .and(path("/api/v1/orders/234125150956625920"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": "200000",
                "data": {
                    "id": "234125150956625920",
                    "symbol": "SOLUSDTM",
                    "side": "buy",
                    "type": "limit",
                    "status": "done",
                    "price": "72",
                    "size": 1,
                    "filledSize": 1,
                    "filledValue": "7.2",
                    "cancelExist": false,
                    "createdAt": 1_700_000_000_000_i64,
                    "postOnly": false,
                    "timeInForce": "IOC",
                    "clientOid": "xl-kucoin-unit-test",
                    "reduceOnly": false
                }
            })),
        )
        .expect(1)
        .mount(&server)
        .await;
    wiremock::Mock::given(method("GET"))
        .and(path("/api/v1/fills"))
        .and(query_param("orderId", "234125150956625920"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": "200000",
                "data": {
                    "items": [{
                        "symbol": "SOLUSDTM",
                        "tradeId": "trade-1",
                        "orderId": "234125150956625920",
                        "side": "buy",
                        "liquidity": "taker",
                        "price": "72",
                        "size": "1",
                        "value": "7.2",
                        "fee": "0.00432",
                        "feeRate": "0.0006",
                        "feeCurrency": "USDT",
                        "settleCurrency": "USDT",
                        "createdAt": 1_700_000_000_001_i64
                    }]
                }
            })),
        )
        .expect(1)
        .mount(&server)
        .await;
    wiremock::Mock::given(method("GET"))
        .and(path("/api/v1/positions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": "200000",
                "data": [{
                    "symbol": "SOLUSDTM",
                    "currentQty": "1",
                    "avgEntryPrice": "72",
                    "markPrice": "72.1",
                    "unrealisedPnl": "0.01",
                    "leverage": "1",
                    "marginMode": "CROSS",
                    "liquidationPrice": "10",
                    "posMargin": "7.2",
                    "maintMarginReq": "0.004",
                    "isOpen": true
                }]
            })),
        )
        .expect(1)
        .mount(&server)
        .await;
    let adapter = Kucoin::new(KucoinConfig {
        credentials: Some(KucoinCredentials {
            api_key: "key".into(),
            api_secret: "secret".into(),
            passphrase: "pass".into(),
        }),
        allow_live_writes: true,
        base_url_override: Some(server.uri()),
        ..KucoinConfig::default()
    })
    .expect("adapter");
    adapter
        .time_synced_at_ms
        .store(common::time::now_ms(), Ordering::Relaxed);

    let order = LiveTradingAdapter::get_order_by_exchange_order_id(
        &adapter,
        "SOL-USDT",
        "234125150956625920",
    )
    .await
    .expect("order query")
    .expect("order row");
    let positions = ExchangeAdapter::get_positions(&adapter, None)
        .await
        .expect("position query");

    assert_eq!(order.quantity, 0.1);
    assert_eq!(order.filled_quantity, 0.1);
    assert_eq!(order.filled_price, 72.0);
    assert_eq!(order.fees, 0.00432);
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0].symbol, "SOL");
    assert_eq!(positions[0].quantity, 0.1);
}

#[tokio::test]
async fn get_order_maps_exact_order_not_exist_response_to_none() {
    use wiremock::matchers::{method, path, query_param};

    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(method("GET"))
        .and(path("/api/v1/orders/byClientOid"))
        .and(query_param("clientOid", "missing-client"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": "100001",
                "msg": "error.getOrder.orderNotExist"
            })),
        )
        .expect(1)
        .mount(&server)
        .await;
    let adapter = Kucoin::new(KucoinConfig {
        credentials: Some(KucoinCredentials {
            api_key: "key".into(),
            api_secret: "secret".into(),
            passphrase: "pass".into(),
        }),
        allow_live_writes: true,
        base_url_override: Some(server.uri()),
        ..KucoinConfig::default()
    })
    .expect("adapter");
    adapter
        .time_synced_at_ms
        .store(common::time::now_ms(), Ordering::Relaxed);

    let order = LiveTradingAdapter::get_order(&adapter, "SOL", "missing-client")
        .await
        .expect("missing order is a successful identity query");

    assert!(order.is_none());
}

#[test]
fn contract_count_normalization_rejects_missing_multiplier_evidence() {
    assert!(kucoin_contracts_to_base(1.0, 0.0, "order.quantity", false).is_err());
    assert!(kucoin_contracts_to_base(-1.0, 0.1, "order.quantity", false).is_err());
}

#[test]
fn reduce_only_position_compatibility_does_not_require_caller_leverage_match() {
    let mut intent = Kucoin::safe_order_test_intent();
    intent.symbol = "SOLUSDT".to_owned();
    intent.reduce_only = true;
    intent.leverage = 1.0;
    intent.margin_mode = MarginMode::Cross;

    ensure_position_matches_intent(&position_with_risk_mode(3.0, "CROSS"), &intent)
        .expect("reduce-only close must not be blocked by stale caller leverage");
}

#[test]
fn entry_position_compatibility_still_requires_leverage_match() {
    let mut intent = Kucoin::safe_order_test_intent();
    intent.symbol = "SOLUSDT".to_owned();
    intent.leverage = 1.0;
    intent.margin_mode = MarginMode::Cross;

    let error = ensure_position_matches_intent(&position_with_risk_mode(3.0, "CROSS"), &intent)
        .expect_err("entry order must preserve the existing leverage invariant");

    assert!(matches!(
        error,
        ExchangeError::Api { code, message, .. }
            if code == "position_compatibility" && message.contains("leverage=3")
    ));
}

#[test]
fn reduce_only_position_compatibility_still_requires_margin_mode_match() {
    let mut intent = Kucoin::safe_order_test_intent();
    intent.symbol = "SOLUSDT".to_owned();
    intent.reduce_only = true;
    intent.margin_mode = MarginMode::Cross;

    let error = ensure_position_matches_intent(&position_with_risk_mode(3.0, "ISOLATED"), &intent)
        .expect_err("reduce-only order must retain the position margin mode");

    assert!(matches!(
        error,
        ExchangeError::Api { code, message, .. }
            if code == "position_compatibility" && message.contains("marginMode=ISOLATED")
    ));
}

fn position_with_risk_mode(leverage: f64, margin_mode: &str) -> PositionInfo {
    PositionInfo {
        symbol: "SOL".to_owned(),
        exchange: NAME.to_owned(),
        side: "long".to_owned(),
        quantity: 0.1,
        entry_price: 72.0,
        mark_price: 72.0,
        unrealized_pnl: 0.0,
        leverage,
        liquidation_price: None,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin: 2.4,
        maintenance_margin_ratio: 0.0,
        position_mode: Some("one_way".to_owned()),
        margin_mode: Some(margin_mode.to_owned()),
        risk_rate: None,
        available_position: Some(0.1),
        frozen_position: Some(0.0),
    }
}

#[tokio::test]
async fn funding_payments_fetches_two_signed_offset_pages_and_deduplicates() {
    use wiremock::matchers::{method, path, query_param, query_param_is_missing};

    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(method("GET"))
        .and(path("/api/v1/contracts/active"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": "200000",
                "data": [{
                    "symbol": "XBTUSDTM",
                    "baseCurrency": "XBT",
                    "quoteCurrency": "USDT",
                    "settleCurrency": "USDT",
                    "multiplier": "0.001",
                    "tickSize": "0.1",
                    "lotSize": 1,
                    "maxOrderQty": 1000000,
                    "marketMaxOrderQty": 1000000,
                    "isInverse": false,
                    "status": "Open"
                }]
            })),
        )
        .mount(&server)
        .await;
    wiremock::Mock::given(method("GET"))
        .and(path("/api/v1/funding-history"))
        .and(query_param("symbol", "XBTUSDTM"))
        .and(query_param("startAt", "1700000000000"))
        .and(query_param("endAt", "1700086400000"))
        .and(query_param_is_missing("offset"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(kucoin_funding_page(true, &[(100, "0.1")])),
        )
        .mount(&server)
        .await;
    wiremock::Mock::given(method("GET"))
        .and(path("/api/v1/funding-history"))
        .and(query_param("symbol", "XBTUSDTM"))
        .and(query_param("startAt", "1700000000000"))
        .and(query_param("endAt", "1700086400000"))
        .and(query_param("offset", "100"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(kucoin_funding_page(false, &[(100, "0.1"), (99, "-0.2")])),
        )
        .mount(&server)
        .await;
    let adapter = Kucoin::new(KucoinConfig {
        credentials: Some(KucoinCredentials {
            api_key: "key".into(),
            api_secret: "secret".into(),
            passphrase: "pass".into(),
        }),
        base_url_override: Some(server.uri()),
        ..KucoinConfig::default()
    })
    .expect("adapter");
    adapter
        .time_synced_at_ms
        .store(common::time::now_ms(), Ordering::Relaxed);

    let payments = ExchangeAdapter::get_funding_payments(
        &adapter,
        Some("BTC"),
        Some(1_700_000_000_000),
        Some(1_700_086_400_000),
    )
    .await
    .expect("two offset pages");

    assert_eq!(payments.len(), 2);
    assert_eq!(payments[0].venue_event_id, "kucoin_funding:100");
    assert_eq!(payments[1].venue_event_id, "kucoin_funding:99");
    let requests = server.received_requests().await.expect("request history");
    let signatures = requests
        .iter()
        .filter_map(|request| request.headers.get("kc-api-sign"))
        .collect::<Vec<_>>();
    assert_eq!(signatures.len(), 2);
    assert_ne!(signatures[0], signatures[1]);
}

fn kucoin_funding_page(has_more: bool, rows: &[(u64, &str)]) -> serde_json::Value {
    serde_json::json!({
        "code": "200000",
        "data": {
            "hasMore": has_more,
            "dataList": rows.iter().enumerate().map(|(index, (id, amount))| serde_json::json!({
                "id": id,
                "symbol": "XBTUSDTM",
                "funding": amount,
                "settleCurrency": "USDT",
                "timePoint": 1_700_000_000_000_i64 + index as i64
            })).collect::<Vec<_>>()
        }
    })
}
