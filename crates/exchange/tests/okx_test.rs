#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! OKX 适配器集成测试（wiremock）。

use exchange::adapters::OkxTdMode;
use exchange::{
    ExchangeAdapter, LiveTradingAdapter, Okx, OkxConfig, OkxCredentials, OkxLive, OkxLiveConfig,
    OkxLiveCredentials,
};
use serde_json::json;
use shared_types::{
    CancelOrderRequest, ExecutionMode, IndexCompositionQuality, MarginMode, OrderIntent, OrderSide,
    OrderSource, OrderType, TimeInForce,
};
use wiremock::matchers::{body_partial_json, header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn okx(server_uri: String, credentials: Option<OkxCredentials>) -> Okx {
    Okx::new(OkxConfig {
        credentials,
        timeout_secs: 5,
        qps: 100,
        base_url_override: Some(server_uri),
    })
    .unwrap()
}

fn okx_live(server_uri: String) -> OkxLive {
    OkxLive::new(OkxLiveConfig {
        credentials: OkxLiveCredentials {
            api_key: "k".into(),
            api_secret: "s".into(),
            passphrase: "p".into(),
        },
        testnet: false,
        timeout_secs: 5,
        qps: 100,
        base_url_override: Some(server_uri),
        td_mode: OkxTdMode::Cross,
    })
    .unwrap()
}

#[tokio::test]
async fn get_funding_rates_resolves_inst_ids_and_joins_volume() {
    let server = MockServer::start().await;

    // Step 1: instruments 列表
    Mock::given(method("GET"))
        .and(path("/api/v5/public/instruments"))
        .and(query_param("instType", "SWAP"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "msg": "",
            "data": [
                {"instId": "BTC-USDT-SWAP"},
                {"instId": "ETH-USDT-SWAP"},
                {"instId": "BTC-USD-SWAP"} // 反向合约，应被过滤
            ]
        })))
        .mount(&server)
        .await;

    // Step 2: tickers 全量
    Mock::given(method("GET"))
        .and(path("/api/v5/market/tickers"))
        .and(query_param("instType", "SWAP"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "data": [
                {
                    "instId": "BTC-USDT-SWAP",
                    "last": "30000",
                    "bidPx": "29999",
                    "askPx": "30001",
                    "volCcy24h": "1500000000",
                    "ts": "1700000000000"
                },
                {
                    "instId": "ETH-USDT-SWAP",
                    "last": "2000",
                    "bidPx": "1999",
                    "askPx": "2001",
                    "volCcy24h": "500000000",
                    "ts": "1700000000000"
                }
            ]
        })))
        .mount(&server)
        .await;

    // Step 3: per-instrument funding-rate
    Mock::given(method("GET"))
        .and(path("/api/v5/public/funding-rate"))
        .and(query_param("instId", "BTC-USDT-SWAP"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "data": [{
                "instId": "BTC-USDT-SWAP",
                "fundingRate": "0.0001",
                "fundingTime": "1700000000000",
                "nextFundingTime": "1700028800000"
            }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v5/public/funding-rate"))
        .and(query_param("instId", "ETH-USDT-SWAP"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "data": [{
                "instId": "ETH-USDT-SWAP",
                "fundingRate": "-0.00005",
                "fundingTime": "1700000000000",
                "nextFundingTime": "1700028800000"
            }]
        })))
        .mount(&server)
        .await;

    let o = okx(server.uri(), None);
    let rates = o.get_funding_rates(None).await.expect("fetch ok");

    assert_eq!(rates.len(), 2, "BTC-USD-SWAP 应被过滤");
    let btc = rates.iter().find(|r| r.symbol == "BTC").unwrap();
    assert_eq!(btc.exchange, "okx");
    assert!((btc.rate - 0.0001).abs() < 1e-12);
    assert_eq!(btc.funding_interval, 8);
    assert!((btc.volume_24h - 1_500_000_000.0).abs() < 1.0);

    let eth = rates.iter().find(|r| r.symbol == "ETH").unwrap();
    assert!((eth.rate - (-0.00005)).abs() < 1e-12);
}

#[tokio::test]
async fn get_funding_rates_retries_failed_fast_item_once() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v5/public/instruments"))
        .and(query_param("instType", "SWAP"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "msg": "",
            "data": [
                {"instId": "BTC-USDT-SWAP"},
                {"instId": "ETH-USDT-SWAP"}
            ]
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/v5/market/tickers"))
        .and(query_param("instType", "SWAP"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "data": [
                {"instId": "BTC-USDT-SWAP", "last": "30000", "bidPx": "29999", "askPx": "30001", "volCcy24h": "1000", "ts": "1700000000000"},
                {"instId": "ETH-USDT-SWAP", "last": "2000", "bidPx": "1999", "askPx": "2001", "volCcy24h": "2000", "ts": "1700000000000"}
            ]
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/v5/public/funding-rate"))
        .and(query_param("instId", "BTC-USDT-SWAP"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "data": [{
                "instId": "BTC-USDT-SWAP",
                "fundingRate": "0.0001",
                "fundingTime": "1700000000000",
                "nextFundingTime": "1700028800000"
            }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v5/public/funding-rate"))
        .and(query_param("instId", "ETH-USDT-SWAP"))
        .respond_with(ResponseTemplate::new(503).set_body_string("temporary"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v5/public/funding-rate"))
        .and(query_param("instId", "ETH-USDT-SWAP"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "data": [{
                "instId": "ETH-USDT-SWAP",
                "fundingRate": "-0.00005",
                "fundingTime": "1700000000000",
                "nextFundingTime": "1700028800000"
            }]
        })))
        .mount(&server)
        .await;

    let o = okx(server.uri(), None);
    let rates = o.get_funding_rates(None).await.expect("fetch ok");

    assert_eq!(rates.len(), 2);
    assert!(rates.iter().any(|r| r.symbol == "BTC"));
    let eth = rates.iter().find(|r| r.symbol == "ETH").unwrap();
    assert!((eth.rate - (-0.00005)).abs() < 1e-12);
}

#[tokio::test]
async fn get_funding_rate_single() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v5/public/funding-rate"))
        .and(query_param("instId", "BTC-USDT-SWAP"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "data": [{
                "instId": "BTC-USDT-SWAP",
                "fundingRate": "0.0003",
                "fundingTime": "1700000000000",
                "nextFundingTime": "1700028800000"
            }]
        })))
        .mount(&server)
        .await;

    let o = okx(server.uri(), None);
    let rate = o.get_funding_rate("BTC").await.expect("ok");
    assert_eq!(rate.symbol, "BTC");
    assert!((rate.rate - 0.0003).abs() < 1e-12);
}

#[tokio::test]
async fn get_tickers_filters_swap_usdt() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v5/market/tickers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "data": [
                {"instId": "BTC-USDT-SWAP", "last": "30000", "bidPx": "29999", "askPx": "30001", "volCcy24h": "100", "ts": "1"},
                {"instId": "BTC-USD-SWAP", "last": "30000", "bidPx": "29999", "askPx": "30001", "volCcy24h": "50", "ts": "1"}
            ]
        })))
        .mount(&server)
        .await;

    let o = okx(server.uri(), None);
    let tickers = o.get_tickers(None).await.expect("ok");
    assert_eq!(tickers.len(), 1);
    assert_eq!(tickers[0].symbol, "BTC");
}

#[tokio::test]
async fn get_orderbook_parses_4col_levels() {
    let server = MockServer::start().await;
    mock_public_swap_instruments(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v5/market/books"))
        .and(query_param("instId", "BTC-USDT-SWAP"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "data": [{
                "bids": [["30000.0", "1.5", "0", "1"], ["29999.5", "2.0", "0", "2"]],
                "asks": [["30001.0", "1.0", "0", "1"], ["30001.5", "0.8", "0", "1"]],
                "ts": "1700000000000"
            }]
        })))
        .mount(&server)
        .await;

    let o = okx(server.uri(), None);
    let ob = o.get_orderbook("BTC", 20).await.expect("ok");
    assert_eq!(ob.symbol, "BTC");
    assert_eq!(ob.exchange, "okx");
    assert_eq!(ob.bids.len(), 2);
    assert_eq!(ob.asks.len(), 2);
    assert!((ob.bids[0][0] - 30000.0).abs() < 1e-9);
    assert!((ob.bids[0][1] - 0.015).abs() < 1e-9);
    assert_eq!(ob.timestamp, 1_700_000_000_000);
}

#[tokio::test]
async fn index_composition_components_follow_official_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v5/market/index-components"))
        .and(query_param("index", "BTC-USDT"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "msg": "",
            "data": {
                "components": [
                    {
                        "symbol": "BTC/USDT",
                        "symPx": "52733.2",
                        "wgt": "0.25",
                        "cnvPx": "52733.2",
                        "exch": "OKX"
                    },
                    {
                        "symbol": "BTC/USDT",
                        "symPx": "",
                        "wgt": "0.25",
                        "cnvPx": "52739.87000000",
                        "exch": "Binance"
                    }
                ],
                "last": "52735.4123234925",
                "index": "BTC-USDT",
                "ts": "1630985335599"
            }
        })))
        .mount(&server)
        .await;

    let o = okx(server.uri(), None);
    let snapshot = o
        .get_index_composition("BTC")
        .await
        .expect("index composition");

    assert_eq!(snapshot.venue, "okx");
    assert_eq!(snapshot.symbol, "BTC");
    assert_eq!(snapshot.index_id, "BTC-USDT");
    assert_eq!(snapshot.quality, IndexCompositionQuality::Verified);
    assert_eq!(snapshot.source, "GET /api/v5/market/index-components");
    assert_eq!(snapshot.received_at_ms, 1_630_985_335_599);
    assert_eq!(snapshot.components.len(), 2);
    assert_eq!(snapshot.components[0].name, "OKX");
    assert_eq!(snapshot.components[0].symbol, "BTC/USDT");
    assert!((snapshot.components[0].weight - 0.25).abs() < 1e-12);
    assert_eq!(snapshot.components[1].price, None);
}

#[tokio::test]
async fn get_balance_sends_all_signed_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v5/account/balance"))
        .and(header_exists("OK-ACCESS-KEY"))
        .and(header_exists("OK-ACCESS-SIGN"))
        .and(header_exists("OK-ACCESS-TIMESTAMP"))
        .and(header_exists("OK-ACCESS-PASSPHRASE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "data": [{
                "details": [
                    {"ccy": "USDT", "eq": "10000", "availBal": "9000", "frozenBal": "1000", "upl": "5"},
                    {"ccy": "BTC", "eq": "0.5", "availBal": "0.5", "frozenBal": "0", "upl": "0"}
                ]
            }]
        })))
        .mount(&server)
        .await;

    let o = okx(
        server.uri(),
        Some(OkxCredentials {
            api_key: "k".into(),
            api_secret: "s".into(),
            passphrase: "p".into(),
        }),
    );
    let balances = o.get_balance(None).await.expect("ok");
    assert_eq!(balances.len(), 2);
    let usdt = balances.get("USDT").unwrap();
    assert!((usdt.total - 10000.0).abs() < 1e-9);
    assert!((usdt.available - 9000.0).abs() < 1e-9);
    assert!((usdt.frozen - 1000.0).abs() < 1e-9);
    assert!((usdt.unrealized_pnl - 5.0).abs() < 1e-9);
}

#[tokio::test]
async fn live_get_balances_sends_account_balance_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v5/account/balance"))
        .and(header_exists("OK-ACCESS-KEY"))
        .and(header_exists("OK-ACCESS-SIGN"))
        .and(header_exists("OK-ACCESS-TIMESTAMP"))
        .and(header_exists("OK-ACCESS-PASSPHRASE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "data": [{
                "details": [
                    {"ccy": "USDT", "eq": "10000", "availBal": "9000", "frozenBal": "1000", "upl": "5"}
                ]
            }]
        })))
        .mount(&server)
        .await;

    let adapter = okx_live(server.uri());
    let balances = LiveTradingAdapter::get_balances(&adapter, None)
        .await
        .expect("live balance");

    assert_eq!(balances.len(), 1);
    assert_eq!(balances[0].venue, "okx");
    assert_eq!(balances[0].currency, "USDT");
    assert!((balances[0].available - 9000.0).abs() < 1e-9);
}

#[tokio::test]
async fn live_positions_queries_account_positions_with_signed_headers() {
    let server = MockServer::start().await;
    let body: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/okx/account_positions_swap.json"))
            .expect("positions fixture");

    Mock::given(method("GET"))
        .and(path("/api/v5/account/positions"))
        .and(query_param("instType", "SWAP"))
        .and(header_exists("OK-ACCESS-KEY"))
        .and(header_exists("OK-ACCESS-SIGN"))
        .and(header_exists("OK-ACCESS-TIMESTAMP"))
        .and(header_exists("OK-ACCESS-PASSPHRASE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let adapter = okx(
        server.uri(),
        Some(OkxCredentials {
            api_key: "k".into(),
            api_secret: "s".into(),
            passphrase: "p".into(),
        }),
    );
    let positions = ExchangeAdapter::get_positions(&adapter, Some("BTC"))
        .await
        .expect("positions");

    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0].symbol, "BTC");
    assert_eq!(positions[0].side, "long");
}

#[tokio::test]
async fn live_place_order_uses_official_instrument_contract_sizing() {
    let server = MockServer::start().await;
    mock_live_instrument(&server).await;
    mock_account_config(&server, "net_mode").await;

    Mock::given(method("POST"))
        .and(path("/api/v5/trade/order"))
        .and(header_exists("OK-ACCESS-KEY"))
        .and(header_exists("OK-ACCESS-SIGN"))
        .and(header_exists("OK-ACCESS-TIMESTAMP"))
        .and(header_exists("OK-ACCESS-PASSPHRASE"))
        .and(body_partial_json(json!({
            "instId": "BTC-USDT-SWAP",
            "tdMode": "cross",
            "side": "buy",
            "posSide": "net",
            "ordType": "limit",
            "sz": "2",
            "px": "50000.1",
            "clOrdId": "cid1"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "data": [{
                "ordId": "7",
                "clOrdId": "cid1",
                "sCode": "0",
                "sMsg": ""
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let adapter = okx_live(server.uri());
    let ack = LiveTradingAdapter::place_order(&adapter, &order_intent(0.02, Some(50_000.1)))
        .await
        .expect("place order");

    assert_eq!(ack.exchange_order_id.as_deref(), Some("7"));
    assert_eq!(ack.client_order_id, "cid1");
}

#[tokio::test]
async fn live_cancel_order_uses_official_cancel_order_path_and_client_order_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v5/trade/cancel-order"))
        .and(header_exists("OK-ACCESS-KEY"))
        .and(header_exists("OK-ACCESS-SIGN"))
        .and(header_exists("OK-ACCESS-TIMESTAMP"))
        .and(header_exists("OK-ACCESS-PASSPHRASE"))
        .and(body_partial_json(json!({
            "instId": "BTC-USDT-SWAP",
            "clOrdId": "cid1"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "data": [{
                "ordId": "12345689",
                "clOrdId": "cid1",
                "sCode": "0",
                "sMsg": ""
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let adapter = okx_live(server.uri());
    let ack = LiveTradingAdapter::cancel_order(&adapter, &cancel_request())
        .await
        .expect("cancel order");

    assert_eq!(ack.exchange_order_id.as_deref(), Some("12345689"));
    assert_eq!(ack.client_order_id, "cid1");
}

#[tokio::test]
async fn live_place_order_rejects_fractional_okx_contract_size() {
    let server = MockServer::start().await;
    mock_live_instrument(&server).await;

    let adapter = okx_live(server.uri());
    let err = LiveTradingAdapter::place_order(&adapter, &order_intent(0.015, Some(50_000.1)))
        .await
        .expect_err("fractional contract");

    assert!(err.to_string().contains("lotSz"));
}

#[tokio::test]
async fn live_get_order_empty_success_is_none() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v5/trade/order"))
        .and(query_param("instId", "BTC-USDT-SWAP"))
        .and(query_param("clOrdId", "cid1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "data": []
        })))
        .mount(&server)
        .await;

    let adapter = okx_live(server.uri());
    let order = LiveTradingAdapter::get_order(&adapter, "BTC", "cid1")
        .await
        .expect("empty success");

    assert!(order.is_none());
}

#[tokio::test]
async fn live_open_orders_queries_orders_pending_with_signed_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v5/trade/orders-pending"))
        .and(query_param("instType", "SWAP"))
        .and(query_param("instId", "BTC-USDT-SWAP"))
        .and(header_exists("OK-ACCESS-KEY"))
        .and(header_exists("OK-ACCESS-SIGN"))
        .and(header_exists("OK-ACCESS-TIMESTAMP"))
        .and(header_exists("OK-ACCESS-PASSPHRASE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "data": [{
                "instId": "BTC-USDT-SWAP",
                "ordId": "590908157585625111",
                "state": "live",
                "ordType": "limit",
                "side": "buy",
                "px": "30000",
                "sz": "1",
                "accFillSz": "0.25",
                "avgPx": "29990",
                "cTime": "1700000000000",
                "clOrdId": "okx-cli-1",
                "reduceOnly": "false"
            }]
        })))
        .mount(&server)
        .await;

    let adapter = okx_live(server.uri());
    let orders = LiveTradingAdapter::get_open_orders(&adapter, Some("BTC"))
        .await
        .expect("open orders");

    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].order_id, "590908157585625111");
    assert!(matches!(orders[0].status, shared_types::OrderStatus::Open));
}

#[tokio::test]
async fn live_get_order_parse_error_is_not_none() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v5/trade/order"))
        .and(query_param("instId", "BTC-USDT-SWAP"))
        .and(query_param("clOrdId", "cid1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "data": [{
                "instId": "BTC-USDT-SWAP",
                "ordId": 123,
                "state": "filled",
                "ordType": "limit",
                "side": "buy"
            }]
        })))
        .mount(&server)
        .await;

    let adapter = okx_live(server.uri());
    let err = LiveTradingAdapter::get_order(&adapter, "BTC", "cid1")
        .await
        .expect_err("malformed order row");

    assert!(matches!(err, exchange::ExchangeError::Parse(_)));
}

#[tokio::test]
async fn api_error_propagates_from_non_zero_code() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v5/public/funding-rate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "50001",
            "msg": "system error",
            "data": []
        })))
        .mount(&server)
        .await;

    let o = okx(server.uri(), None);
    let err = o.get_funding_rate("BTC").await.expect_err("api error");
    match err {
        exchange::ExchangeError::Api {
            exchange,
            code,
            message,
        } => {
            assert_eq!(exchange, "okx");
            assert_eq!(code, "50001");
            assert!(message.contains("system error"));
        }
        other => panic!("unexpected error variant: {other:?}"),
    }
}

#[tokio::test]
async fn missing_credentials_returns_auth_error() {
    let server = MockServer::start().await;
    let o = okx(server.uri(), None);
    let err = o
        .get_balance(None)
        .await
        .expect_err("should require credentials");
    assert!(matches!(err, exchange::ExchangeError::Auth(_)));
}

async fn mock_live_instrument(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/v5/public/instruments"))
        .and(query_param("instType", "SWAP"))
        .and(query_param("instId", "BTC-USDT-SWAP"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "data": [{
                "instId": "BTC-USDT-SWAP",
                "ctVal": "0.01",
                "ctValCcy": "BTC",
                "lotSz": "1",
                "minSz": "1",
                "tickSz": "0.1",
                "state": "live"
            }]
        })))
        .expect(1)
        .mount(server)
        .await;
}

async fn mock_public_swap_instruments(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/v5/public/instruments"))
        .and(query_param("instType", "SWAP"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../fixtures/okx/public_instruments_swap.json")),
        )
        .expect(1)
        .mount(server)
        .await;
}

async fn mock_account_config(server: &MockServer, pos_mode: &str) {
    Mock::given(method("GET"))
        .and(path("/api/v5/account/config"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "data": [{
                "posMode": pos_mode
            }]
        })))
        .expect(1)
        .mount(server)
        .await;
}

fn order_intent(quantity: f64, price: Option<f64>) -> OrderIntent {
    OrderIntent {
        id: "i1".into(),
        source: OrderSource::ArbitragePreview,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: "okx".into(),
        symbol: "BTC".into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity,
        price,
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Gtc,
        post_only: false,
        margin_mode: MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "cid1".into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

fn cancel_request() -> CancelOrderRequest {
    CancelOrderRequest {
        exchange: "okx".into(),
        symbol: "BTC".into(),
        internal_order_id: "i1".into(),
        exchange_order_id: Some("12345689".into()),
        client_order_id: "cid1".into(),
    }
}
