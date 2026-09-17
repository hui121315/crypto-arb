#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! Bitget 适配器集成测试（wiremock）。

use exchange::{Bitget, BitgetConfig, BitgetCredentials, ExchangeAdapter, LiveTradingAdapter};
use serde_json::json;
use shared_types::{
    CancelOrderRequest, ExecutionMode, IndexCompositionQuality, LiveOrderState, OrderIntent,
    OrderSide, OrderSource, OrderType,
};
use wiremock::matchers::{body_partial_json, header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn bitget(server_uri: String, credentials: Option<BitgetCredentials>) -> Bitget {
    Bitget::new(BitgetConfig {
        credentials,
        allow_live_writes: true,
        timeout_secs: 5,
        qps: 100,
        base_url_override: Some(server_uri),
        ..Default::default()
    })
    .unwrap()
}

async fn mount_instrument_identity_matrix(server: &MockServer) {
    let source: serde_json::Value = serde_json::from_str(include_str!(
        "../fixtures/bitget/uta_instruments_identity_matrix.json"
    ))
    .expect("identity matrix fixture");
    for category in ["USDT-FUTURES", "USDC-FUTURES", "COIN-FUTURES"] {
        let rows = source["data"]
            .as_array()
            .expect("matrix rows")
            .iter()
            .filter(|row| row["category"] == category)
            .cloned()
            .collect::<Vec<_>>();
        Mock::given(method("GET"))
            .and(path("/api/v3/market/instruments"))
            .and(query_param("category", category))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "code": "00000",
                "msg": "success",
                "data": rows
            })))
            .expect(1)
            .mount(server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/api/v3/market/instruments"))
        .and(query_param("category", "SPOT"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "00000",
            "msg": "success",
            "data": [{
                "symbol": "BTCUSDT",
                "category": "SPOT",
                "baseCoin": "BTC",
                "quoteCoin": "USDT",
                "status": "online",
                "symbolType": "crypto",
                "isRwa": "NO",
                "isReality": "NO",
                "pricePrecision": "2",
                "quantityPrecision": "6",
                "minOrderQty": "0.000001",
                "minOrderAmount": "1"
            }]
        })))
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_hedge_account_mode(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/v3/account/settings"))
        .and(header_exists("ACCESS-SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "00000",
            "msg": "success",
            "data": {"holdMode": "hedge_mode"}
        })))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn refresh_instrument_specs_fans_out_all_uta_product_categories() {
    let server = MockServer::start().await;
    mount_instrument_identity_matrix(&server).await;
    let rows = bitget(server.uri(), None)
        .fetch_instruments()
        .await
        .expect("Bitget instrument matrix");
    assert_eq!(rows.len(), 5);
    assert!(rows.iter().any(|row| {
        row.native_symbol == "BTCPERP" && row.execution_supported && row.is_hedge_constructible()
    }));
    assert!(rows.iter().any(|row| {
        row.native_symbol == "BTCUSD_CM" && !row.execution_supported && row.is_observation_only()
    }));
    assert!(rows.iter().any(|row| {
        row.native_symbol == "TSLAUSDT" && !row.execution_supported && row.is_observation_only()
    }));
    assert!(rows.iter().any(|row| {
        row.native_symbol == "BTCUSDT"
            && row.product_type.as_deref() == Some("spot")
            && row.execution_supported
            && row.is_hedge_constructible()
    }));
}

#[tokio::test]
async fn batch_funding_rates_with_volume_join() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v3/market/tickers"))
        .and(query_param("category", "USDT-FUTURES"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "00000",
            "data": [
                {"symbol": "BTCUSDT", "lastPrice": "30000", "bid1Price": "29999", "ask1Price": "30001", "volume24h": "50000", "turnover24h": "1500000000", "fundingRate": "0.0001", "nextFundingTime": "1700028800000", "ts": "1700000000000"},
                {"symbol": "ETHUSDT", "lastPrice": "2000", "bid1Price": "1999", "ask1Price": "2001", "volume24h": "250000", "turnover24h": "500000000", "fundingRate": "-0.00005", "nextFundingTime": "1700028800000", "ts": "1700000000000"},
                {"symbol": "DERIVEDNEXTUSDT", "lastPrice": "1", "bid1Price": "1", "ask1Price": "1", "volume24h": "1", "turnover24h": "1", "fundingRate": "0.0002", "ts": "1700000000000"},
                {"symbol": "BADINTERVALUSDT", "lastPrice": "1", "bid1Price": "1", "ask1Price": "1", "volume24h": "1", "turnover24h": "1", "fundingRate": "0.0002", "nextFundingTime": "1700028800000", "ts": "1700000000000"}
            ]
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/v3/market/instruments"))
        .and(query_param("category", "USDT-FUTURES"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "00000",
            "data": [
                {"symbol": "BTCUSDT", "category": "USDT-FUTURES", "baseCoin": "BTC", "quoteCoin": "USDT", "isRwa": "NO", "symbolType": "crypto", "status": "online", "fundInterval": "8"},
                {"symbol": "ETHUSDT", "category": "USDT-FUTURES", "baseCoin": "ETH", "quoteCoin": "USDT", "isRwa": "NO", "symbolType": "crypto", "status": "online", "fundInterval": "8"},
                {"symbol": "DERIVEDNEXTUSDT", "category": "USDT-FUTURES", "baseCoin": "DERIVEDNEXT", "quoteCoin": "USDT", "isRwa": "NO", "symbolType": "crypto", "status": "online", "fundInterval": "8"},
                {"symbol": "BADINTERVALUSDT", "category": "USDT-FUTURES", "baseCoin": "BADINTERVAL", "quoteCoin": "USDT", "isRwa": "NO", "symbolType": "crypto", "status": "online", "fundInterval": "3"}
            ]
        })))
        .mount(&server)
        .await;

    let b = bitget(server.uri(), None);
    let rates = b.get_funding_rates(None).await.expect("ok");

    assert_eq!(rates.len(), 3);
    let btc = rates.iter().find(|r| r.symbol == "BTC").unwrap();
    assert_eq!(btc.exchange, "bitget");
    assert!((btc.rate - 0.0001).abs() < 1e-12);
    assert_eq!(btc.funding_interval, 8);
    assert!((btc.volume_24h - 1_500_000_000.0).abs() < 1.0);
    assert_eq!(btc.next_funding_time, 1_700_028_800_000);

    let eth = rates.iter().find(|r| r.symbol == "ETH").unwrap();
    assert!((eth.rate - (-0.00005)).abs() < 1e-12);

    let derived = rates.iter().find(|r| r.symbol == "DERIVEDNEXT").unwrap();
    assert_eq!(derived.next_funding_time, 1_700_006_400_000);
}

#[tokio::test]
async fn get_funding_rate_single_finds_in_batch() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v3/market/current-fund-rate"))
        .and(query_param("symbol", "BTCUSDT"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "00000",
            "data": [
                {"symbol": "BTCUSDT", "fundingRate": "0.0003", "nextUpdate": "1700028800000", "fundingRateInterval": "8"}
            ]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v3/market/tickers"))
        .and(query_param("category", "USDT-FUTURES"))
        .and(query_param("symbol", "BTCUSDT"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "00000",
            "data": []
        })))
        .mount(&server)
        .await;

    let b = bitget(server.uri(), None);
    let r = b.get_funding_rate("BTC").await.expect("ok");
    assert_eq!(r.symbol, "BTC");
    assert!((r.rate - 0.0003).abs() < 1e-12);
}

#[tokio::test]
async fn get_orderbook_parses_merge_depth_object() {
    let server = MockServer::start().await;
    // V3 `/api/v3/market/orderbook` uses short keys `a` / `b` and accepts an
    // arbitrary limit through 1000, so the adapter preserves `depth=7`.
    Mock::given(method("GET"))
        .and(path("/api/v3/market/orderbook"))
        .and(query_param("category", "USDT-FUTURES"))
        .and(query_param("symbol", "BTCUSDT"))
        .and(query_param("limit", "7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "00000",
            "msg": "success",
            "data": {
                "a": [[77440.4, 22.5899], ["77440.5", "0.0001"]],
                "b": [[77440.3, 2.5416], ["77440.2", "0.0001"]],
                "ts": "1779276531694"
            }
        })))
        .mount(&server)
        .await;

    let b = bitget(server.uri(), None);
    let book = b.get_orderbook("BTC", 7).await.expect("orderbook");

    assert_eq!(book.exchange, "bitget");
    assert_eq!(book.symbol, "BTC");
    assert_eq!(book.bids.len(), 2);
    assert_eq!(book.asks.len(), 2);
    assert!((book.bids[0][0] - 77440.3).abs() < 1e-9);
    assert!((book.asks[0][1] - 22.5899).abs() < 1e-9);
    assert_eq!(book.timestamp, 1_779_276_531_694);
}

#[tokio::test]
async fn index_composition_components_follow_official_endpoint() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v3/market/index-components"))
        .and(query_param("symbol", "BTCUSDT"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "00000",
            "msg": "success",
            "requestTime": 1_767_159_256_214_i64,
            "data": {
                "symbol": "BTCUSDT",
                "componentList": [
                    {
                        "exchange": "BITGET_FUTURE",
                        "spotPair": "BTC/USDT",
                        "equivalentPrice": "88432.1",
                        "weight": "0.4696"
                    },
                    {
                        "exchange": "OKX",
                        "spotPair": "BTC/USDT",
                        "equivalentPrice": "",
                        "weight": "0.0468"
                    }
                ]
            }
        })))
        .mount(&server)
        .await;

    let b = bitget(server.uri(), None);
    let snapshot = b
        .get_index_composition("BTC")
        .await
        .expect("index composition");

    assert_eq!(snapshot.venue, "bitget");
    assert_eq!(snapshot.symbol, "BTC");
    assert_eq!(snapshot.index_id, "BTCUSDT");
    assert_eq!(snapshot.quality, IndexCompositionQuality::Verified);
    assert_eq!(snapshot.source, "GET /api/v3/market/index-components");
    assert_eq!(snapshot.components.len(), 2);
    assert_eq!(snapshot.components[0].name, "BITGET_FUTURE");
    assert_eq!(snapshot.components[0].symbol, "BTC/USDT");
    assert!((snapshot.components[0].weight - 0.4696).abs() < 1e-12);
    assert_eq!(snapshot.components[1].price, None);
}

#[tokio::test]
async fn balance_signed_headers() {
    let server = MockServer::start().await;
    // V3 `/api/v3/account/assets` is a unified wallet endpoint keyed by
    // `coin`; data wraps account equity plus an `assets[]` list.
    Mock::given(method("GET"))
        .and(path("/api/v3/account/assets"))
        .and(header_exists("ACCESS-KEY"))
        .and(header_exists("ACCESS-SIGN"))
        .and(header_exists("ACCESS-TIMESTAMP"))
        .and(header_exists("ACCESS-PASSPHRASE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "00000",
            "data": {
                "accountEquity": "10005",
                "effEquity": "9000",
                "imr": "1000",
                "mmr": "100",
                "mgnRatio": "0.10",
                "positionMgnRatio": "0.01",
                "usdtEquity": "10005",
                "usdtUnrealisedPnl": "5",
                "assets": [{
                    "coin": "USDT",
                    "available": "9000",
                    "locked": "1000",
                    "balance": "10000",
                    "equity": "10000",
                    "usdValue": "10000"
                }]
            }
        })))
        .mount(&server)
        .await;

    let b = bitget(
        server.uri(),
        Some(BitgetCredentials {
            api_key: "k".into(),
            api_secret: "s".into(),
            passphrase: "p".into(),
        }),
    );
    let account = LiveTradingAdapter::get_account_read(&b, None)
        .await
        .expect("account read");
    let usdt = &account.balances[0];
    assert!((usdt.total - 10000.0).abs() < 1e-9);
    assert!((usdt.available - 9000.0).abs() < 1e-9);
    assert!((usdt.frozen - 1000.0).abs() < 1e-9);
    assert!((usdt.unrealized_pnl - 5.0).abs() < 1e-9);
    let summary = &account.summaries[0];
    assert_eq!(summary.total_equity_usd, 10005.0);
    assert_eq!(summary.total_available_balance_usd, 9000.0);
    assert_eq!(summary.total_initial_margin_usd, 1000.0);
    assert_eq!(summary.total_maintenance_margin_usd, 100.0);
    assert_eq!(summary.account_im_rate, 0.10);
    assert_eq!(summary.account_mm_rate, 0.01);
}

#[tokio::test]
async fn live_positions_queries_current_position_with_signed_headers() {
    let server = MockServer::start().await;
    let body: serde_json::Value = serde_json::from_str(include_str!(
        "../fixtures/bitget/uta_current_position_btcusdt.json"
    ))
    .expect("current-position fixture");

    Mock::given(method("GET"))
        .and(path("/api/v3/position/current-position"))
        .and(query_param("category", "USDT-FUTURES"))
        .and(query_param("symbol", "BTCUSDT"))
        .and(header_exists("ACCESS-KEY"))
        .and(header_exists("ACCESS-SIGN"))
        .and(header_exists("ACCESS-TIMESTAMP"))
        .and(header_exists("ACCESS-PASSPHRASE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let b = bitget(server.uri(), Some(credentials()));
    let positions = ExchangeAdapter::get_positions(&b, Some("BTC"))
        .await
        .expect("positions");

    assert_eq!(positions.len(), 2);
    assert!(positions.iter().any(|p| p.side == "long"));
    assert!(positions.iter().any(|p| p.side == "short"));
}

#[tokio::test]
async fn api_error_propagates_non_00000_code() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v3/market/tickers"))
        .and(query_param("category", "USDT-FUTURES"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "40001",
            "msg": "param error",
            "data": []
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v3/market/instruments"))
        .and(query_param("category", "USDT-FUTURES"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "00000",
            "data": []
        })))
        .mount(&server)
        .await;

    let b = bitget(server.uri(), None);
    let err = b.get_funding_rates(None).await.expect_err("api error");
    match err {
        exchange::ExchangeError::Api {
            exchange,
            code,
            message,
        } => {
            assert_eq!(exchange, "bitget");
            assert_eq!(code, "40001");
            assert!(message.contains("param error"));
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn missing_credentials_returns_auth_error() {
    let server = MockServer::start().await;
    let b = bitget(server.uri(), None);
    let err = b.get_balance(None).await.expect_err("auth");
    assert!(matches!(err, exchange::ExchangeError::Auth(_)));
}

#[tokio::test]
async fn live_place_order_sends_uta_v3_limit_order() {
    let server = MockServer::start().await;
    mount_instrument_identity_matrix(&server).await;
    mount_hedge_account_mode(&server).await;
    // V3 `POST /api/v3/trade/place-order` body swaps V2 `productType` for
    // `category`, V2 `size` for `qty`, and V2 `force` for `timeInForce`
    // (see UtaPlaceOrderBody).
    Mock::given(method("POST"))
        .and(path("/api/v3/trade/place-order"))
        .and(header_exists("ACCESS-KEY"))
        .and(header_exists("ACCESS-SIGN"))
        .and(header_exists("ACCESS-TIMESTAMP"))
        .and(header_exists("ACCESS-PASSPHRASE"))
        .and(body_partial_json(json!({
            "symbol": "BTCUSDT",
            "category": "USDT-FUTURES",
            "marginMode": "crossed",
            "qty": "0.01",
            "side": "buy",
            "orderType": "limit",
            "price": "30000",
            "timeInForce": "ioc",
            "clientOid": "client-1"
            ,"posSide": "long"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "00000",
            "msg": "success",
            "data": {"orderId": "bitget-1", "clientOid": "client-1"}
        })))
        .mount(&server)
        .await;

    let b = bitget(server.uri(), Some(credentials()));
    let ack = LiveTradingAdapter::place_order(&b, &limit_intent())
        .await
        .expect("place order");
    assert_eq!(ack.exchange_order_id.as_deref(), Some("bitget-1"));
    assert_eq!(ack.client_order_id, "client-1");
    assert_eq!(ack.state, LiveOrderState::Accepted);
}

#[tokio::test]
async fn live_place_order_recovers_unknown_result_by_client_oid_query() {
    let server = MockServer::start().await;
    mount_instrument_identity_matrix(&server).await;
    mount_hedge_account_mode(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/v3/trade/place-order"))
        .and(body_partial_json(json!({"clientOid": "client-1"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "40010",
            "msg": "Request timed out. Please query the order by clientOid",
            "data": null
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v3/trade/order-info"))
        .and(query_param("symbol", "BTCUSDT"))
        .and(query_param("category", "USDT-FUTURES"))
        .and(query_param("clientOid", "client-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "00000",
            "msg": "success",
            "data": {
                "symbol": "BTCUSDT",
                "orderId": "bitget-recovered-1",
                "orderStatus": "filled",
                "orderType": "limit",
                "timeInForce": "ioc",
                "side": "buy",
                "price": "30000",
                "qty": "0.01",
                "cumExecQty": "0.01",
                "avgPrice": "29999.5",
                "createdTime": "1700000000000",
                "clientOid": "client-1",
                "reduceOnly": "NO",
                "feeDetail": [{"feeCoin": "USDT", "fee": "0.12"}],
                "execType": "trade",
                "cancelReason": ""
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let b = bitget(server.uri(), Some(credentials()));
    let ack = LiveTradingAdapter::place_order(&b, &limit_intent())
        .await
        .expect("unknown result should recover from the read-side order query");

    assert_eq!(ack.exchange_order_id.as_deref(), Some("bitget-recovered-1"));
    assert_eq!(ack.state, LiveOrderState::Filled);
    assert_eq!(ack.filled_quantity, Some(0.01));
    assert_eq!(ack.filled_price, Some(29_999.5));
    assert_eq!(ack.filled_fee, Some(0.12));
    assert!(ack.message.unwrap().contains("using clientOid"));
}

#[tokio::test]
async fn live_place_order_unknown_result_fails_closed_when_query_has_no_order() {
    let server = MockServer::start().await;
    mount_instrument_identity_matrix(&server).await;
    mount_hedge_account_mode(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/v3/trade/place-order"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "45001",
            "msg": "Unknown order result; query by clientOid",
            "data": null
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v3/trade/order-info"))
        .and(query_param("clientOid", "client-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "00000",
            "msg": "success",
            "data": null
        })))
        .expect(1)
        .mount(&server)
        .await;

    let b = bitget(server.uri(), Some(credentials()));
    let error = LiveTradingAdapter::place_order(&b, &limit_intent())
        .await
        .expect_err("an unconfirmed write must remain failed closed");

    assert!(matches!(
        error,
        exchange::ExchangeError::Api { code, .. } if code == "45001"
    ));
}

#[tokio::test]
async fn live_cancel_order_returns_cancel_requested() {
    let server = MockServer::start().await;
    // V3 `POST /api/v3/trade/cancel-order` body uses `category` (no
    // `productType` / `marginCoin`); see UtaCancelOrderBody.
    Mock::given(method("POST"))
        .and(path("/api/v3/trade/cancel-order"))
        .and(header_exists("ACCESS-KEY"))
        .and(header_exists("ACCESS-SIGN"))
        .and(body_partial_json(json!({
            "symbol": "BTCUSDT",
            "category": "USDT-FUTURES",
            "orderId": "bitget-1",
            "clientOid": "client-1"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "00000",
            "msg": "success",
            "data": {"orderId": "bitget-1", "clientOid": "client-1"}
        })))
        .mount(&server)
        .await;

    let b = bitget(server.uri(), Some(credentials()));
    let ack = LiveTradingAdapter::cancel_order(&b, &cancel_request())
        .await
        .expect("cancel order");
    assert_eq!(ack.exchange_order_id.as_deref(), Some("bitget-1"));
    assert_eq!(ack.state, LiveOrderState::CancelRequested);
    assert!(ack.message.unwrap().contains("final state"));
}

#[tokio::test]
async fn safe_cancel_no_match_probe_uses_uta_v3_cancel_without_live_write_gate() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v3/trade/cancel-order"))
        .and(header_exists("ACCESS-KEY"))
        .and(header_exists("ACCESS-SIGN"))
        .and(body_partial_json(json!({
            "symbol": "BTCUSDT",
            "category": "USDT-FUTURES"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "40004",
            "msg": "order does not exist",
            "data": null
        })))
        .mount(&server)
        .await;

    let b = Bitget::new(BitgetConfig {
        credentials: Some(credentials()),
        allow_live_writes: false,
        timeout_secs: 5,
        qps: 100,
        base_url_override: Some(server.uri()),
        ..Default::default()
    })
    .unwrap();

    b.validate_safe_order_cancel_no_match_permission()
        .await
        .expect("nonmatching cancel proves signed cancel endpoint reachability without enabling live writes");
}

#[tokio::test]
async fn live_get_order_queries_detail_by_client_oid() {
    let server = MockServer::start().await;
    // V3 `GET /api/v3/trade/order-info` uses `category` query and the V3
    // UtaOrderRow schema (`qty`, `cumExecQty`, `avgPrice`, `createdTime`).
    Mock::given(method("GET"))
        .and(path("/api/v3/trade/order-info"))
        .and(query_param("symbol", "BTCUSDT"))
        .and(query_param("category", "USDT-FUTURES"))
        .and(query_param("clientOid", "client-1"))
        .and(header_exists("ACCESS-SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "00000",
            "msg": "success",
            "data": {
                "symbol": "BTCUSDT",
                "orderId": "bitget-1",
                "orderStatus": "live",
                "orderType": "limit",
                "timeInForce": "gtc",
                "side": "buy",
                "price": "30000",
                "qty": "0.01",
                "cumExecQty": "0",
                "avgPrice": "0",
                "createdTime": "1700000000000",
                "clientOid": "client-1",
                "reduceOnly": "NO"
            }
        })))
        .mount(&server)
        .await;

    let b = bitget(server.uri(), Some(credentials()));
    let order = LiveTradingAdapter::get_order(&b, "BTC", "client-1")
        .await
        .expect("get order")
        .expect("order exists");
    assert_eq!(order.order_id, "bitget-1");
    assert_eq!(order.symbol, "BTC");
    assert_eq!(order.quantity, 0.01);
}

#[tokio::test]
async fn live_open_orders_queries_unfilled_orders_with_v3_category() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v3/trade/unfilled-orders"))
        .and(query_param("category", "USDT-FUTURES"))
        .and(query_param("symbol", "BTCUSDT"))
        .and(header_exists("ACCESS-SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "00000",
            "msg": "success",
            "data": {
                "list": [{
                    "symbol": "BTCUSDT",
                    "orderId": "bitget-open-1",
                    "clientOid": "client-1",
                    "orderStatus": "live",
                    "orderType": "limit",
                    "timeInForce": "gtc",
                    "side": "buy",
                    "price": "30000",
                    "qty": "0.01",
                    "cumExecQty": "0",
                    "avgPrice": "0",
                    "createdTime": "1700000000000",
                    "reduceOnly": "NO"
                }],
                "cursor": ""
            }
        })))
        .mount(&server)
        .await;

    let b = bitget(server.uri(), Some(credentials()));
    let orders = LiveTradingAdapter::get_open_orders(&b, Some("BTC"))
        .await
        .expect("open orders");

    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].order_id, "bitget-open-1");
    assert_eq!(orders[0].symbol, "BTC");
    assert_eq!(orders[0].client_order_id.as_deref(), Some("client-1"));
}

fn credentials() -> BitgetCredentials {
    BitgetCredentials {
        api_key: "test-key".into(),
        api_secret: "test-secret".into(),
        passphrase: "test-pass".into(),
    }
}

fn limit_intent() -> OrderIntent {
    OrderIntent {
        id: "internal-1".into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: "bitget".into(),
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

fn cancel_request() -> CancelOrderRequest {
    CancelOrderRequest {
        exchange: "bitget".into(),
        symbol: "BTC".into(),
        internal_order_id: "internal-1".into(),
        exchange_order_id: Some("bitget-1".into()),
        client_order_id: "client-1".into(),
    }
}
