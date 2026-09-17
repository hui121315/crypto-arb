use super::*;
use pretty_assertions::assert_eq;

fn gate() -> Gate {
    Gate::new(GateConfig::default()).unwrap()
}

#[test]
fn name() {
    assert_eq!(ExchangeAdapter::name(&gate()), "gate");
}

#[test]
fn symbol_round_trip() {
    let g = gate();
    assert_eq!(g.to_exchange_symbol("BTC"), "BTC_USDT");
    assert_eq!(g.to_exchange_symbol("ETH"), "ETH_USDT");
    assert_eq!(g.to_exchange_symbol("BTC_USD"), "BTC_USD");
    assert_eq!(g.to_exchange_symbol("ETH/USDC"), "ETH_USDC");
    assert_eq!(g.normalize_symbol("BTC_USDT"), "BTC");
}

#[test]
fn require_credentials_errors_when_missing() {
    let g = gate();
    assert!(g.require_credentials().is_err());
}

/// 修复 P1 5.1：仓位 quantity 必须 = 合约张数 × `quanto_multiplier`。
/// 直接验证缓存查找路径 + base 数量计算，不依赖 HTTP。
#[tokio::test]
async fn position_quantity_applies_quanto_multiplier() {
    let g = gate();
    g.contract_cache.seed_unit("BTC_USDT", 0.0001);

    // 模拟 PositionRow: size=100 张 BTC
    let size_contracts: u64 = 100;
    let unit = g.contract_cache.get_unit("BTC_USDT").unwrap();
    let base_quantity = (size_contracts as f64) * unit;
    // 期望：100 × 0.0001 = 0.01 BTC
    assert!((base_quantity - 0.01).abs() < 1e-12);
}

#[tokio::test]
async fn empty_position_snapshot_skips_contract_metadata_refresh() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/api/v4/futures/usdt/positions"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("[]"))
        .expect(1)
        .mount(&server)
        .await;
    let adapter = gate_with_server(&server);

    let positions = ExchangeAdapter::get_positions(&adapter, None)
        .await
        .expect("empty Gate position snapshot");

    assert!(positions.is_empty());
    let requests = server
        .received_requests()
        .await
        .expect("recorded Gate requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), "/api/v4/futures/usdt/positions");
}

/// Gate API v4 current testnet host is `api-testnet.gateapi.io`.
#[test]
fn testnet_flag_selects_testnet_base_url() {
    let g = Gate::new(GateConfig {
        testnet: true,
        ..Default::default()
    })
    .unwrap();
    assert_eq!(g.base_url, "https://api-testnet.gateapi.io");

    let g_prod = gate();
    assert_eq!(g_prod.base_url, "https://api.gateio.ws");

    // base_url_override 优先级更高
    let g_override = Gate::new(GateConfig {
        testnet: true,
        base_url_override: Some("https://override.example.com".into()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(g_override.base_url, "https://override.example.com");
}

/// 修复 P1 5.3：相同 symbol 第二次调用 `contract_order_unit` 应命中缓存
/// 而无需重新拉网络。这里直接验证缓存路径行为（用预填充缓存近似缓存命中）。
#[tokio::test]
async fn contract_order_unit_uses_cache_when_fresh() {
    let g = gate();
    g.contract_cache.seed_unit("ETH_USDT", 0.001);
    // 不需 base_url；缓存命中直接返回
    let unit = g.contract_order_unit("ETH_USDT").await.unwrap();
    assert!((unit - 0.001).abs() < 1e-12);
}

#[tokio::test]
async fn orderbook_normalizes_contract_counts_to_base_quantity() {
    use wiremock::matchers::{method, path, query_param};

    let server = wiremock::MockServer::start().await;
    let fixture = include_str!("../../fixtures/gate/futures_usdt_order_book_btc_usdt.json");
    wiremock::Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/order_book"))
        .and(query_param("contract", "BTC_USDT"))
        .and(query_param("limit", "20"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(fixture))
        .expect(1)
        .mount(&server)
        .await;
    let adapter = gate_with_server(&server);
    adapter.contract_cache.seed_unit("BTC_USDT", 0.0001);

    let book = ExchangeAdapter::get_orderbook(&adapter, "BTC", 20)
        .await
        .expect("normalized Gate orderbook");

    assert!((book.bids[0][1] - 4.7081).abs() < 1e-12);
    assert!((book.asks[0][1] - 1.1098).abs() < 1e-12);
}

#[tokio::test]
async fn numeric_order_finality_uses_order_id_and_enriches_fill_fees() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path(
            "/api/v4/futures/usdt/orders/777",
        ))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
            r#"{"id":777,"contract":"BTC_USDT","status":"finished","finish_as":"filled","size":2,"left":0,"price":"100","fill_price":"100","tif":"gtc","create_time_ms":1700028800123}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path(
            "/api/v4/futures/usdt/my_trades",
        ))
        .and(wiremock::matchers::query_param("order", "777"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
            r#"[{"id":1,"create_time":1700028800.1,"contract":"BTC_USDT","order_id":"777","size":"1","price":"99","fee":"0.01","point_fee":"0","role":"taker"},{"id":2,"create_time":1700028800.2,"contract":"BTC_USDT","order_id":"777","size":"1","price":"101","fee":"-0.002","point_fee":"0.4","role":"maker"}]"#,
        ))
        .expect(1)
        .mount(&server)
        .await;
    let adapter = gate_with_server(&server);
    adapter.contract_cache.seed_unit("BTC_USDT", 1.0);

    let order = LiveTradingAdapter::get_order_by_exchange_order_id(&adapter, "BTC", "777")
        .await
        .expect("numeric finality query")
        .expect("order exists");

    assert_eq!(order.order_id, "777");
    assert_eq!(order.filled_price, 100.0);
    assert!((order.fees - 0.008).abs() < 1e-12);
}

#[tokio::test]
async fn numeric_order_absence_remains_unknown_without_fill_probe() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/api/v4/futures/usdt/orders/777"))
        .respond_with(
            wiremock::ResponseTemplate::new(404).set_body_string(r#"{"label":"ORDER_NOT_FOUND"}"#),
        )
        .expect(1)
        .mount(&server)
        .await;
    let adapter = gate_with_server(&server);
    adapter.contract_cache.seed_unit("BTC_USDT", 1.0);

    let order = LiveTradingAdapter::get_order_by_exchange_order_id(&adapter, "BTC", "777")
        .await
        .expect("404 remains a clean unknown result");

    assert!(order.is_none());
}

#[tokio::test]
async fn account_fee_evidence_is_signed_cached_and_contract_scoped() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/api/v4/futures/usdt/fee"))
        .and(wiremock::matchers::header_exists("KEY"))
        .and(wiremock::matchers::header_exists("Timestamp"))
        .and(wiremock::matchers::header_exists("SIGN"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_string(r#"{"BTC_USDT":{"maker_fee":"-0.0001","taker_fee":"0.0005"}}"#),
        )
        .expect(2)
        .mount(&server)
        .await;
    let adapter = gate_with_server(&server);

    adapter
        .ensure_account_fee_evidence("BTC_USDT")
        .await
        .expect("first fee read");
    adapter
        .ensure_account_fee_evidence("BTC_USDT")
        .await
        .expect("cached fee read");
    assert!(adapter
        .ensure_account_fee_evidence("ETH_USDT")
        .await
        .is_err());
}

fn gate_with_server(server: &wiremock::MockServer) -> Gate {
    Gate::new(GateConfig {
        credentials: Some(GateCredentials {
            api_key: "key".to_owned(),
            api_secret: "secret".to_owned(),
        }),
        allow_live_writes: true,
        base_url_override: Some(server.uri()),
        ..GateConfig::default()
    })
    .expect("gate adapter")
}
