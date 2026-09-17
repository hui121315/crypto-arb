#![allow(clippy::expect_used, clippy::unwrap_used)]

use exchange::{ExchangeAdapter, Hyperliquid, HyperliquidConfig, HyperliquidMarket};
use serde_json::{json, Value};
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn core_and_builder_metadata_requests_follow_official_info_shapes() {
    let core_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_json(json!({"type": "meta"})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(fixture("../fixtures/hyperliquid/meta.json")),
        )
        .expect(1)
        .mount(&core_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_json(json!({"type": "spotMeta"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            fixture("../fixtures/hyperliquid/spot_meta_and_asset_ctxs_purr_hfun.json")[0].clone(),
        ))
        .expect(1)
        .mount(&core_server)
        .await;
    let core = adapter(core_server.uri(), HyperliquidMarket::Core);
    let core_instruments = core.fetch_instruments().await.expect("core metadata");
    assert_eq!(core_instruments[0].venue, "hyperliquid");
    assert_eq!(core_instruments[0].native_symbol, "BTC");
    assert_eq!(core_instruments[0].builder_dex, None);
    assert!(core_instruments.iter().any(|row| {
        row.native_symbol == "@10000"
            && row.canonical_symbol == "PURR"
            && row.quote_asset.as_deref() == Some("USDC")
            && row.product_type.as_deref() == Some("spot")
    }));

    let builder_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_json(json!({"type": "meta", "dex": "xyz"})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(fixture("../fixtures/hyperliquid/meta_compiler_xyz.json")),
        )
        .expect(1)
        .mount(&builder_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_json(json!({"type": "perpDexs"})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(fixture("../fixtures/hyperliquid/perp_dexs_compiler.json")),
        )
        .expect(1)
        .mount(&builder_server)
        .await;
    let builder = adapter(builder_server.uri(), HyperliquidMarket::XYZ);
    let builder_instruments = builder.fetch_instruments().await.expect("builder metadata");
    assert_eq!(builder_instruments[0].venue, "hyperliquid:xyz");
    assert_eq!(builder_instruments[0].native_symbol, "xyz:CBRS");
    assert_eq!(builder_instruments[0].builder_dex.as_deref(), Some("xyz"));
}

fn adapter(server_uri: String, market: HyperliquidMarket) -> Hyperliquid {
    Hyperliquid::new(HyperliquidConfig {
        market,
        base_url_override: Some(server_uri),
        timeout_secs: 5,
        qps: 100,
        ..HyperliquidConfig::default()
    })
    .expect("adapter")
}

fn fixture(path: &str) -> Value {
    let contents = match path {
        "../fixtures/hyperliquid/meta.json" => include_str!("../fixtures/hyperliquid/meta.json"),
        "../fixtures/hyperliquid/meta_compiler_xyz.json" => {
            include_str!("../fixtures/hyperliquid/meta_compiler_xyz.json")
        }
        "../fixtures/hyperliquid/perp_dexs_compiler.json" => {
            include_str!("../fixtures/hyperliquid/perp_dexs_compiler.json")
        }
        "../fixtures/hyperliquid/spot_meta_and_asset_ctxs_purr_hfun.json" => {
            include_str!("../fixtures/hyperliquid/spot_meta_and_asset_ctxs_purr_hfun.json")
        }
        _ => unreachable!("known fixture"),
    };
    serde_json::from_str(contents).expect("fixture JSON")
}
