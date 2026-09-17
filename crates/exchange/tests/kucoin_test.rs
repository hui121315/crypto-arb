#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! KuCoin 适配器集成测试（wiremock）。

use exchange::{ExchangeAdapter, Kucoin, KucoinConfig, KucoinCredentials, LiveTradingAdapter};
use serde_json::json;
use shared_types::{
    CancelOrderRequest, ExecutionMode, IndexCompositionQuality, LiveOrderState, OrderIntent,
    OrderSide, OrderSource, OrderType,
};
use wiremock::matchers::{body_partial_json, header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ku(server_uri: String, credentials: Option<KucoinCredentials>) -> Kucoin {
    Kucoin::new(KucoinConfig {
        credentials,
        allow_live_writes: true,
        timeout_secs: 5,
        qps: 100,
        base_url_override: Some(server_uri),
        margin_mode: exchange::adapters::KucoinMarginMode::Isolated,
        default_leverage: 1,
    })
    .unwrap()
}

fn kucoin_credentials() -> KucoinCredentials {
    KucoinCredentials {
        api_key: "test-key".into(),
        api_secret: "test-secret".into(),
        passphrase: "p".into(),
    }
}

fn limit_intent() -> OrderIntent {
    OrderIntent {
        id: "internal-1".into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: "kucoin".into(),
        symbol: "BTC".into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 0.01,
        price: Some(30_000.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: shared_types::TimeInForce::Ioc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "client-1".into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

fn cancel_request() -> CancelOrderRequest {
    CancelOrderRequest {
        exchange: "kucoin".into(),
        symbol: "BTC".into(),
        internal_order_id: "internal-1".into(),
        exchange_order_id: None,
        client_order_id: "client-1".into(),
    }
}

#[tokio::test]
async fn batch_funding_rates_from_contracts_active() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/contracts/active"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "200000",
            "data": [
                {
                    "symbol": "XBTUSDTM",
                    "fundingFeeRate": 0.0001,
                    "predictedFundingFeeRate": 0.00012,
                    "fundingRateGranularity": 28800000,
                    "nextFundingRateDateTime": 1700028800000_i64,
                    "lastTradePrice": 30000.0,
                    "turnoverOf24h": 1500000000.0
                },
                {
                    "symbol": "ETHUSDTM",
                    "fundingFeeRate": 0.00005,
                    "fundingRateGranularity": 14400000,
                    "nextFundingRateDateTime": 1700014400000_i64,
                    "lastTradePrice": 2000.0,
                    "turnoverOf24h": 500000000.0
                },
                // 非 USDT-M 应被过滤
                {
                    "symbol": "BTCUSDM",
                    "fundingFeeRate": 0.0001,
                    "fundingRateGranularity": 28800000,
                    "nextFundingRateDateTime": 1700028800000_i64,
                    "lastTradePrice": 30000.0,
                    "turnoverOf24h": 100000.0
                }
            ]
        })))
        .mount(&server)
        .await;

    let k = ku(server.uri(), None);
    let rates = k.get_funding_rates(None).await.expect("ok");
    assert_eq!(rates.len(), 2, "USDM should be filtered out");

    let btc = rates.iter().find(|r| r.symbol == "BTC").unwrap();
    assert_eq!(btc.exchange, "kucoin");
    assert_eq!(btc.funding_interval, 8);
    assert!((btc.rate - 0.0001).abs() < 1e-12);
    assert!((btc.rate_8h - 0.0001).abs() < 1e-12);
    assert!(btc.predicted_rate.is_some());
    assert!((btc.predicted_rate.unwrap() - 0.00012).abs() < 1e-12);

    let eth = rates.iter().find(|r| r.symbol == "ETH").unwrap();
    assert_eq!(eth.funding_interval, 4);
    // 4h 0.005% → 8h 0.01%
    assert!((eth.rate_8h - 0.0001).abs() < 1e-12);
}

#[tokio::test]
async fn balance_signed_with_encrypted_passphrase() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/account-overview"))
        .and(query_param("currency", "USDT"))
        .and(header_exists("KC-API-KEY"))
        .and(header_exists("KC-API-SIGN"))
        .and(header_exists("KC-API-TIMESTAMP"))
        .and(header_exists("KC-API-PASSPHRASE"))
        .and(header_exists("KC-API-KEY-VERSION"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "200000",
            "data": {
                "accountEquity": 10000.0,
                "unrealisedPNL": 5.0,
                "marginBalance": 10000.0,
                "positionMargin": 800.0,
                "orderMargin": 100.0,
                "frozenFunds": 100.0,
                "availableBalance": 9000.0,
                "availableMargin": 9000.0,
                "riskRatio": 0.1,
                "currency": "USDT"
            }
        })))
        .mount(&server)
        .await;

    let k = ku(
        server.uri(),
        Some(KucoinCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
            passphrase: "p".into(),
        }),
    );
    let balances = k.get_balance(None).await.expect("ok");
    let usdt = balances.get("USDT").unwrap();
    assert!((usdt.total - 10000.0).abs() < 1e-9);
    assert!((usdt.available - 9000.0).abs() < 1e-9);
    assert!((usdt.frozen - 1000.0).abs() < 1e-9);
    assert!((usdt.unrealized_pnl - 5.0).abs() < 1e-9);
}

#[tokio::test]
async fn balance_missing_core_field_returns_parse_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/account-overview"))
        .and(query_param("currency", "USDT"))
        .and(header_exists("KC-API-SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "200000",
            "data": {
                "unrealisedPNL": 5.0,
                "positionMargin": 800.0,
                "orderMargin": 100.0,
                "frozenFunds": 100.0,
                "availableBalance": 9000.0,
                "currency": "USDT"
            }
        })))
        .mount(&server)
        .await;

    let k = ku(server.uri(), Some(kucoin_credentials()));
    let err = k
        .get_balance(None)
        .await
        .expect_err("missing accountEquity must fail closed");

    assert!(matches!(
        err,
        exchange::ExchangeError::Parse(message) if message.contains("missing accountEquity")
    ));
}

#[tokio::test]
async fn positions_filter_open_and_signed_qty() {
    let server = MockServer::start().await;
    mount_contract(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/positions"))
        .and(header_exists("KC-API-KEY"))
        .and(header_exists("KC-API-SIGN"))
        .and(header_exists("KC-API-TIMESTAMP"))
        .and(header_exists("KC-API-PASSPHRASE"))
        .and(header_exists("KC-API-KEY-VERSION"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "200000",
            "data": [
                {
                    "symbol": "XBTUSDTM",
                    "currentQty": 5.0,
                    "avgEntryPrice": 30000.0,
                    "markPrice": 30100.0,
                    "unrealisedPnl": 0.5,
                    "leverage": 10.0,
                    "liquidationPrice": 27000.0,
                    "posMargin": 10.0,
                    "maintMarginReq": 0.012,
                    "isOpen": true
                },
                {
                    "symbol": "ETHUSDTM",
                    "currentQty": -3.0,
                    "avgEntryPrice": 2000.0,
                    "markPrice": 1990.0,
                    "unrealisedPnl": 0.3,
                    "leverage": 10.0,
                    "liquidationPrice": 2200.0,
                    "posMargin": 5.0,
                    "maintMarginReq": 0.013,
                    "isOpen": true
                },
                // closed position should be filtered
                {
                    "symbol": "SOLUSDTM",
                    "currentQty": 0.0,
                    "avgEntryPrice": 0.0,
                    "markPrice": 0.0,
                    "unrealisedPnl": 0.0,
                    "leverage": 10.0,
                    "liquidationPrice": 0.0,
                    "posMargin": 0.0,
                    "isOpen": false
                }
            ]
        })))
        .mount(&server)
        .await;

    let k = ku(
        server.uri(),
        Some(KucoinCredentials {
            api_key: "k".into(),
            api_secret: "s".into(),
            passphrase: "p".into(),
        }),
    );
    let positions = ExchangeAdapter::get_positions(&k, None).await.expect("ok");
    assert_eq!(positions.len(), 2);
    let btc = positions.iter().find(|p| p.symbol == "BTC").unwrap();
    assert_eq!(btc.side, "long");
    assert!((btc.quantity - 0.005).abs() < 1e-12);

    let eth = positions.iter().find(|p| p.symbol == "ETH").unwrap();
    assert_eq!(eth.side, "short");
    assert!((eth.quantity - 0.03).abs() < 1e-12);
}

#[tokio::test]
async fn positions_missing_current_qty_returns_parse_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/positions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "200000",
            "data": [
                {
                    "symbol": "XBTUSDTM",
                    "avgEntryPrice": 30000.0,
                    "markPrice": 30100.0,
                    "unrealisedPnl": 0.5,
                    "leverage": 10.0,
                    "liquidationPrice": 27000.0,
                    "posMargin": 10.0,
                    "isOpen": true
                }
            ]
        })))
        .mount(&server)
        .await;

    let k = ku(server.uri(), Some(kucoin_credentials()));
    let err = ExchangeAdapter::get_positions(&k, None)
        .await
        .expect_err("missing currentQty must fail closed");

    assert!(matches!(
        err,
        exchange::ExchangeError::Parse(message) if message.contains("missing currentQty")
    ));
}

#[tokio::test]
async fn index_composition_is_unverified_without_fake_components() {
    let server = MockServer::start().await;
    let k = ku(server.uri(), None);
    let snapshot = k
        .get_index_composition("BTC")
        .await
        .expect("unverified snapshot");

    assert_eq!(snapshot.venue, "kucoin");
    assert_eq!(snapshot.symbol, "BTC");
    assert_eq!(snapshot.index_id, "BTC");
    assert_eq!(snapshot.quality, IndexCompositionQuality::Unverified);
    assert!(snapshot.components.is_empty());
    assert!(snapshot.error.unwrap().contains("no verified"));
}

#[tokio::test]
async fn api_error_propagates_non_200000_code() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/contracts/active"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "400001",
            "msg": "invalid request",
            "data": null
        })))
        .mount(&server)
        .await;

    let k = ku(server.uri(), None);
    let err = k.get_funding_rates(None).await.expect_err("api error");
    match err {
        exchange::ExchangeError::Api {
            exchange,
            code,
            message,
        } => {
            assert_eq!(exchange, "kucoin");
            assert_eq!(code, "400001");
            assert!(message.contains("invalid request"));
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn missing_credentials_returns_auth_error() {
    let server = MockServer::start().await;
    let k = ku(server.uri(), None);
    let err = k.get_balance(None).await.expect_err("auth");
    assert!(matches!(err, exchange::ExchangeError::Auth(_)));
}

#[tokio::test]
async fn live_place_order_one_way_sends_position_side_both() {
    let server = MockServer::start().await;
    mount_position_mode(&server, 0).await;
    mount_contract(&server).await;
    mount_position(&server, None).await;
    mount_place_order(&server, "BOTH", "buy").await;

    let k = ku(server.uri(), Some(kucoin_credentials()));
    let ack = LiveTradingAdapter::place_order(&k, &limit_intent())
        .await
        .expect("place order");

    assert_eq!(ack.exchange_order_id.as_deref(), Some("kucoin-1"));
    assert_eq!(ack.client_order_id, "client-1");
    assert_eq!(ack.state, LiveOrderState::Accepted);
}

#[tokio::test]
async fn live_place_order_hedge_buy_sends_position_side_long() {
    let server = MockServer::start().await;
    mount_position_mode(&server, 1).await;
    mount_contract(&server).await;
    mount_position(&server, Some((1.0, Some("CROSS")))).await;
    mount_place_order(&server, "LONG", "buy").await;

    let k = ku(server.uri(), Some(kucoin_credentials()));
    let ack = LiveTradingAdapter::place_order(&k, &limit_intent())
        .await
        .expect("place order");

    assert_eq!(ack.exchange_order_id.as_deref(), Some("kucoin-1"));
}

#[tokio::test]
async fn live_place_order_rechecks_position_change_and_submits_zero_orders() {
    let server = MockServer::start().await;
    mount_position_mode(&server, 0).await;
    mount_contract(&server).await;
    mount_position(&server, None).await;
    let k = ku(server.uri(), Some(kucoin_credentials()));
    let intent = limit_intent();

    LiveTradingAdapter::preflight_order(&k, "kucoin", &intent)
        .await
        .expect("preview preflight sees no open position");
    server.reset().await;
    mount_position(&server, Some((2.0, Some("CROSS")))).await;

    let err = LiveTradingAdapter::place_order(&k, &intent)
        .await
        .expect_err("changed leverage must fail before submit");
    assert!(matches!(
        err,
        exchange::ExchangeError::Api { code, message, .. }
            if code == "position_compatibility" && message.contains("leverage=2")
    ));
    let requests = server.received_requests().await.expect("request history");
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method.as_str() == "POST")
            .count(),
        0
    );
    assert!(requests.iter().any(|request| {
        request.url.path() == "/api/v2/position" && request.url.query() == Some("symbol=XBTUSDTM")
    }));
}

#[tokio::test]
async fn live_place_order_blocks_missing_margin_mode_evidence() {
    let server = MockServer::start().await;
    mount_position_mode(&server, 0).await;
    mount_contract(&server).await;
    mount_position(&server, Some((1.0, None))).await;

    let k = ku(server.uri(), Some(kucoin_credentials()));
    let err = LiveTradingAdapter::place_order(&k, &limit_intent())
        .await
        .expect_err("open position without margin mode must fail closed");

    assert!(matches!(
        err,
        exchange::ExchangeError::Api { code, message, .. }
            if code == "position_compatibility" && message.contains("missing marginMode evidence")
    ));
    let requests = server.received_requests().await.expect("request history");
    assert!(!requests
        .iter()
        .any(|request| request.method.as_str() == "POST"));
}

#[tokio::test]
async fn live_place_order_blocks_unknown_position_mode() {
    let server = MockServer::start().await;
    mount_position_mode(&server, 9).await;

    let k = ku(server.uri(), Some(kucoin_credentials()));
    let err = LiveTradingAdapter::place_order(&k, &limit_intent())
        .await
        .expect_err("unknown mode blocks");

    assert!(matches!(err, exchange::ExchangeError::Api { code, .. } if code == "validation"));
}

#[tokio::test]
async fn live_place_order_rejects_requested_quote_mismatch() {
    let server = MockServer::start().await;
    mount_position_mode(&server, 0).await;
    mount_contract(&server).await;

    let k = ku(server.uri(), Some(kucoin_credentials()));
    let mut intent = limit_intent();
    intent.symbol = "BTC-USDC".into();
    let err = LiveTradingAdapter::place_order(&k, &intent)
        .await
        .expect_err("quote mismatch blocks");

    assert!(matches!(err, exchange::ExchangeError::Api { code, .. } if code == "validation"));
}

#[tokio::test]
async fn live_account_mode_reads_one_way_position_mode() {
    let server = MockServer::start().await;
    mount_position_mode(&server, 0).await;

    let k = ku(server.uri(), Some(kucoin_credentials()));
    let mode = LiveTradingAdapter::get_exchange_account_mode(&k, "kucoin")
        .await
        .expect("account mode")
        .expect("mode");

    assert_eq!(mode.venue, "kucoin");
    assert_eq!(mode.mode, "one_way");
    assert_eq!(mode.source, "kucoin.GET /api/v2/position/getPositionMode");
    assert!(mode.freshness_ms.is_some());
}

#[tokio::test]
async fn live_account_mode_reads_hedge_position_mode() {
    let server = MockServer::start().await;
    mount_position_mode(&server, 1).await;

    let k = ku(server.uri(), Some(kucoin_credentials()));
    let mode = LiveTradingAdapter::get_exchange_account_mode(&k, "kucoin")
        .await
        .expect("account mode")
        .expect("mode");

    assert_eq!(mode.mode, "hedge");
}

#[tokio::test]
async fn account_mode_read_does_not_require_live_writes() {
    let server = MockServer::start().await;
    mount_position_mode(&server, 1).await;

    let k = Kucoin::new(KucoinConfig {
        credentials: Some(kucoin_credentials()),
        allow_live_writes: false,
        timeout_secs: 5,
        qps: 100,
        base_url_override: Some(server.uri()),
        margin_mode: exchange::adapters::KucoinMarginMode::Isolated,
        default_leverage: 1,
    })
    .expect("kucoin adapter");

    let mode = LiveTradingAdapter::get_exchange_account_mode(&k, "kucoin")
        .await
        .expect("account mode")
        .expect("mode");

    assert_eq!(mode.mode, "hedge");
    assert_eq!(mode.source, "kucoin.GET /api/v2/position/getPositionMode");
    assert_eq!(mode.account_scope.as_deref(), Some("classic_futures"));
}

#[tokio::test]
async fn live_get_order_uses_official_by_client_oid_query() {
    let server = MockServer::start().await;
    mount_contract(&server).await;

    Mock::given(method("GET"))
        .and(path("/api/v1/orders/byClientOid"))
        .and(query_param("clientOid", "client-1"))
        .and(header_exists("KC-API-SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "200000",
            "data": {
                "id": "284486580251463680",
                "symbol": "XBTUSDTM",
                "type": "limit",
                "side": "buy",
                "price": "30000",
                "size": 10,
                "filledSize": 2,
                "filledValue": "60",
                "createdAt": 1700000000000_i64,
                "postOnly": false,
                "cancelExist": false,
                "status": "open",
                "clientOid": "client-1",
                "reduceOnly": false
            }
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/v1/fills"))
        .and(query_param("orderId", "284486580251463680"))
        .and(header_exists("KC-API-SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "200000",
            "data": {
                "items": [{
                    "symbol": "XBTUSDTM",
                    "tradeId": "1828954878212",
                    "orderId": "284486580251463680",
                    "side": "buy",
                    "liquidity": "taker",
                    "price": "30000",
                    "size": 2,
                    "value": "60",
                    "fee": "0.036",
                    "feeRate": "0.0006",
                    "feeCurrency": "USDT",
                    "settleCurrency": "USDT",
                    "createdAt": 1740640088427_i64
                }]
            }
        })))
        .mount(&server)
        .await;

    let k = ku(server.uri(), Some(kucoin_credentials()));
    let order = LiveTradingAdapter::get_order(&k, "BTC", "client-1")
        .await
        .expect("get order")
        .expect("order");

    assert_eq!(order.order_id, "284486580251463680");
    assert_eq!(order.symbol, "BTC");
    assert_eq!(order.filled_quantity, 0.002);
    assert!((order.filled_price - 30_000.0).abs() < 1e-9);
    assert!((order.fees - 0.036).abs() < 1e-12);
}

#[tokio::test]
async fn live_get_order_rejects_malformed_order_payload() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/orders/byClientOid"))
        .and(query_param("clientOid", "client-1"))
        .and(header_exists("KC-API-SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "200000",
            "data": {
                "id": "kucoin-1",
                "symbol": "XBTUSDTM",
                "type": "limit",
                "side": "buy",
                "price": "bad-price",
                "size": 10,
                "filledSize": 0,
                "filledValue": "0",
                "createdAt": 1700000000000_i64,
                "postOnly": false,
                "cancelExist": false,
                "status": "open",
                "clientOid": "client-1",
                "reduceOnly": false
            }
        })))
        .mount(&server)
        .await;

    let k = ku(server.uri(), Some(kucoin_credentials()));
    let err = LiveTradingAdapter::get_order(&k, "BTC", "client-1")
        .await
        .expect_err("bad price must fail closed");

    assert!(matches!(
        err,
        exchange::ExchangeError::Parse(message) if message.contains("invalid price")
    ));
}

#[tokio::test]
async fn live_open_orders_rejects_malformed_order_row() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/orders"))
        .and(query_param("status", "active"))
        .and(header_exists("KC-API-SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "200000",
            "data": {
                "items": [
                    {
                        "id": "kucoin-1",
                        "symbol": "XBTUSDTM",
                        "type": "limit",
                        "side": "buy",
                        "price": "30000",
                        "size": 10,
                        "filledSize": 2,
                        "filledValue": "bad-filled-value",
                        "createdAt": 1700000000000_i64,
                        "postOnly": false,
                        "cancelExist": false,
                        "status": "open",
                        "clientOid": "client-1",
                        "reduceOnly": false
                    }
                ]
            }
        })))
        .mount(&server)
        .await;

    let k = ku(server.uri(), Some(kucoin_credentials()));
    let err = LiveTradingAdapter::get_open_orders(&k, None)
        .await
        .expect_err("malformed filledValue must fail closed before fill reconciliation");

    assert!(matches!(
        err,
        exchange::ExchangeError::Parse(message) if message.contains("invalid filledValue")
    ));
}

#[tokio::test]
async fn live_cancel_order_by_client_oid() {
    let server = MockServer::start().await;
    mount_contract(&server).await;

    Mock::given(method("DELETE"))
        .and(path("/api/v1/orders/client-order/client-1"))
        .and(query_param("symbol", "XBTUSDTM"))
        .and(header_exists("KC-API-SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "200000",
            "data": {"cancelledOrderIds": ["284486580251463680"]}
        })))
        .mount(&server)
        .await;

    let k = ku(server.uri(), Some(kucoin_credentials()));
    let ack = LiveTradingAdapter::cancel_order(&k, &cancel_request())
        .await
        .expect("cancel order");
    assert_eq!(ack.exchange_order_id.as_deref(), Some("284486580251463680"));
    assert_eq!(ack.state, LiveOrderState::CancelRequested);
}

async fn mount_position_mode(server: &MockServer, position_mode: i64) {
    Mock::given(method("GET"))
        .and(path("/api/v2/position/getPositionMode"))
        .and(header_exists("KC-API-SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "200000",
            "data": {"positionMode": position_mode}
        })))
        .mount(server)
        .await;
}

async fn mount_contract(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/v1/contracts/active"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            include_str!("../fixtures/kucoin/contracts_active_xbt_eth_usdtm.json"),
            "application/json",
        ))
        .mount(server)
        .await;
}

async fn mount_position(server: &MockServer, position: Option<(f64, Option<&str>)>) {
    let data = position
        .map(|(leverage, margin_mode)| {
            vec![json!({
                "symbol": "XBTUSDTM",
                "currentQty": 1.0,
                "avgEntryPrice": 30000.0,
                "markPrice": 30100.0,
                "unrealisedPnl": 1.0,
                "leverage": leverage,
                "marginMode": margin_mode,
                "liquidationPrice": 25000.0,
                "posMargin": 30.0,
                "maintMarginReq": 0.004,
                "isOpen": true
            })]
        })
        .unwrap_or_default();
    Mock::given(method("GET"))
        .and(path("/api/v2/position"))
        .and(query_param("symbol", "XBTUSDTM"))
        .and(header_exists("KC-API-SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "200000",
            "data": data
        })))
        .mount(server)
        .await;
}

async fn mount_place_order(server: &MockServer, position_side: &str, side: &str) {
    Mock::given(method("POST"))
        .and(path("/api/v1/orders"))
        .and(header_exists("KC-API-SIGN"))
        .and(body_partial_json(json!({
            "clientOid": "client-1",
            "symbol": "XBTUSDTM",
            "marginMode": "CROSS",
            "leverage": 1,
            "positionSide": position_side,
            "side": side,
            "type": "limit",
            "size": 10,
            "price": "30000",
            "timeInForce": "IOC"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "200000",
            "data": {"orderId": "kucoin-1", "clientOid": "client-1"}
        })))
        .mount(server)
        .await;
}
