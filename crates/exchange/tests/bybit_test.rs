#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! Bybit 适配器集成测试（wiremock）。

use exchange::{Bybit, BybitConfig, BybitCredentials, ExchangeAdapter, LiveTradingAdapter};
use serde_json::{json, Value};
use shared_types::{
    CancelOrderRequest, ExecutionMode, IndexCompositionQuality, LiveOrderState, OrderIntent,
    OrderSide, OrderSource, OrderType,
};
use wiremock::matchers::{body_partial_json, header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn bybit(server_uri: String, credentials: Option<BybitCredentials>) -> Bybit {
    Bybit::new(BybitConfig {
        credentials,
        testnet: false,
        allow_live_writes: false,
        timeout_secs: 5,
        qps: 100,
        recv_window: "5000".into(),
        base_url_override: Some(server_uri),
        ..Default::default()
    })
    .unwrap()
}

#[tokio::test]
async fn batch_funding_rates_with_interval_join() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v5/market/tickers"))
        .and(query_param("category", "linear"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": {
                "list": [
                    {
                        "symbol": "BTCUSDT",
                        "lastPrice": "30000",
                        "bid1Price": "29999",
                        "ask1Price": "30001",
                        "turnover24h": "1500000000",
                        "fundingRate": "0.0001",
                        "nextFundingTime": "1700028800000"
                    },
                    {
                        "symbol": "ETHUSDT",
                        "lastPrice": "2000",
                        "bid1Price": "1999",
                        "ask1Price": "2001",
                        "turnover24h": "500000000",
                        "fundingRate": "0.00005",
                        "nextFundingTime": "1700014400000"
                    },
                    // 非 USDT 应被过滤（且 instruments-info 不会有 fundingInterval）
                    {
                        "symbol": "BTCUSDC",
                        "lastPrice": "30000",
                        "bid1Price": "29999",
                        "ask1Price": "30001",
                        "turnover24h": "100000",
                        "fundingRate": "0.0002",
                        "nextFundingTime": "1700028800000"
                    }
                ]
            }
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/v5/market/instruments-info"))
        .and(query_param("category", "linear"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "retCode": 0,
            "result": {
                "list": [
                    {"symbol": "BTCUSDT", "fundingInterval": 480, "settleCoin": "USDT"},
                    {"symbol": "ETHUSDT", "fundingInterval": 240, "settleCoin": "USDT"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let b = bybit(server.uri(), None);
    let rates = b.get_funding_rates(None).await.expect("ok");

    assert_eq!(
        rates.len(),
        2,
        "BTCUSDC 应被过滤（无 USDT 结算 instrument-info）"
    );

    let btc = rates.iter().find(|r| r.symbol == "BTC").unwrap();
    assert_eq!(btc.exchange, "bybit");
    assert!((btc.rate - 0.0001).abs() < 1e-12);
    assert_eq!(btc.funding_interval, 8); // 480 minutes / 60 = 8h
    assert!((btc.rate_8h - 0.0001).abs() < 1e-12);
    assert!((btc.volume_24h - 1_500_000_000.0).abs() < 1.0);

    let eth = rates.iter().find(|r| r.symbol == "ETH").unwrap();
    assert_eq!(eth.funding_interval, 4); // 240 minutes / 60 = 4h
                                         // 4h 0.005% → 8h 标准化 0.01%
    assert!((eth.rate_8h - 0.0001).abs() < 1e-12);
}

#[tokio::test]
async fn single_funding_rate() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v5/market/tickers"))
        .and(query_param("category", "linear"))
        .and(query_param("symbol", "BTCUSDT"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "retCode": 0,
            "result": {
                "list": [{
                    "symbol": "BTCUSDT",
                    "lastPrice": "30000",
                    "bid1Price": "29999",
                    "ask1Price": "30001",
                    "turnover24h": "0",
                    "fundingRate": "0.0003",
                    "nextFundingTime": "1700028800000"
                }]
            }
        })))
        .mount(&server)
        .await;

    let b = bybit(server.uri(), None);
    let r = b.get_funding_rate("BTC").await.expect("ok");
    assert_eq!(r.symbol, "BTC");
    assert!((r.rate - 0.0003).abs() < 1e-12);
}

#[tokio::test]
async fn orderbook_parses_b_a_levels() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v5/market/orderbook"))
        .and(query_param("category", "linear"))
        .and(query_param("symbol", "BTCUSDT"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "retCode": 0,
            "result": {
                "s": "BTCUSDT",
                "b": [["30000.0", "1.5"], ["29999.5", "2.0"]],
                "a": [["30001.0", "1.0"], ["30001.5", "0.8"]],
                "ts": 1700000000000_i64,
                "u": 1
            }
        })))
        .mount(&server)
        .await;

    let b = bybit(server.uri(), None);
    let ob = b.get_orderbook("BTC", 50).await.expect("ok");
    assert_eq!(ob.symbol, "BTC");
    assert_eq!(ob.bids.len(), 2);
    assert!((ob.bids[0][0] - 30000.0).abs() < 1e-9);
    assert!((ob.bids[0][1] - 1.5).abs() < 1e-9);
}

#[tokio::test]
async fn index_composition_components_follow_official_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v5/market/index-price-components"))
        .and(query_param("indexName", "BTCUSDT"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": {
                "indexName": "BTCUSDT",
                "lastPrice": "94000",
                "updateTime": "1758182745072",
                "components": [
                    {
                        "exchange": "Binance",
                        "spotPair": "BTCUSDT",
                        "equivalentPrice": "94001.0",
                        "multiplier": "1",
                        "price": "94001.0",
                        "weight": "0.50"
                    },
                    {
                        "exchange": "GateIO",
                        "spotPair": "BTC_USDT",
                        "equivalentPrice": "93999.0",
                        "multiplier": "1",
                        "price": "93999.0",
                        "weight": "0.25"
                    }
                ]
            },
            "time": 1758182745621_i64
        })))
        .mount(&server)
        .await;

    let b = bybit(server.uri(), None);
    let snapshot = b
        .get_index_composition("BTC")
        .await
        .expect("index composition");

    assert_eq!(snapshot.venue, "bybit");
    assert_eq!(snapshot.symbol, "BTC");
    assert_eq!(snapshot.index_id, "BTCUSDT");
    assert_eq!(snapshot.quality, IndexCompositionQuality::Verified);
    assert_eq!(snapshot.source, "GET /v5/market/index-price-components");
    assert_eq!(snapshot.components.len(), 2);
    assert_eq!(snapshot.components[0].name, "Binance");
    assert_eq!(snapshot.components[0].symbol, "BTCUSDT");
    assert!((snapshot.components[0].weight - 0.5).abs() < 1e-12);
    assert_eq!(snapshot.components[0].price, Some(94001.0));
}

#[tokio::test]
async fn balance_signed_headers() {
    let server = MockServer::start().await;
    let body: Value = serde_json::from_str(include_str!(
        "../fixtures/bybit/wallet_balance_unified_account_metrics.json"
    ))
    .expect("Bybit UNIFIED account fixture");
    Mock::given(method("GET"))
        .and(path("/v5/account/wallet-balance"))
        .and(query_param("accountType", "UNIFIED"))
        .and(header_exists("X-BAPI-API-KEY"))
        .and(header_exists("X-BAPI-SIGN"))
        .and(header_exists("X-BAPI-TIMESTAMP"))
        .and(header_exists("X-BAPI-RECV-WINDOW"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let b = bybit(
        server.uri(),
        Some(BybitCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
        }),
    );
    let read = LiveTradingAdapter::get_account_read(&b, None)
        .await
        .expect("signed UNIFIED account read");
    assert_eq!(read.balances.len(), 1);
    assert_eq!(read.balances[0].currency, "USDT");
    assert_eq!(read.balances[0].available, 9_556.605_655_5);

    let [summary] = read.summaries.as_slice() else {
        panic!("one Bybit UNIFIED account summary expected");
    };
    assert_eq!(summary.account_type, "UNIFIED");
    assert_eq!(summary.total_equity_usd, 10_262.913_350_23);
    assert_eq!(summary.total_initial_margin_usd, 127.857_316_14);
    assert_eq!(summary.total_maintenance_margin_usd, 54.328_462_87);
    assert_eq!(summary.account_im_rate, 0.021);
    assert_eq!(summary.account_mm_rate, 0.009);
    assert!(summary.problem.is_none());
}

#[tokio::test]
async fn api_error_propagates() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v5/market/tickers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "retCode": 10001,
            "retMsg": "param error",
            "result": {"list": []}
        })))
        .mount(&server)
        .await;

    let b = bybit(server.uri(), None);
    let err = b.get_funding_rate("BTC").await.expect_err("api error");
    match err {
        exchange::ExchangeError::Api {
            exchange,
            code,
            message,
        } => {
            assert_eq!(exchange, "bybit");
            assert_eq!(code, "10001");
            assert!(message.contains("param error"));
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn missing_credentials_returns_auth_error() {
    let server = MockServer::start().await;
    let b = bybit(server.uri(), None);
    let err = b.get_balance(None).await.expect_err("auth");
    assert!(matches!(err, exchange::ExchangeError::Auth(_)));
}

#[tokio::test]
async fn live_account_mode_reads_one_way_position_idx() {
    let server = MockServer::start().await;
    mount_account_mode_rows(&server, json!([{"symbol": "BTCUSDT", "positionIdx": 0}])).await;

    let b = bybit(server.uri(), Some(credentials()));
    let info = LiveTradingAdapter::get_exchange_account_mode(&b, "bybit")
        .await
        .expect("account mode")
        .expect("mode present");

    assert_eq!(info.venue, "bybit");
    assert_eq!(info.mode, "one_way");
    assert_eq!(info.source, "bybit.GET /v5/position/list positionIdx");
    assert_eq!(info.account_scope.as_deref(), Some("linear_usdt"));
    assert_eq!(info.freshness_ms, Some(0));
    assert!(info.checked_at_ms > 0);
}

#[tokio::test]
async fn live_account_mode_reads_hedge_position_idx() {
    let server = MockServer::start().await;
    mount_account_mode_rows(
        &server,
        json!([
            {"symbol": "BTCUSDT", "positionIdx": 1},
            {"symbol": "BTCUSDT", "positionIdx": 2}
        ]),
    )
    .await;

    let b = bybit(server.uri(), Some(credentials()));
    let info = LiveTradingAdapter::get_exchange_account_mode(&b, "bybit")
        .await
        .expect("account mode")
        .expect("mode present");

    assert_eq!(info.mode, "hedge");
    assert_eq!(info.account_scope.as_deref(), Some("linear_usdt"));
}

#[tokio::test]
async fn live_account_mode_returns_none_for_unscoped_empty_position_list() {
    let server = MockServer::start().await;
    mount_account_mode_rows(&server, json!([])).await;

    let b = bybit(server.uri(), Some(credentials()));
    let info = LiveTradingAdapter::get_exchange_account_mode(&b, "bybit")
        .await
        .expect("account mode");

    assert!(info.is_none());
}

#[tokio::test]
async fn live_account_mode_queries_symbol_position_idx() {
    let server = MockServer::start().await;
    mount_symbol_account_mode_rows(
        &server,
        "ETHUSDT",
        json!([{"symbol": "ETHUSDT", "positionIdx": 0}]),
    )
    .await;

    let b = bybit(server.uri(), Some(credentials()));
    let info = LiveTradingAdapter::get_exchange_symbol_account_mode(&b, "bybit", "ETH")
        .await
        .expect("account mode")
        .expect("mode present");

    assert_eq!(info.mode, "one_way");
    assert_eq!(info.account_scope.as_deref(), Some("linear_usdt:ETHUSDT"));
}

#[tokio::test]
async fn live_positions_queries_position_list_with_signed_headers() {
    let server = MockServer::start().await;
    let body: Value = serde_json::from_str(include_str!(
        "../fixtures/bybit/position_list_linear_open.json"
    ))
    .expect("positions fixture");

    Mock::given(method("GET"))
        .and(path("/v5/position/list"))
        .and(query_param("category", "linear"))
        .and(query_param("symbol", "BTCUSDT"))
        .and(header_exists("X-BAPI-API-KEY"))
        .and(header_exists("X-BAPI-SIGN"))
        .and(header_exists("X-BAPI-TIMESTAMP"))
        .and(header_exists("X-BAPI-RECV-WINDOW"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let b = bybit(server.uri(), Some(credentials()));
    let positions = ExchangeAdapter::get_positions(&b, Some("BTC"))
        .await
        .expect("positions");

    assert_eq!(positions.len(), 2);
    assert!(positions.iter().any(|p| p.side == "long"));
    assert!(positions.iter().any(|p| p.side == "short"));
}

#[tokio::test]
async fn live_account_mode_rejects_mixed_position_idx() {
    let server = MockServer::start().await;
    mount_account_mode_rows(
        &server,
        json!([
            {"symbol": "BTCUSDT", "positionIdx": 0},
            {"symbol": "ETHUSDT", "positionIdx": 1}
        ]),
    )
    .await;

    let b = bybit(server.uri(), Some(credentials()));
    let err = LiveTradingAdapter::get_exchange_account_mode(&b, "bybit")
        .await
        .expect_err("mixed evidence");

    match err {
        exchange::ExchangeError::Parse(message) => assert!(message.contains("mixed")),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn live_place_order_sends_v5_limit_order() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v5/position/list"))
        .and(query_param("category", "linear"))
        .and(query_param("symbol", "BTCUSDT"))
        .and(header_exists("X-BAPI-SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": {
                "list": [{"symbol": "BTCUSDT", "positionIdx": 0}]
            }
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v5/order/create"))
        .and(header_exists("X-BAPI-API-KEY"))
        .and(header_exists("X-BAPI-SIGN"))
        .and(header_exists("X-BAPI-TIMESTAMP"))
        .and(header_exists("X-BAPI-RECV-WINDOW"))
        .and(body_partial_json(json!({
            "category": "linear",
            "symbol": "BTCUSDT",
            "side": "Buy",
            "orderType": "Limit",
            "qty": "0.01",
            "price": "30000",
            "timeInForce": "IOC",
            "positionIdx": 0,
            "orderLinkId": "client-1"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": {"orderId": "bybit-1", "orderLinkId": "client-1"}
        })))
        .mount(&server)
        .await;

    let b = bybit(server.uri(), Some(credentials()));
    let ack = LiveTradingAdapter::place_order(&b, &limit_intent())
        .await
        .expect("place order");
    assert_eq!(ack.exchange_order_id.as_deref(), Some("bybit-1"));
    assert_eq!(ack.client_order_id, "client-1");
    assert_eq!(ack.state, LiveOrderState::Accepted);
}

#[tokio::test]
async fn live_place_market_order_sends_official_slippage() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v5/position/list"))
        .and(query_param("category", "linear"))
        .and(query_param("symbol", "BTCUSDT"))
        .and(header_exists("X-BAPI-SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": {
                "list": [{"symbol": "BTCUSDT", "positionIdx": 0}]
            }
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v5/order/create"))
        .and(header_exists("X-BAPI-API-KEY"))
        .and(header_exists("X-BAPI-SIGN"))
        .and(body_partial_json(json!({
            "category": "linear",
            "symbol": "BTCUSDT",
            "side": "Buy",
            "orderType": "Market",
            "qty": "0.01",
            "positionIdx": 0,
            "orderLinkId": "client-1",
            "slippageToleranceType": "Percent",
            "slippageTolerance": "0.05"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": {"orderId": "bybit-1", "orderLinkId": "client-1"}
        })))
        .mount(&server)
        .await;

    let b = bybit(server.uri(), Some(credentials()));
    let ack = LiveTradingAdapter::place_order(&b, &market_intent())
        .await
        .expect("place market order");
    assert_eq!(ack.exchange_order_id.as_deref(), Some("bybit-1"));
    assert_eq!(ack.client_order_id, "client-1");
    assert_eq!(ack.state, LiveOrderState::Accepted);
}

#[tokio::test]
async fn live_cancel_order_returns_cancel_requested() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v5/order/cancel"))
        .and(header_exists("X-BAPI-API-KEY"))
        .and(header_exists("X-BAPI-SIGN"))
        .and(body_partial_json(json!({
            "category": "linear",
            "symbol": "BTCUSDT",
            "orderId": "bybit-1",
            "orderLinkId": "client-1"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": {"orderId": "bybit-1", "orderLinkId": "client-1"}
        })))
        .mount(&server)
        .await;

    let b = bybit(server.uri(), Some(credentials()));
    let ack = LiveTradingAdapter::cancel_order(&b, &cancel_request())
        .await
        .expect("cancel order");
    assert_eq!(ack.exchange_order_id.as_deref(), Some("bybit-1"));
    assert_eq!(ack.state, LiveOrderState::CancelRequested);
    assert!(ack.message.unwrap().contains("final state"));
}

#[tokio::test]
async fn live_get_order_queries_realtime_by_order_link_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v5/order/realtime"))
        .and(query_param("category", "linear"))
        .and(query_param("symbol", "BTCUSDT"))
        .and(query_param("orderLinkId", "client-1"))
        .and(header_exists("X-BAPI-SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": {
                "list": [{
                    "symbol": "BTCUSDT",
                    "orderId": "bybit-1",
                    "orderStatus": "New",
                    "orderType": "Limit",
                    "side": "Buy",
                    "price": "30000",
                    "qty": "0.01",
                    "cumExecQty": "0",
                    "avgPrice": "0",
                    "createdTime": "1700000000000",
                    "cumExecFee": "0",
                    "positionIdx": 0,
                    "cancelType": "UNKNOWN",
                    "rejectReason": "EC_NoError",
                    "leavesQty": "0.01",
                    "timeInForce": "GTC",
                    "orderLinkId": "client-1",
                    "reduceOnly": false
                }]
            }
        })))
        .mount(&server)
        .await;

    let b = bybit(server.uri(), Some(credentials()));
    let order = LiveTradingAdapter::get_order(&b, "BTC", "client-1")
        .await
        .expect("get order")
        .expect("order exists");
    assert_eq!(order.order_id, "bybit-1");
    assert_eq!(order.symbol, "BTC");
    assert_eq!(order.quantity, 0.01);
}

fn credentials() -> BybitCredentials {
    BybitCredentials {
        api_key: "test-key".into(),
        api_secret: "test-secret".into(),
    }
}

async fn mount_account_mode_rows(server: &MockServer, rows: Value) {
    Mock::given(method("GET"))
        .and(path("/v5/position/list"))
        .and(query_param("category", "linear"))
        .and(query_param("settleCoin", "USDT"))
        .and(header_exists("X-BAPI-SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": {
                "list": rows
            }
        })))
        .mount(server)
        .await;
}

async fn mount_symbol_account_mode_rows(server: &MockServer, symbol: &str, rows: Value) {
    Mock::given(method("GET"))
        .and(path("/v5/position/list"))
        .and(query_param("category", "linear"))
        .and(query_param("symbol", symbol))
        .and(header_exists("X-BAPI-SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": {
                "list": rows
            }
        })))
        .mount(server)
        .await;
}

fn limit_intent() -> OrderIntent {
    OrderIntent {
        id: "internal-1".into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: "bybit".into(),
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
        created_at_ms: common::time::now_ms(),
    }
}

fn market_intent() -> OrderIntent {
    let mut intent = limit_intent();
    intent.order_type = OrderType::Market;
    intent.price = Some(30_015.0);
    intent.slippage_tolerance_bps = Some(5.0);
    intent
}

fn cancel_request() -> CancelOrderRequest {
    CancelOrderRequest {
        exchange: "bybit".into(),
        symbol: "BTC".into(),
        internal_order_id: "internal-1".into(),
        exchange_order_id: Some("bybit-1".into()),
        client_order_id: "client-1".into(),
    }
}
