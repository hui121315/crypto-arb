#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! 真网响应诊断（一次性 V1.1 工具）：用真实 fixture 测试每家适配器的反序列化路径。
//!
//! 诊断 fixture 必须入库，避免 CI 因本机 `/tmp` 缺少文件而静默跳过解析路径。

use exchange::ExchangeAdapter;

// ============== Binance ==============

#[tokio::test]
async fn diagnose_binance_real_response() {
    let body = include_str!("../fixtures/binance/spot_ticker_24hr_full.json");

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v3/ticker/24hr"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let b = exchange::Binance::new(exchange::BinanceConfig {
        base_url_override: Some(server.uri()),
        timeout_secs: 5,
        qps: 100,
        ..Default::default()
    })
    .unwrap();

    match b.get_spot_tickers(None).await {
        Ok(ticks) => assert!(!ticks.is_empty(), "Binance fixture parsed no spot ticks"),
        Err(e) => panic!("❌ Binance parse failed: {e}"),
    }
}

// ============== OKX ==============

#[tokio::test]
async fn diagnose_okx_real_response() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../fixtures/okx/market_tickers_swap_spot_btc_eth_usdt.json"
    ))
    .expect("okx market tickers fixture");
    let spot_body = fixture
        .get("spot")
        .cloned()
        .unwrap_or_else(|| panic!("okx market tickers fixture missing spot section"));

    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v5/market/tickers"))
        .and(query_param("instType", "SPOT"))
        .respond_with(ResponseTemplate::new(200).set_body_json(spot_body))
        .mount(&server)
        .await;

    let o = exchange::Okx::new(exchange::OkxConfig {
        base_url_override: Some(server.uri()),
        timeout_secs: 5,
        qps: 100,
        ..Default::default()
    })
    .unwrap();

    match o.get_spot_tickers(None).await {
        Ok(ticks) => assert!(!ticks.is_empty(), "OKX fixture parsed no spot ticks"),
        Err(e) => panic!("❌ OKX parse failed: {e}"),
    }
}

// ============== KuCoin ==============

#[tokio::test]
async fn diagnose_kucoin_real_response() {
    let body = include_str!("../fixtures/kucoin/contracts_active_xbt_eth_usdtm.json");
    // 直接用 wiremock 模拟返回这个响应
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/contracts/active"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let k = exchange::Kucoin::new(exchange::KucoinConfig {
        base_url_override: Some(server.uri()),
        timeout_secs: 5,
        qps: 100,
        ..Default::default()
    })
    .unwrap();

    match k.get_funding_rates(None).await {
        Ok(rates) => assert!(!rates.is_empty(), "KuCoin fixture parsed no rates"),
        Err(e) => panic!("❌ KuCoin parse failed: {e}"),
    }
}

// ============== Bybit ==============

#[tokio::test]
async fn diagnose_bybit_real_response() {
    let tickers_fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../fixtures/bybit/market_tickers_linear_spot_btcusdt.json"
    ))
    .expect("bybit market tickers fixture");
    let tickers_body = tickers_fixture
        .get("linear")
        .cloned()
        .unwrap_or_else(|| panic!("bybit market tickers fixture missing linear section"));
    let info_body = include_str!("../fixtures/bybit/instruments_info_linear_btcusdt.json");

    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v5/market/tickers"))
        .and(query_param("category", "linear"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tickers_body))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v5/market/instruments-info"))
        .and(query_param("category", "linear"))
        .respond_with(ResponseTemplate::new(200).set_body_string(info_body))
        .mount(&server)
        .await;

    let b = exchange::Bybit::new(exchange::BybitConfig {
        base_url_override: Some(server.uri()),
        timeout_secs: 5,
        qps: 100,
        ..Default::default()
    })
    .unwrap();

    match b.get_funding_rates(None).await {
        Ok(rates) => assert!(!rates.is_empty(), "Bybit fixture parsed no rates"),
        Err(e) => panic!("❌ Bybit parse failed: {e}"),
    }
}

// ============== Bitget ==============

#[tokio::test]
async fn diagnose_bitget_real_response() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../fixtures/bitget/uta_tickers_usdt_futures_spot_btcusdt.json"
    ))
    .expect("bitget UTA tickers fixture");
    let spot_body = fixture
        .get("spot")
        .cloned()
        .unwrap_or_else(|| panic!("bitget UTA tickers fixture missing spot section"));

    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v3/market/tickers"))
        .and(query_param("category", "SPOT"))
        .respond_with(ResponseTemplate::new(200).set_body_json(spot_body))
        .mount(&server)
        .await;

    let b = exchange::Bitget::new(exchange::BitgetConfig {
        base_url_override: Some(server.uri()),
        timeout_secs: 5,
        qps: 100,
        ..Default::default()
    })
    .unwrap();

    match b.get_spot_tickers(None).await {
        Ok(ticks) => assert!(!ticks.is_empty(), "Bitget fixture parsed no spot ticks"),
        Err(e) => panic!("❌ Bitget parse failed: {e}"),
    }
}

// ============== Gate ==============

#[tokio::test]
async fn diagnose_gate_real_response() {
    let body = include_str!("../fixtures/gate/spot_tickers_btc_eth_usdt.json");

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/spot/tickers"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let g = exchange::Gate::new(exchange::GateConfig {
        base_url_override: Some(server.uri()),
        timeout_secs: 5,
        qps: 100,
        ..Default::default()
    })
    .unwrap();

    match g.get_spot_tickers(None).await {
        Ok(ticks) => assert!(!ticks.is_empty(), "Gate fixture parsed no spot ticks"),
        Err(e) => panic!("❌ Gate parse failed: {e}"),
    }
}

// ============== Hyperliquid ==============

#[tokio::test]
async fn diagnose_hyperliquid_real_response() {
    let body = include_str!("../fixtures/hyperliquid/spot_meta_and_asset_ctxs_purr_hfun.json");
    let payload: serde_json::Value =
        serde_json::from_str(body).expect("hyperliquid spot meta/asset ctxs fixture");

    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(
            serde_json::json!({"type": "spotMetaAndAssetCtxs"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(payload))
        .mount(&server)
        .await;

    let h = exchange::Hyperliquid::new(exchange::HyperliquidConfig {
        base_url_override: Some(server.uri()),
        market: exchange::HyperliquidMarket::Core,
        timeout_secs: 5,
        qps: 100,
        ..Default::default()
    })
    .unwrap();

    match h.get_spot_tickers(None).await {
        Ok(ticks) => assert!(
            !ticks.is_empty(),
            "Hyperliquid fixture parsed no spot ticks"
        ),
        Err(e) => panic!("❌ Hyperliquid parse failed: {e}"),
    }
}
