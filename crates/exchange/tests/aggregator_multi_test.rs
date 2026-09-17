#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! Aggregator 多家集成测试。
//!
//! 验证 Binance + OKX + Bybit 三个真实适配器同时注册时：
//! 1. `fetch_all_funding_rates_report` 并发返回多家数据
//! 2. `fetch_all_tickers_report` 同上
//! 3. 单家 5xx 失败时其他两家照常返回（部分失败容忍）
//! 4. Symbol 归一化一致：BTC/ETH 在三家都标准化为基础币种

use exchange::{
    Aggregator, Binance, BinanceConfig, Bitget, BitgetConfig, Bybit, BybitConfig, Gate, GateConfig,
    Hyperliquid, HyperliquidConfig, Kucoin, KucoinConfig, Okx, OkxConfig,
};
use serde_json::json;
use std::collections::HashSet;
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn test_lock() -> &'static Mutex<()> {
    TEST_LOCK.get_or_init(|| Mutex::new(()))
}

/// 启动一个 mock Binance 服务，返回 [BTC, ETH] 永续 funding rate + tickers。
async fn mock_binance(server: &MockServer) {
    let premium = json!([
        {
            "symbol": "BTCUSDT",
            "markPrice": "30000",
            "lastFundingRate": "0.0001",
            "nextFundingTime": 1_700_028_800_000_i64,
            "time": 1_700_000_000_000_i64,
        },
        {
            "symbol": "ETHUSDT",
            "lastFundingRate": "0.00005",
            "nextFundingTime": 1_700_028_800_000_i64,
            "time": 1_700_000_000_000_i64,
        }
    ]);
    let ticker = json!([
        {"symbol": "BTCUSDT", "lastPrice": "30000", "bidPrice": "29999", "askPrice": "30001", "quoteVolume": "1500000000", "closeTime": 1_700_000_000_000_i64},
        {"symbol": "ETHUSDT", "lastPrice": "2000", "bidPrice": "1999", "askPrice": "2001", "quoteVolume": "500000000", "closeTime": 1_700_000_000_000_i64}
    ]);

    // Binance USD-M `get_tickers(None)` joins ticker/24hr with ticker/bookTicker
    // (`public_rest::tickers_and_books`); mock both so the join does not collapse
    // on a 404 body during the aggregator full scan.
    let book = json!([
        {"symbol": "BTCUSDT", "bidPrice": "29999", "askPrice": "30001", "time": 1_700_000_000_000_i64},
        {"symbol": "ETHUSDT", "bidPrice": "1999", "askPrice": "2001", "time": 1_700_000_000_000_i64}
    ]);

    Mock::given(method("GET"))
        .and(path("/fapi/v1/fundingInfo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/fapi/v1/premiumIndex"))
        .respond_with(ResponseTemplate::new(200).set_body_json(premium))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/fapi/v1/ticker/24hr"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ticker))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/fapi/v1/ticker/bookTicker"))
        .respond_with(ResponseTemplate::new(200).set_body_json(book))
        .mount(server)
        .await;
}

async fn mock_okx(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/v5/public/instruments"))
        .and(query_param("instType", "SWAP"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "data": [
                {"instId": "BTC-USDT-SWAP"},
                {"instId": "ETH-USDT-SWAP"}
            ]
        })))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/v5/market/tickers"))
        .and(query_param("instType", "SWAP"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "data": [
                {"instId": "BTC-USDT-SWAP", "last": "30000", "bidPx": "29999", "askPx": "30001", "volCcy24h": "1200000000", "ts": "1700000000000"},
                {"instId": "ETH-USDT-SWAP", "last": "2000", "bidPx": "1999", "askPx": "2001", "volCcy24h": "400000000", "ts": "1700000000000"}
            ]
        })))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/v5/public/funding-rate"))
        .and(query_param("instId", "BTC-USDT-SWAP"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "data": [{"instId": "BTC-USDT-SWAP", "fundingRate": "0.00012", "fundingTime": "1700000000000", "nextFundingTime": "1700028800000"}]
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v5/public/funding-rate"))
        .and(query_param("instId", "ETH-USDT-SWAP"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "data": [{"instId": "ETH-USDT-SWAP", "fundingRate": "0.00006", "fundingTime": "1700000000000", "nextFundingTime": "1700028800000"}]
        })))
        .mount(server)
        .await;
}

async fn mock_bybit(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v5/market/tickers"))
        .and(query_param("category", "linear"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "retCode": 0,
            "result": {"list": [
                {"symbol": "BTCUSDT", "lastPrice": "30000", "bid1Price": "29999", "ask1Price": "30001", "turnover24h": "1300000000", "fundingRate": "0.00009", "nextFundingTime": "1700028800000"},
                {"symbol": "ETHUSDT", "lastPrice": "2000", "bid1Price": "1999", "ask1Price": "2001", "turnover24h": "450000000", "fundingRate": "0.00004", "nextFundingTime": "1700028800000"}
            ]}
        })))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/v5/market/instruments-info"))
        .and(query_param("category", "linear"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "retCode": 0,
            "result": {"list": [
                {"symbol": "BTCUSDT", "fundingInterval": "480", "settleCoin": "USDT"},
                {"symbol": "ETHUSDT", "fundingInterval": "480", "settleCoin": "USDT"}
            ]}
        })))
        .mount(server)
        .await;
}

fn make_aggregator(binance_uri: String, okx_uri: String, bybit_uri: String) -> Aggregator {
    let agg = Aggregator::new();
    agg.register(Arc::new(
        Binance::new(BinanceConfig {
            base_url_override: Some(binance_uri),
            timeout_secs: 5,
            qps: 100,
            ..Default::default()
        })
        .unwrap(),
    ));
    agg.register(Arc::new(
        Okx::new(OkxConfig {
            base_url_override: Some(okx_uri),
            timeout_secs: 5,
            qps: 100,
            ..Default::default()
        })
        .unwrap(),
    ));
    agg.register(Arc::new(
        Bybit::new(BybitConfig {
            base_url_override: Some(bybit_uri),
            timeout_secs: 5,
            qps: 100,
            ..Default::default()
        })
        .unwrap(),
    ));
    agg
}

#[tokio::test]
async fn aggregator_fetches_funding_rates_from_all_three() {
    let _guard = test_lock().lock().await;
    let binance_srv = MockServer::start().await;
    let okx_srv = MockServer::start().await;
    let bybit_srv = MockServer::start().await;

    mock_binance(&binance_srv).await;
    mock_okx(&okx_srv).await;
    mock_bybit(&bybit_srv).await;

    let agg = make_aggregator(binance_srv.uri(), okx_srv.uri(), bybit_srv.uri());
    assert_eq!(agg.len(), 3);
    assert_eq!(agg.names(), vec!["binance", "bybit", "okx"]);

    let rates = agg.fetch_all_funding_rates_report().await.rows;

    // 3 家 × 2 币种 = 6 条
    assert_eq!(rates.len(), 6, "expected 6 rates, got {}", rates.len());

    // 验证每家都返回了
    let exchanges: HashSet<&str> = rates.iter().map(|r| r.exchange.as_str()).collect();
    assert!(exchanges.contains("binance"));
    assert!(exchanges.contains("okx"));
    assert!(exchanges.contains("bybit"));

    // 验证 symbol 归一化一致：所有家的 BTC/ETH 都标记为基础币种
    let symbols: HashSet<&str> = rates.iter().map(|r| r.symbol.as_str()).collect();
    assert_eq!(symbols.len(), 2, "expected exactly BTC + ETH normalized");
    assert!(symbols.contains("BTC"));
    assert!(symbols.contains("ETH"));
}

#[tokio::test]
async fn aggregator_fetches_tickers_from_all_three() {
    let _guard = test_lock().lock().await;
    let binance_srv = MockServer::start().await;
    let okx_srv = MockServer::start().await;
    let bybit_srv = MockServer::start().await;

    mock_binance(&binance_srv).await;
    mock_okx(&okx_srv).await;
    mock_bybit(&bybit_srv).await;

    let agg = make_aggregator(binance_srv.uri(), okx_srv.uri(), bybit_srv.uri());
    let tickers = agg.fetch_all_tickers_report().await.rows;

    assert_eq!(
        tickers.len(),
        6,
        "expected 3 exchanges × 2 symbols = 6 tickers"
    );
    let exchanges: HashSet<&str> = tickers.iter().map(|t| t.exchange.as_str()).collect();
    assert_eq!(exchanges.len(), 3);
}

#[tokio::test]
async fn aggregator_tolerates_one_exchange_failure() {
    let _guard = test_lock().lock().await;
    let binance_srv = MockServer::start().await;
    let okx_srv = MockServer::start().await; // 不挂载任何 mock → 所有请求 404
    let bybit_srv = MockServer::start().await;

    mock_binance(&binance_srv).await;
    // 故意不调用 mock_okx，让 OKX 全部 404 → 返回空
    mock_bybit(&bybit_srv).await;

    let agg = make_aggregator(binance_srv.uri(), okx_srv.uri(), bybit_srv.uri());
    let report = agg.fetch_all_funding_rates_report().await;
    let rates = report.rows;

    // OKX 失败被吞，剩余两家共 4 条
    assert_eq!(
        rates.len(),
        4,
        "OKX failure should leave 4 rates from binance + bybit"
    );
    let exchanges: HashSet<&str> = rates.iter().map(|r| r.exchange.as_str()).collect();
    assert!(exchanges.contains("binance"));
    assert!(exchanges.contains("bybit"));
    assert!(!exchanges.contains("okx"), "OKX should be missing");
    let okx_outcome = report
        .venues
        .iter()
        .find(|outcome| outcome.venue == "okx")
        .expect("okx outcome");
    assert_eq!(okx_outcome.rows, 0);
    assert!(okx_outcome.problem.is_some());
}

#[tokio::test]
async fn aggregator_get_individual_adapter() {
    let _guard = test_lock().lock().await;
    let binance_srv = MockServer::start().await;
    let okx_srv = MockServer::start().await;
    let bybit_srv = MockServer::start().await;

    let agg = make_aggregator(binance_srv.uri(), okx_srv.uri(), bybit_srv.uri());
    let bn = agg.get("binance").expect("binance present");
    assert_eq!(bn.name(), "binance");
    let unknown = agg.get("ftx");
    assert!(unknown.is_none());
}

// ============================================================
// 扩展：全量 7 家适配器并发集成测试
// ============================================================

async fn mock_bitget(server: &MockServer) {
    // Bitget V3 / UTA endpoints (see `bitget_uta_public_rest::funding_rates_and_tickers`).
    // REST query value is upper-case `USDT-FUTURES`; payload key names follow
    // the V3 ticker schema (`lastPrice` / `bid1Price` / `ask1Price` / `turnover24h`).
    Mock::given(method("GET"))
        .and(path("/api/v3/market/instruments"))
        .and(query_param("category", "USDT-FUTURES"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "00000",
            "data": [
                {"symbol": "BTCUSDT", "fundInterval": "8"},
                {"symbol": "ETHUSDT", "fundInterval": "8"}
            ]
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v3/market/tickers"))
        .and(query_param("category", "USDT-FUTURES"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "00000",
            "data": [
                {"symbol": "BTCUSDT", "lastPrice": "30000", "bid1Price": "29999", "ask1Price": "30001", "volume24h": "30000", "turnover24h": "1100000000", "fundingRate": "0.00011", "nextFundingTime": "1700028800000", "ts": "1700000000000"},
                {"symbol": "ETHUSDT", "lastPrice": "2000", "bid1Price": "1999", "ask1Price": "2001", "volume24h": "100000", "turnover24h": "300000000", "fundingRate": "0.00006", "nextFundingTime": "1700028800000", "ts": "1700000000000"}
            ]
        })))
        .mount(server)
        .await;
}

async fn mock_gate(server: &MockServer) {
    let contract = |name: &str, funding_rate: &str| {
        json!({
            "name": name,
            "type": "direct",
            "quanto_multiplier": "0.0001",
            "funding_rate": funding_rate,
            "funding_next_apply": 1_700_028_800_i64,
            "funding_interval": 28_800,
            "in_delisting": false,
            "status": "trading",
            "order_price_round": "0.1",
            "order_size_min": 1,
            "order_size_max": 1_000_000,
            "market_order_size_max": 100_000,
            "enable_decimal": false,
            "leverage_min": "1",
            "leverage_max": "100",
            "maintenance_rate": "0.005",
            "maker_fee_rate": "-0.0001",
            "taker_fee_rate": "0.0005"
        })
    };
    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/contracts"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            contract("BTC_USDT", "0.00009"),
            contract("ETH_USDT", "0.00004")
        ])))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/tickers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"contract": "BTC_USDT", "last": "30000", "highest_bid": "29999", "lowest_ask": "30001", "funding_rate": "0.00009", "volume_24h_quote": "1200000000", "volume_24h_settle": "1200000000"},
            {"contract": "ETH_USDT", "last": "2000", "highest_bid": "1999", "lowest_ask": "2001", "funding_rate": "0.00004", "volume_24h_quote": "350000000", "volume_24h_settle": "350000000"}
        ])))
        .mount(server)
        .await;
}

async fn mock_kucoin(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/v1/contracts/active"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "200000",
            "data": [
                {"symbol": "XBTUSDTM", "fundingFeeRate": 0.00012, "fundingRateGranularity": 28800000, "nextFundingRateDateTime": 1700028800000_i64, "lastTradePrice": 30000.0, "turnoverOf24h": 950000000.0},
                {"symbol": "ETHUSDTM", "fundingFeeRate": 0.00007, "fundingRateGranularity": 28800000, "nextFundingRateDateTime": 1700028800000_i64, "lastTradePrice": 2000.0, "turnoverOf24h": 280000000.0}
            ]
        })))
        .mount(server)
        .await;
}

async fn mock_hyperliquid(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({"type": "metaAndAssetCtxs"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"universe": [
                {"name": "BTC", "szDecimals": 5, "maxLeverage": 50},
                {"name": "ETH", "szDecimals": 4, "maxLeverage": 50}
            ]},
            [
                {"funding": "0.0000125", "markPx": "30000", "midPx": "30000", "dayNtlVlm": "850000000", "impactPxs": ["29999", "30001"]},
                {"funding": "0.000005", "markPx": "2000", "midPx": "2000", "dayNtlVlm": "260000000", "impactPxs": ["1999", "2001"]}
            ]
        ])))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({"type": "predictedFundings"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            ["BTC", [["HlPerp", {"fundingRate": "0.0000125", "nextFundingTime": 1700028800000_i64, "fundingIntervalHours": 1}]]],
            ["ETH", [["HlPerp", {"fundingRate": "0.000005", "nextFundingTime": 1700028800000_i64, "fundingIntervalHours": 1}]]]
        ])))
        .mount(server)
        .await;
}

#[tokio::test]
async fn aggregator_fetches_funding_rates_from_all_seven() {
    let _guard = test_lock().lock().await;
    // 启动 7 个独立 mock 服务
    let bin_srv = MockServer::start().await;
    let okx_srv = MockServer::start().await;
    let by_srv = MockServer::start().await;
    let bg_srv = MockServer::start().await;
    let gt_srv = MockServer::start().await;
    let ku_srv = MockServer::start().await;
    let hl_srv = MockServer::start().await;

    mock_binance(&bin_srv).await;
    mock_okx(&okx_srv).await;
    mock_bybit(&by_srv).await;
    mock_bitget(&bg_srv).await;
    mock_gate(&gt_srv).await;
    mock_kucoin(&ku_srv).await;
    mock_hyperliquid(&hl_srv).await;

    // 注册 7 家
    let agg = Aggregator::new();
    agg.register(Arc::new(
        Binance::new(BinanceConfig {
            base_url_override: Some(bin_srv.uri()),
            timeout_secs: 5,
            qps: 100,
            ..Default::default()
        })
        .unwrap(),
    ));
    agg.register(Arc::new(
        Okx::new(OkxConfig {
            base_url_override: Some(okx_srv.uri()),
            timeout_secs: 5,
            qps: 100,
            ..Default::default()
        })
        .unwrap(),
    ));
    agg.register(Arc::new(
        Bybit::new(BybitConfig {
            base_url_override: Some(by_srv.uri()),
            timeout_secs: 5,
            qps: 100,
            ..Default::default()
        })
        .unwrap(),
    ));
    agg.register(Arc::new(
        Bitget::new(BitgetConfig {
            base_url_override: Some(bg_srv.uri()),
            timeout_secs: 5,
            qps: 100,
            ..Default::default()
        })
        .unwrap(),
    ));
    agg.register(Arc::new(
        Gate::new(GateConfig {
            base_url_override: Some(gt_srv.uri()),
            timeout_secs: 5,
            qps: 100,
            ..Default::default()
        })
        .unwrap(),
    ));
    agg.register(Arc::new(
        Kucoin::new(KucoinConfig {
            base_url_override: Some(ku_srv.uri()),
            timeout_secs: 5,
            qps: 100,
            ..Default::default()
        })
        .unwrap(),
    ));
    agg.register(Arc::new(
        Hyperliquid::new(HyperliquidConfig {
            base_url_override: Some(hl_srv.uri()),
            timeout_secs: 5,
            qps: 100,
            ..Default::default()
        })
        .unwrap(),
    ));
    assert_eq!(agg.len(), 7);
    let names = agg.names();
    assert_eq!(
        names,
        vec![
            "binance",
            "bitget",
            "bybit",
            "gate",
            "hyperliquid",
            "kucoin",
            "okx"
        ]
    );

    let report = agg.fetch_all_funding_rates_report().await;
    let venue_debug = report
        .venues
        .iter()
        .map(|outcome| {
            (
                outcome.venue.clone(),
                outcome.rows,
                outcome
                    .problem
                    .as_ref()
                    .map(|problem| problem.message.clone()),
            )
        })
        .collect::<Vec<_>>();
    let rates = report.rows;

    // 7 家 × 2 币种（BTC + ETH）= 14 条
    assert_eq!(
        rates.len(),
        14,
        "expected 7 exchanges × 2 symbols = 14 rates, got {}; venues={venue_debug:?}",
        rates.len(),
    );

    // 验证 7 家全部出现
    let exchanges: HashSet<&str> = rates.iter().map(|r| r.exchange.as_str()).collect();
    assert_eq!(exchanges.len(), 7);
    for name in [
        "binance",
        "okx",
        "bybit",
        "bitget",
        "gate",
        "kucoin",
        "hyperliquid",
    ] {
        assert!(exchanges.contains(name), "missing exchange: {name}");
    }

    // 验证 symbol 归一化跨 7 家一致：原生 symbol 格式都归为 {BTC, ETH}
    let symbols: HashSet<&str> = rates.iter().map(|r| r.symbol.as_str()).collect();
    assert_eq!(symbols.len(), 2, "should normalize to exactly BTC + ETH");
    assert!(symbols.contains("BTC"));
    assert!(symbols.contains("ETH"));

    // 验证每家 BTC funding rate 不全为零（验证数据真实流通）
    let btc_rates: Vec<f64> = rates
        .iter()
        .filter(|r| r.symbol == "BTC")
        .map(|r| r.rate)
        .collect();
    assert_eq!(btc_rates.len(), 8);
    let nonzero_count = btc_rates.iter().filter(|r| **r != 0.0).count();
    assert_eq!(
        nonzero_count, 8,
        "all 7 exchanges should report non-zero BTC funding rate"
    );
}

#[tokio::test]
async fn aggregator_seven_with_partial_failure() {
    let _guard = test_lock().lock().await;
    // 7 家中 OKX 未挂载 mock（全部 404），其余 6 家正常
    let bin_srv = MockServer::start().await;
    let okx_srv = MockServer::start().await; // empty
    let by_srv = MockServer::start().await;
    let bg_srv = MockServer::start().await;
    let gt_srv = MockServer::start().await;
    let ku_srv = MockServer::start().await;
    let hl_srv = MockServer::start().await;

    mock_binance(&bin_srv).await;
    mock_bybit(&by_srv).await;
    mock_bitget(&bg_srv).await;
    mock_gate(&gt_srv).await;
    mock_kucoin(&ku_srv).await;
    mock_hyperliquid(&hl_srv).await;

    let agg = Aggregator::new();
    let configs: Vec<(&str, Arc<dyn exchange::ExchangeAdapter>)> = vec![
        (
            "binance",
            Arc::new(
                Binance::new(BinanceConfig {
                    base_url_override: Some(bin_srv.uri()),
                    timeout_secs: 3,
                    qps: 100,
                    ..Default::default()
                })
                .unwrap(),
            ),
        ),
        (
            "okx",
            Arc::new(
                Okx::new(OkxConfig {
                    base_url_override: Some(okx_srv.uri()),
                    timeout_secs: 3,
                    qps: 100,
                    ..Default::default()
                })
                .unwrap(),
            ),
        ),
        (
            "bybit",
            Arc::new(
                Bybit::new(BybitConfig {
                    base_url_override: Some(by_srv.uri()),
                    timeout_secs: 3,
                    qps: 100,
                    ..Default::default()
                })
                .unwrap(),
            ),
        ),
        (
            "bitget",
            Arc::new(
                Bitget::new(BitgetConfig {
                    base_url_override: Some(bg_srv.uri()),
                    timeout_secs: 3,
                    qps: 100,
                    ..Default::default()
                })
                .unwrap(),
            ),
        ),
        (
            "gate",
            Arc::new(
                Gate::new(GateConfig {
                    base_url_override: Some(gt_srv.uri()),
                    timeout_secs: 3,
                    qps: 100,
                    ..Default::default()
                })
                .unwrap(),
            ),
        ),
        (
            "kucoin",
            Arc::new(
                Kucoin::new(KucoinConfig {
                    base_url_override: Some(ku_srv.uri()),
                    timeout_secs: 3,
                    qps: 100,
                    ..Default::default()
                })
                .unwrap(),
            ),
        ),
        (
            "hyperliquid",
            Arc::new(
                Hyperliquid::new(HyperliquidConfig {
                    base_url_override: Some(hl_srv.uri()),
                    timeout_secs: 3,
                    qps: 100,
                    ..Default::default()
                })
                .unwrap(),
            ),
        ),
    ];
    for (_, ad) in configs {
        agg.register(ad);
    }

    let report = agg.fetch_all_funding_rates_report().await;
    let rates = report.rows;
    // 6 家正常 × 2 币种 = 12
    assert_eq!(rates.len(), 12, "OKX failure should leave 12 rates");

    let exchanges: HashSet<&str> = rates.iter().map(|r| r.exchange.as_str()).collect();
    assert!(exchanges.contains("binance"));
    assert!(exchanges.contains("bybit"));
    assert!(exchanges.contains("bitget"));
    assert!(exchanges.contains("gate"));
    assert!(exchanges.contains("kucoin"));
    assert!(exchanges.contains("hyperliquid"));
    assert!(!exchanges.contains("okx"));
    let failed: HashSet<&str> = report
        .venues
        .iter()
        .filter(|outcome| outcome.problem.is_some())
        .map(|outcome| outcome.venue.as_str())
        .collect();
    assert_eq!(failed, HashSet::from(["okx"]));
}
