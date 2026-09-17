use super::*;
use crate::adapter::ExchangeAdapter;
use crate::adapters::hyperliquid_config::HyperliquidMarket;
use serde_json::{json, Value};
use shared_types::{
    ExecutionMode, InstrumentAssetClass, MarginMode, OrderIntent, OrderSide, OrderSource,
    OrderType, TimeInForce,
};
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn warmed_builder_compiler_uses_cached_official_metadata() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_json(json!({"type": "meta", "dex": "xyz"})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(fixture("../../fixtures/hyperliquid/meta_compiler_xyz.json")),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_json(json!({"type": "perpDexs"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture(
            "../../fixtures/hyperliquid/perp_dexs_compiler.json",
        )))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_json(json!({"type": "perpCategories"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture(
            "../../fixtures/hyperliquid/perp_categories_identity.json",
        )))
        .expect(1)
        .mount(&server)
        .await;

    let adapter = Hyperliquid::new(HyperliquidConfig {
        market: HyperliquidMarket::XYZ,
        base_url_override: Some(server.uri()),
        timeout_secs: 5,
        qps: 100,
        ..HyperliquidConfig::default()
    })
    .expect("builder adapter");
    let instruments = ExchangeAdapter::fetch_instruments(&adapter)
        .await
        .expect("warm metadata cache");
    assert_eq!(instruments[0].asset_class, InstrumentAssetClass::Equity);

    let action = adapter
        .compile_order_action(&builder_intent())
        .expect("cache-backed action");
    assert_eq!(action["orders"][0]["a"], 110_000);
    assert_eq!(action["orders"][0]["t"]["limit"]["tif"], "Ioc");
    assert_eq!(
        action["orders"][0]["c"],
        "0xffffffffffffffffffffffffffffffff"
    );
}

#[tokio::test]
async fn concurrent_builder_refreshes_share_global_directory_requests() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_json(json!({"type": "meta", "dex": "xyz"})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(fixture("../../fixtures/hyperliquid/meta_compiler_xyz.json")),
        )
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_json(json!({"type": "perpDexs"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture(
            "../../fixtures/hyperliquid/perp_dexs_compiler.json",
        )))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_json(json!({"type": "perpCategories"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture(
            "../../fixtures/hyperliquid/perp_categories_identity.json",
        )))
        .expect(1)
        .mount(&server)
        .await;

    let config = HyperliquidConfig {
        market: HyperliquidMarket::XYZ,
        base_url_override: Some(server.uri()),
        timeout_secs: 5,
        qps: 100,
        ..HyperliquidConfig::default()
    };
    let first = Hyperliquid::new(config.clone()).expect("first builder adapter");
    let second = Hyperliquid::new(config).expect("second builder adapter");

    let (first, second) = tokio::join!(
        ExchangeAdapter::fetch_instruments(&first),
        ExchangeAdapter::fetch_instruments(&second)
    );

    assert!(!first.expect("first refresh").is_empty());
    assert!(!second.expect("second refresh").is_empty());
}

#[tokio::test]
async fn malformed_or_unusable_refresh_retains_last_known_good_builder_cache() {
    let server = MockServer::start().await;
    mount_builder_metadata(
        &server,
        fixture("../../fixtures/hyperliquid/meta_compiler_xyz.json"),
    )
    .await;
    let adapter = Hyperliquid::new(HyperliquidConfig {
        market: HyperliquidMarket::XYZ,
        base_url_override: Some(server.uri()),
        timeout_secs: 5,
        qps: 100,
        ..HyperliquidConfig::default()
    })
    .expect("builder adapter");
    ExchangeAdapter::fetch_instruments(&adapter)
        .await
        .expect("warm valid metadata");

    for invalid_meta in [
        json!({"universe": []}),
        json!({"universe": [{"name": "xyz:CBRS"}]}),
    ] {
        server.reset().await;
        mount_builder_metadata(&server, invalid_meta).await;

        assert!(ExchangeAdapter::fetch_instruments(&adapter).await.is_err());
        let action = adapter
            .compile_order_action(&builder_intent())
            .expect("last known-good metadata remains usable");
        assert_eq!(action["orders"][0]["a"], 110_000);
    }
}

#[test]
fn cold_compiler_fails_before_any_write_transport() {
    let adapter = Hyperliquid::new(HyperliquidConfig {
        base_url_override: Some("http://hyperliquid-cache-cold.invalid".into()),
        ..HyperliquidConfig::default()
    })
    .expect("adapter");

    let error = adapter
        .compile_order_action(&builder_intent())
        .expect_err("cold metadata cache must block an order");

    assert!(error.to_string().contains("metadata cache is cold"));
}

fn fixture(path: &str) -> Value {
    let contents = match path {
        "../../fixtures/hyperliquid/meta_compiler_xyz.json" => {
            include_str!("../../fixtures/hyperliquid/meta_compiler_xyz.json")
        }
        "../../fixtures/hyperliquid/perp_dexs_compiler.json" => {
            include_str!("../../fixtures/hyperliquid/perp_dexs_compiler.json")
        }
        "../../fixtures/hyperliquid/perp_categories_identity.json" => {
            include_str!("../../fixtures/hyperliquid/perp_categories_identity.json")
        }
        _ => unreachable!("known fixture"),
    };
    serde_json::from_str(contents).expect("fixture JSON")
}

async fn mount_builder_metadata(server: &MockServer, meta: Value) {
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_json(json!({"type": "meta", "dex": "xyz"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(meta))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_json(json!({"type": "perpDexs"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture(
            "../../fixtures/hyperliquid/perp_dexs_compiler.json",
        )))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_json(json!({"type": "perpCategories"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture(
            "../../fixtures/hyperliquid/perp_categories_identity.json",
        )))
        .mount(server)
        .await;
}

fn builder_intent() -> OrderIntent {
    OrderIntent {
        id: "hyperliquid-builder-order".into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: "hyperliquid:xyz".into(),
        symbol: "CBRS".into(),
        side: OrderSide::Buy,
        order_type: OrderType::Market,
        quantity: 0.001,
        price: Some(123.45),
        slippage_tolerance_bps: Some(10.0),
        reduce_only: false,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        margin_mode: MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "0xffffffffffffffffffffffffffffffff".into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}
