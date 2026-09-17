#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! Binance 适配器集成测试（wiremock）。
//!
//! 覆盖：
//! - 批量资金费率（含 24h 成交量合并）
//! - 单合约资金费率
//! - 批量行情过滤
//! - 订单簿解析
//! - 私有 GET balance（鉴权头）

use exchange::{
    Binance, BinanceConfig, BinanceCredentials, ExchangeAdapter, ExchangeError, LiveTradingAdapter,
};
use serde_json::json;
use shared_types::{
    CancelOrderRequest, ExecutionMode, IndexCompositionQuality, LiveOrderState, OrderIntent,
    OrderSide, OrderSource, OrderType,
};
use std::sync::OnceLock;
use tokio::sync::{Mutex, MutexGuard};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

struct QueryParamPresent(&'static str);

impl Match for QueryParamPresent {
    fn matches(&self, request: &Request) -> bool {
        request.url.query_pairs().any(|(key, _)| key == self.0)
    }
}

struct NoQueryParams;

impl Match for NoQueryParams {
    fn matches(&self, request: &Request) -> bool {
        request.url.query().is_none()
    }
}

fn binance(server_uri: String, credentials: Option<BinanceCredentials>) -> Binance {
    Binance::new(BinanceConfig {
        credentials,
        testnet: false,
        allow_live_writes: false,
        timeout_secs: 5,
        qps: 50,
        base_url_override: Some(server_uri),
    })
    .unwrap()
}

fn test_lock() -> &'static Mutex<()> {
    TEST_LOCK.get_or_init(|| Mutex::new(()))
}

async fn locked_server() -> (MutexGuard<'static, ()>, MockServer) {
    let guard = test_lock().lock().await;
    let server = MockServer::start().await;
    (guard, server)
}

async fn mock_funding_info_empty(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/fapi/v1/fundingInfo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(server)
        .await;
}

#[tokio::test]
async fn batch_funding_rates_join_volume() {
    let (_guard, server) = locked_server().await;
    mock_funding_info_empty(&server).await;

    let premium_body = json!([
        {
            "symbol": "BTCUSDT",
            "markPrice": "30000",
            "lastFundingRate": "0.0001",
            "nextFundingTime": 1_700_000_000_000_i64,
            "time": 1_699_999_999_000_i64,
        },
        {
            "symbol": "ETHUSDT",
            "markPrice": "2000",
            "lastFundingRate": "-0.00005",
            "nextFundingTime": 1_700_000_000_000_i64,
            "time": 1_699_999_999_000_i64,
        },
        // USDC 永续必须能被显式请求，但不能与同 base 的 USDT 合约混入默认发现结果。
        {
            "symbol": "BTCUSDC",
            "lastFundingRate": "0.0002",
            "nextFundingTime": 1_700_000_000_000_i64,
            "time": 1_699_999_999_000_i64,
        }
    ]);

    let ticker_body = json!([
        {
            "symbol": "BTCUSDT",
            "lastPrice": "30000",
            "bidPrice": "29999",
            "askPrice": "30001",
            "quoteVolume": "1500000000",
            "closeTime": 1_699_999_999_000_i64,
        },
        {
            "symbol": "ETHUSDT",
            "lastPrice": "2000",
            "bidPrice": "1999",
            "askPrice": "2001",
            "quoteVolume": "500000000",
            "closeTime": 1_699_999_999_000_i64,
        }
    ]);

    Mock::given(method("GET"))
        .and(path("/fapi/v1/premiumIndex"))
        .respond_with(ResponseTemplate::new(200).set_body_json(premium_body))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/fapi/v1/ticker/24hr"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ticker_body))
        .mount(&server)
        .await;

    let bx = binance(server.uri(), None);
    let rates = bx.get_funding_rates(None).await.expect("should fetch");

    assert_eq!(rates.len(), 2, "default discovery is quote-safe USDT only");

    let btc_usdt = rates
        .iter()
        .find(|r| r.symbol == "BTC" && (r.rate - 0.0001).abs() < 1e-12)
        .expect("BTCUSDT must be present");
    assert!((btc_usdt.volume_24h - 1_500_000_000.0).abs() < 1.0);
    assert_eq!(btc_usdt.exchange, "binance");
    assert_eq!(btc_usdt.funding_interval, 8);

    assert!(
        rates.iter().all(|row| (row.rate - 0.0002).abs() > 1e-12),
        "USDC must not be mixed into quote-less discovery"
    );

    let eth = rates.iter().find(|r| r.symbol == "ETH").unwrap();
    assert!((eth.rate - (-0.00005)).abs() < 1e-12);
    assert!((eth.volume_24h - 500_000_000.0).abs() < 1.0);

    let explicit = bx
        .get_funding_rates(Some(&["BTCUSDC".to_owned()]))
        .await
        .expect("explicit USDC request should fetch");
    assert_eq!(explicit.len(), 1);
    assert!((explicit[0].rate - 0.0002).abs() < 1e-12);
    assert!(explicit[0].volume_24h < 1.0);
}

#[tokio::test]
async fn single_funding_rate_query_param() {
    let (_guard, server) = locked_server().await;
    mock_funding_info_empty(&server).await;

    Mock::given(method("GET"))
        .and(path("/fapi/v1/premiumIndex"))
        .and(query_param("symbol", "BTCUSDT"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "symbol": "BTCUSDT",
            "markPrice": "30000",
            "lastFundingRate": "0.0003",
            "nextFundingTime": 1_700_000_000_000_i64,
            "time": 1_699_999_999_000_i64,
        })))
        .mount(&server)
        .await;

    let bx = binance(server.uri(), None);
    let rate = bx.get_funding_rate("BTC").await.expect("should fetch");
    assert_eq!(rate.symbol, "BTC");
    assert!((rate.rate - 0.0003).abs() < 1e-12);
}

#[tokio::test]
async fn funding_info_http_failure_blocks_default_interval_fallback() {
    let (_guard, server) = locked_server().await;

    Mock::given(method("GET"))
        .and(path("/fapi/v1/fundingInfo"))
        .respond_with(ResponseTemplate::new(429).set_body_string("rate limit"))
        .mount(&server)
        .await;

    let bx = binance(server.uri(), None);
    let err = bx
        .get_funding_rates(None)
        .await
        .expect_err("fundingInfo outage must not fall back to default 8h");

    assert!(matches!(err, ExchangeError::RateLimited { .. }));
}

#[tokio::test]
async fn tickers_filter_to_usdt() {
    let (_guard, server) = locked_server().await;

    Mock::given(method("GET"))
        .and(path("/fapi/v1/ticker/24hr"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "symbol": "BTCUSDT",
                "lastPrice": "30000",
                "bidPrice": "29999",
                "askPrice": "30001",
                "quoteVolume": "100",
                "closeTime": 1_700_000_000_000_i64,
            },
            {
                "symbol": "BTCBUSD",
                "lastPrice": "30000",
                "bidPrice": "29999",
                "askPrice": "30001",
                "quoteVolume": "100",
                "closeTime": 1_700_000_000_000_i64,
            }
        ])))
        .mount(&server)
        .await;

    // `get_tickers(None)` joins ticker/24hr with ticker/bookTicker; the
    // adapter does `tokio::try_join!` so missing the book endpoint produces
    // a parse error on the implicit 404 body.
    Mock::given(method("GET"))
        .and(path("/fapi/v1/ticker/bookTicker"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "symbol": "BTCUSDT",
                "bidPrice": "29999",
                "askPrice": "30001",
                "time": 1_700_000_000_000_i64,
            },
            {
                "symbol": "BTCBUSD",
                "bidPrice": "29999",
                "askPrice": "30001",
                "time": 1_700_000_000_000_i64,
            }
        ])))
        .mount(&server)
        .await;

    let bx = binance(server.uri(), None);
    let tickers = bx.get_tickers(None).await.expect("ok");
    assert_eq!(tickers.len(), 1);
    assert_eq!(tickers[0].symbol, "BTC");
}

#[tokio::test]
async fn orderbook_parses_official_fixture_levels() {
    let (_guard, server) = locked_server().await;
    let fixture = include_str!("../fixtures/binance/usdm_order_book_depth_btcusdt.json");
    let body: serde_json::Value = serde_json::from_str(fixture).expect("depth fixture json");

    Mock::given(method("GET"))
        .and(path("/fapi/v1/depth"))
        .and(query_param("symbol", "BTCUSDT"))
        .and(query_param("limit", "20"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let bx = binance(server.uri(), None);
    let ob = bx.get_orderbook("BTC", 20).await.expect("ok");
    assert_eq!(ob.symbol, "BTC");
    assert_eq!(ob.exchange, "binance");
    assert_eq!(ob.bids.len(), 2);
    assert_eq!(ob.asks.len(), 2);
    assert!((ob.bids[0][0] - 30000.1).abs() < 1e-9);
    assert!((ob.bids[0][1] - 0.25).abs() < 1e-9);
    assert!((ob.asks[0][0] - 30000.2).abs() < 1e-9);
    assert!((ob.asks[0][1] - 0.4).abs() < 1e-9);
}

#[tokio::test]
async fn index_composition_constituents_follow_official_endpoint() {
    let (_guard, server) = locked_server().await;

    Mock::given(method("GET"))
        .and(path("/fapi/v1/constituents"))
        .and(query_param("symbol", "BTCUSDT"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "symbol": "BTCUSDT",
            "time": 1_745_401_553_408_i64,
            "constituents": [
                {
                    "exchange": "binance",
                    "symbol": "BTCUSDT",
                    "price": "94057.03000000",
                    "weight": "0.51282051"
                },
                {
                    "exchange": "coinbase",
                    "symbol": "BTC-USDT",
                    "price": "-1",
                    "weight": "0.15384615"
                }
            ]
        })))
        .mount(&server)
        .await;

    let bx = binance(server.uri(), None);
    let snapshot = bx
        .get_index_composition("BTC")
        .await
        .expect("index composition");

    assert_eq!(snapshot.venue, "binance");
    assert_eq!(snapshot.symbol, "BTC");
    assert_eq!(snapshot.index_id, "BTCUSDT");
    assert_eq!(snapshot.quality, IndexCompositionQuality::Verified);
    assert_eq!(snapshot.components.len(), 2);
    assert_eq!(snapshot.components[0].name, "binance");
    assert_eq!(snapshot.components[0].symbol, "BTCUSDT");
    assert!((snapshot.components[0].weight - 0.51282051).abs() < 1e-10);
    assert_eq!(snapshot.components[1].price, None);
}

#[tokio::test]
async fn balance_signed_request_with_api_key_header() {
    let (_guard, server) = locked_server().await;

    Mock::given(method("GET"))
        .and(path("/fapi/v3/balance"))
        .and(header("X-MBX-APIKEY", "test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "asset": "USDT",
                "balance": "1000.5",
                "availableBalance": "950.0",
                "crossUnPnl": "10.0",
            },
            {
                "asset": "BTC",
                "balance": "0.05",
                "availableBalance": "0.05",
                "crossUnPnl": "0",
            }
        ])))
        .mount(&server)
        .await;

    let bx = binance(
        server.uri(),
        Some(BinanceCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
        }),
    );
    let balances = bx.get_balance(None).await.expect("ok");
    assert_eq!(balances.len(), 2);
    let usdt = balances.get("USDT").unwrap();
    assert!((usdt.total - 1000.5).abs() < 1e-9);
    assert!((usdt.available - 950.0).abs() < 1e-9);
    assert!((usdt.frozen - 50.5).abs() < 1e-6);
}

#[tokio::test]
async fn balance_filter_by_currency() {
    let (_guard, server) = locked_server().await;

    Mock::given(method("GET"))
        .and(path("/fapi/v3/balance"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"asset": "USDT", "balance": "100", "availableBalance": "100", "crossUnPnl": "0"},
            {"asset": "BTC", "balance": "0.1", "availableBalance": "0.1", "crossUnPnl": "0"}
        ])))
        .mount(&server)
        .await;

    let bx = binance(
        server.uri(),
        Some(BinanceCredentials {
            api_key: "k".into(),
            api_secret: "s".into(),
        }),
    );
    let balances = bx.get_balance(Some("BTC")).await.expect("ok");
    assert_eq!(balances.len(), 1);
    assert!(balances.contains_key("BTC"));
}

#[tokio::test]
async fn balance_bad_numeric_returns_parse_error() {
    let (_guard, server) = locked_server().await;

    Mock::given(method("GET"))
        .and(path("/fapi/v3/balance"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"asset": "USDT", "balance": "bad", "availableBalance": "100", "crossUnPnl": "0"}
        ])))
        .mount(&server)
        .await;

    let bx = binance(
        server.uri(),
        Some(BinanceCredentials {
            api_key: "k".into(),
            api_secret: "s".into(),
        }),
    );

    let err = bx
        .get_balance(Some("USDT"))
        .await
        .expect_err("bad balance numeric must not become zero");

    assert!(matches!(err, ExchangeError::Parse(message) if message.contains("balance")));
}

#[tokio::test]
async fn live_positions_queries_signed_position_risk() {
    let (_guard, server) = locked_server().await;
    let body: serde_json::Value = serde_json::from_str(include_str!(
        "../fixtures/binance/usdm_position_risk_btcusdt.json"
    ))
    .expect("position risk fixture");

    Mock::given(method("GET"))
        .and(path("/fapi/v3/positionRisk"))
        .and(header("X-MBX-APIKEY", "test-key"))
        .and(QueryParamPresent("timestamp"))
        .and(QueryParamPresent("signature"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let bx = binance(
        server.uri(),
        Some(BinanceCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
        }),
    );
    let positions = ExchangeAdapter::get_positions(&bx, Some("BTC"))
        .await
        .expect("positions");

    assert_eq!(positions.len(), 2);
    assert!(positions.iter().any(|p| p.side == "long"));
    assert!(positions.iter().any(|p| p.side == "short"));
}

#[tokio::test]
async fn positions_bad_nonzero_numeric_returns_parse_error() {
    let (_guard, server) = locked_server().await;

    Mock::given(method("GET"))
        .and(path("/fapi/v3/positionRisk"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "symbol": "BTCUSDT",
                "positionAmt": "0.5",
                "entryPrice": "bad",
                "markPrice": "110",
                "unRealizedProfit": "5",
                "liquidationPrice": "50",
                "notional": "55",
                "positionInitialMargin": "27.5",
                "maintMargin": "1.375",
                "positionSide": "LONG"
            }
        ])))
        .mount(&server)
        .await;

    let bx = binance(
        server.uri(),
        Some(BinanceCredentials {
            api_key: "k".into(),
            api_secret: "s".into(),
        }),
    );

    let err = ExchangeAdapter::get_positions(&bx, Some("BTC"))
        .await
        .expect_err("bad position numeric must not become zero");

    assert!(matches!(err, ExchangeError::Parse(message) if message.contains("entryPrice")));
}

#[tokio::test]
async fn missing_credentials_returns_auth_error() {
    let (_guard, server) = locked_server().await;
    let bx = binance(server.uri(), None);
    let err = bx.get_balance(None).await.expect_err("should require auth");
    assert!(matches!(err, exchange::ExchangeError::Auth(_)));
}

#[tokio::test]
async fn user_data_stream_start_uses_api_key_header_without_signature() {
    let (_guard, server) = locked_server().await;

    Mock::given(method("POST"))
        .and(path("/fapi/v1/listenKey"))
        .and(header("X-MBX-APIKEY", "test-key"))
        .and(NoQueryParams)
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "listenKey": "listen-key-1"
        })))
        .mount(&server)
        .await;

    let bx = binance(
        server.uri(),
        Some(BinanceCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
        }),
    );

    let listen_key = bx.start_user_data_stream().await.expect("listenKey");

    assert_eq!(listen_key, "listen-key-1");
}

#[tokio::test]
async fn user_data_stream_keepalive_uses_put_without_query_params() {
    let (_guard, server) = locked_server().await;

    Mock::given(method("PUT"))
        .and(path("/fapi/v1/listenKey"))
        .and(header("X-MBX-APIKEY", "test-key"))
        .and(NoQueryParams)
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "listenKey": "listen-key-1"
        })))
        .mount(&server)
        .await;

    let bx = binance(
        server.uri(),
        Some(BinanceCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
        }),
    );

    let listen_key = bx.keepalive_user_data_stream().await.expect("keepalive");

    assert_eq!(listen_key, "listen-key-1");
}

#[tokio::test]
async fn user_data_stream_close_uses_delete_without_query_params() {
    let (_guard, server) = locked_server().await;

    Mock::given(method("DELETE"))
        .and(path("/fapi/v1/listenKey"))
        .and(header("X-MBX-APIKEY", "test-key"))
        .and(NoQueryParams)
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    let bx = binance(
        server.uri(),
        Some(BinanceCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
        }),
    );

    bx.close_user_data_stream().await.expect("close");
}

fn live_intent(id: &str) -> OrderIntent {
    OrderIntent {
        id: id.into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Testnet,
        exchange: "binance".into(),
        symbol: "BTC".into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 0.01,
        price: Some(50_000.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: shared_types::TimeInForce::Ioc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: format!("client-{id}"),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

async fn mock_live_exchange_info(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/fapi/v1/exchangeInfo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "symbols": [
                {
                    "symbol": "BTCUSDT",
                    "baseAsset": "BTC",
                    "quoteAsset": "USDT",
                    "marginAsset": "USDT",
                    "status": "TRADING",
                    "contractType": "PERPETUAL",
                    "orderTypes": ["LIMIT", "MARKET"],
                    "timeInForce": ["GTC", "IOC", "FOK", "GTX"],
                    "filters": [
                        {
                            "filterType": "PRICE_FILTER",
                            "minPrice": "0.10",
                            "maxPrice": "1000000",
                            "tickSize": "0.10"
                        },
                        {
                            "filterType": "LOT_SIZE",
                            "minQty": "0.001",
                            "maxQty": "1000",
                            "stepSize": "0.001"
                        },
                        {
                            "filterType": "MARKET_LOT_SIZE",
                            "minQty": "0.001",
                            "maxQty": "100",
                            "stepSize": "0.001"
                        },
                        {
                            "filterType": "MIN_NOTIONAL",
                            "notional": "50"
                        }
                    ]
                }
            ]
        })))
        .mount(server)
        .await;
}

async fn mock_dual_quote_exchange_info(server: &MockServer) {
    let body: serde_json::Value = serde_json::from_str(include_str!(
        "../fixtures/binance/usdm_exchange_info_usdt_usdc.json"
    ))
    .expect("dual quote exchangeInfo fixture");
    Mock::given(method("GET"))
        .and(path("/fapi/v1/exchangeInfo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn mock_position_mode(server: &MockServer, dual_side_position: bool) {
    Mock::given(method("GET"))
        .and(path("/fapi/v1/positionSide/dual"))
        .and(header("X-MBX-APIKEY", "test-key"))
        .and(query_param("recvWindow", "5000"))
        .and(QueryParamPresent("timestamp"))
        .and(QueryParamPresent("signature"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dualSidePosition": dual_side_position
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn exchange_account_mode_reads_official_position_side_dual_endpoint() {
    let (_guard, server) = locked_server().await;
    mock_position_mode(&server, true).await;
    let bx = binance(
        server.uri(),
        Some(BinanceCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
        }),
    );

    let mode = bx
        .get_exchange_account_mode("binance")
        .await
        .expect("account mode request")
        .expect("binance account mode exists");

    assert_eq!(mode.venue, "binance");
    assert_eq!(mode.mode, "hedge");
    assert!(mode.source.contains("/fapi/v1/positionSide/dual"));
    assert_eq!(mode.account_scope.as_deref(), Some("usds_m_futures"));
    assert!(mode.checked_at_ms > 0);
}

#[tokio::test]
async fn live_place_order_sends_signed_testnet_order() {
    let (_guard, server) = locked_server().await;
    mock_live_exchange_info(&server).await;
    mock_position_mode(&server, false).await;

    Mock::given(method("POST"))
        .and(path("/fapi/v1/order"))
        .and(header("X-MBX-APIKEY", "test-key"))
        .and(query_param("symbol", "BTCUSDT"))
        .and(query_param("side", "BUY"))
        .and(query_param("positionSide", "BOTH"))
        .and(query_param("type", "LIMIT"))
        .and(query_param("quantity", "0.01"))
        .and(query_param("price", "50000"))
        .and(query_param("timeInForce", "IOC"))
        .and(query_param("newClientOrderId", "client-live1"))
        .and(QueryParamPresent("timestamp"))
        .and(QueryParamPresent("signature"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "orderId": 12345_u64,
            "symbol": "BTCUSDT",
            "status": "NEW",
            "type": "LIMIT",
            "side": "BUY",
            "price": "50000",
            "origQty": "0.01",
            "executedQty": "0",
            "avgPrice": "0",
            "time": 1_700_000_000_000_i64,
            "timeInForce": "IOC",
            "clientOrderId": "client-live1",
            "reduceOnly": false
        })))
        .mount(&server)
        .await;

    let bx = binance(
        server.uri(),
        Some(BinanceCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
        }),
    );

    let ack = bx.place_order(&live_intent("live1")).await.expect("ack");

    assert_eq!(ack.internal_order_id, "live1");
    assert_eq!(ack.exchange_order_id.as_deref(), Some("12345"));
    assert_eq!(ack.client_order_id, "client-live1");
    assert_eq!(ack.state, LiveOrderState::Accepted);
}

#[tokio::test]
async fn live_place_order_preserves_verified_usdc_native_symbol() {
    let (_guard, server) = locked_server().await;
    mock_dual_quote_exchange_info(&server).await;
    mock_position_mode(&server, false).await;

    Mock::given(method("POST"))
        .and(path("/fapi/v1/order"))
        .and(query_param("symbol", "BTCUSDC"))
        .and(query_param("positionSide", "BOTH"))
        .and(query_param("quantity", "0.01"))
        .and(query_param("price", "50000"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "orderId": 22345_u64,
            "symbol": "BTCUSDC",
            "status": "NEW",
            "type": "LIMIT",
            "side": "BUY",
            "price": "50000",
            "origQty": "0.01",
            "executedQty": "0",
            "avgPrice": "0",
            "time": 1_700_000_000_000_i64,
            "timeInForce": "IOC",
            "clientOrderId": "client-live-usdc",
            "reduceOnly": false
        })))
        .mount(&server)
        .await;

    let bx = binance(
        server.uri(),
        Some(BinanceCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
        }),
    );
    let mut order = live_intent("live-usdc");
    order.symbol = "BTC-USDC-SWAP".into();

    let ack = bx.place_order(&order).await.expect("USDC order ack");

    assert_eq!(ack.exchange_order_id.as_deref(), Some("22345"));
}

#[tokio::test]
async fn live_place_order_rejects_explicit_quote_mismatch() {
    let (_guard, server) = locked_server().await;
    mock_live_exchange_info(&server).await;
    let bx = binance(
        server.uri(),
        Some(BinanceCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
        }),
    );
    let mut order = live_intent("live-quote-mismatch");
    order.symbol = "BTCUSDC".into();

    let err = bx
        .place_order(&order)
        .await
        .expect_err("explicit USDC must not fall back to USDT");

    assert!(matches!(err, ExchangeError::UnsupportedSymbol(symbol) if symbol == "BTCUSDC"));
}

#[tokio::test]
async fn live_place_order_rejects_below_min_notional_before_submit() {
    let (_guard, server) = locked_server().await;
    mock_live_exchange_info(&server).await;
    let bx = binance(
        server.uri(),
        Some(BinanceCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
        }),
    );
    let mut order = live_intent("live-min-notional");
    order.quantity = 0.001;
    order.price = Some(10_000.0);

    let err = bx.place_order(&order).await.expect_err("local validation");

    match err {
        exchange::ExchangeError::Api { code, message, .. } => {
            assert_eq!(code, "validation");
            assert!(message.contains("below minNotional"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn live_place_order_rejects_non_trading_symbol_before_submit() {
    let (_guard, server) = locked_server().await;
    Mock::given(method("GET"))
        .and(path("/fapi/v1/exchangeInfo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "symbols": [
                {
                    "symbol": "BTCUSDT",
                    "baseAsset": "BTC",
                    "quoteAsset": "USDT",
                    "marginAsset": "USDT",
                    "status": "BREAK",
                    "contractType": "PERPETUAL",
                    "orderTypes": ["LIMIT", "MARKET"],
                    "timeInForce": ["GTC", "IOC", "FOK", "GTX"],
                    "filters": []
                }
            ]
        })))
        .mount(&server)
        .await;
    let bx = binance(
        server.uri(),
        Some(BinanceCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
        }),
    );

    let err = bx
        .place_order(&live_intent("live-status"))
        .await
        .expect_err("local validation");

    match err {
        exchange::ExchangeError::Api { code, message, .. } => {
            assert_eq!(code, "validation");
            assert!(message.contains("not TRADING"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn live_place_order_rejects_asset_symbol_mismatch_before_submit() {
    let (_guard, server) = locked_server().await;
    Mock::given(method("GET"))
        .and(path("/fapi/v1/exchangeInfo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "symbols": [
                {
                    "symbol": "BTCUSDT",
                    "baseAsset": "ETH",
                    "quoteAsset": "USDT",
                    "marginAsset": "USDT",
                    "status": "TRADING",
                    "contractType": "PERPETUAL",
                    "orderTypes": ["LIMIT", "MARKET"],
                    "timeInForce": ["GTC", "IOC", "FOK", "GTX"],
                    "filters": []
                }
            ]
        })))
        .mount(&server)
        .await;
    let bx = binance(
        server.uri(),
        Some(BinanceCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
        }),
    );

    let err = bx
        .place_order(&live_intent("live-asset"))
        .await
        .expect_err("local validation");

    match err {
        exchange::ExchangeError::Api { code, message, .. } => {
            assert_eq!(code, "validation");
            assert!(message.contains("baseAsset/quoteAsset"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn live_place_order_rejects_quantity_step_before_submit() {
    let (_guard, server) = locked_server().await;
    mock_live_exchange_info(&server).await;
    let bx = binance(
        server.uri(),
        Some(BinanceCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
        }),
    );
    let mut order = live_intent("live-step");
    order.quantity = 0.0015;

    let err = bx.place_order(&order).await.expect_err("local validation");

    match err {
        exchange::ExchangeError::Api { code, message, .. } => {
            assert_eq!(code, "validation");
            assert!(message.contains("quantity"));
            assert!(message.contains("step"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn live_place_order_rejects_price_tick_before_submit() {
    let (_guard, server) = locked_server().await;
    mock_live_exchange_info(&server).await;
    let bx = binance(
        server.uri(),
        Some(BinanceCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
        }),
    );
    let mut order = live_intent("live-price-tick");
    order.price = Some(50_000.05);

    let err = bx.place_order(&order).await.expect_err("local validation");

    match err {
        exchange::ExchangeError::Api { code, message, .. } => {
            assert_eq!(code, "validation");
            assert!(message.contains("price"));
            assert!(message.contains("step"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn live_cancel_order_uses_orig_client_order_id() {
    let (_guard, server) = locked_server().await;
    mock_live_exchange_info(&server).await;

    Mock::given(method("DELETE"))
        .and(path("/fapi/v1/order"))
        .and(header("X-MBX-APIKEY", "test-key"))
        .and(query_param("symbol", "BTCUSDT"))
        .and(query_param("origClientOrderId", "client-live2"))
        .and(QueryParamPresent("timestamp"))
        .and(QueryParamPresent("signature"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "orderId": 12346_u64,
            "symbol": "BTCUSDT",
            "status": "CANCELED",
            "type": "LIMIT",
            "side": "BUY",
            "price": "50000",
            "origQty": "0.01",
            "executedQty": "0",
            "avgPrice": "0",
            "time": 1_700_000_000_000_i64,
            "timeInForce": "GTC",
            "clientOrderId": "client-live2",
            "reduceOnly": false
        })))
        .mount(&server)
        .await;

    let bx = binance(
        server.uri(),
        Some(BinanceCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
        }),
    );
    let request = CancelOrderRequest {
        exchange: "binance".into(),
        symbol: "BTC".into(),
        internal_order_id: "live2".into(),
        exchange_order_id: Some("12346".into()),
        client_order_id: "client-live2".into(),
    };

    let ack = bx.cancel_order(&request).await.expect("cancel ack");

    assert_eq!(ack.internal_order_id, "live2");
    assert_eq!(ack.exchange_order_id.as_deref(), Some("12346"));
    assert_eq!(ack.state, LiveOrderState::Cancelled);
}

#[tokio::test]
async fn live_get_order_returns_order_info() {
    let (_guard, server) = locked_server().await;
    mock_live_exchange_info(&server).await;

    Mock::given(method("GET"))
        .and(path("/fapi/v1/order"))
        .and(header("X-MBX-APIKEY", "test-key"))
        .and(query_param("symbol", "BTCUSDT"))
        .and(query_param("origClientOrderId", "client-live3"))
        .and(QueryParamPresent("timestamp"))
        .and(QueryParamPresent("signature"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "orderId": 12347_u64,
            "symbol": "BTCUSDT",
            "status": "PARTIALLY_FILLED",
            "type": "LIMIT",
            "side": "SELL",
            "price": "51000",
            "origQty": "0.02",
            "executedQty": "0.01",
            "avgPrice": "51010",
            "time": 1_700_000_000_000_i64,
            "timeInForce": "GTC",
            "clientOrderId": "client-live3",
            "reduceOnly": false
        })))
        .mount(&server)
        .await;

    let bx = binance(
        server.uri(),
        Some(BinanceCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
        }),
    );

    let order = bx
        .get_order("BTC", "client-live3")
        .await
        .expect("query")
        .expect("order");

    assert_eq!(order.order_id, "12347");
    assert_eq!(order.symbol, "BTC");
    assert_eq!(order.side, OrderSide::Sell);
    assert_eq!(order.status, shared_types::OrderStatus::PartiallyFilled);
    assert!((order.filled_quantity - 0.01).abs() < 1e-12);
}

#[tokio::test]
async fn live_open_orders_queries_signed_current_open_orders() {
    let (_guard, server) = locked_server().await;

    Mock::given(method("GET"))
        .and(path("/fapi/v1/openOrders"))
        .and(header("X-MBX-APIKEY", "test-key"))
        .and(query_param("symbol", "BTCUSDT"))
        .and(QueryParamPresent("timestamp"))
        .and(QueryParamPresent("signature"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "orderId": 1917641_u64,
                "symbol": "BTCUSDT",
                "status": "NEW",
                "type": "LIMIT",
                "side": "BUY",
                "price": "9300",
                "origQty": "0.40",
                "executedQty": "0.10",
                "avgPrice": "0",
                "time": 1_579_276_756_075_i64,
                "timeInForce": "GTC",
                "clientOrderId": "binance-cli-1",
                "reduceOnly": false
            }
        ])))
        .mount(&server)
        .await;

    let bx = binance(
        server.uri(),
        Some(BinanceCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
        }),
    );

    let orders = LiveTradingAdapter::get_open_orders(&bx, Some("BTC"))
        .await
        .expect("open orders");

    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].order_id, "1917641");
    assert_eq!(orders[0].client_order_id.as_deref(), Some("binance-cli-1"));
    assert_eq!(orders[0].status, shared_types::OrderStatus::Open);
}

#[tokio::test]
async fn live_get_order_bad_numeric_returns_parse_error() {
    let (_guard, server) = locked_server().await;
    mock_live_exchange_info(&server).await;

    Mock::given(method("GET"))
        .and(path("/fapi/v1/order"))
        .and(header("X-MBX-APIKEY", "test-key"))
        .and(query_param("symbol", "BTCUSDT"))
        .and(query_param("origClientOrderId", "client-live-bad"))
        .and(QueryParamPresent("timestamp"))
        .and(QueryParamPresent("signature"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "orderId": 12348_u64,
            "symbol": "BTCUSDT",
            "status": "NEW",
            "type": "LIMIT",
            "side": "BUY",
            "price": "bad",
            "origQty": "0.02",
            "executedQty": "0",
            "avgPrice": "0",
            "time": 1_700_000_000_000_i64,
            "timeInForce": "GTC",
            "clientOrderId": "client-live-bad",
            "reduceOnly": false
        })))
        .mount(&server)
        .await;

    let bx = binance(
        server.uri(),
        Some(BinanceCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
        }),
    );

    let err = bx
        .get_order("BTC", "client-live-bad")
        .await
        .expect_err("bad order numeric must not become zero");

    assert!(matches!(err, ExchangeError::Parse(message) if message.contains("price")));
}
