use super::*;
use crate::adapters::hyperliquid_public_rest;
use pretty_assertions::assert_eq;
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn hl() -> Hyperliquid {
    Hyperliquid::new(HyperliquidConfig::default()).unwrap()
}

#[test]
fn name() {
    assert_eq!(ExchangeAdapter::name(&hl()), "hyperliquid");
}

#[test]
fn symbol_no_suffix() {
    let h = hl();
    assert_eq!(h.to_exchange_symbol("BTC"), "BTC");
    assert_eq!(h.normalize_symbol("BTC"), "BTC");
    // 已有 USDT 后缀的也能 idempotent 处理
    assert_eq!(h.normalize_symbol("BTCUSDT"), "BTC");
}

#[test]
fn require_user_errors_when_missing() {
    let h = hl();
    assert!(h.require_user().is_err());
}

#[test]
fn private_state_body_adds_builder_dex_only_for_builder_markets() {
    let core = hl();
    let core_body = core.private_state_body("clearinghouseState", USER);
    assert_eq!(core_body["type"], "clearinghouseState");
    assert_eq!(core_body["user"], USER);
    assert!(core_body.get("dex").is_none());

    let builder = Hyperliquid::new(HyperliquidConfig {
        market: HyperliquidMarket::XYZ,
        ..HyperliquidConfig::default()
    })
    .expect("builder config");
    let builder_body = builder.private_state_body("frontendOpenOrders", USER);
    assert_eq!(builder_body["type"], "frontendOpenOrders");
    assert_eq!(builder_body["user"], USER);
    assert_eq!(builder_body["dex"], "xyz");
    assert_eq!(
        builder.ws_perp_coins(&["SNDK".to_owned()]),
        vec!["xyz:SNDK"]
    );
}

#[test]
fn hyperliquid_meta_body_uses_official_body_with_optional_dex() {
    assert_eq!(hyperliquid_public_rest::INFO_PATH, "/info");

    let core = hl();
    let core_body = core.meta_body();
    assert_eq!(core_body["type"], "metaAndAssetCtxs");
    assert!(core_body.get("dex").is_none());

    let builder = Hyperliquid::new(HyperliquidConfig {
        market: HyperliquidMarket::XYZ,
        ..HyperliquidConfig::default()
    })
    .expect("builder config");
    let builder_body = builder.meta_body();
    assert_eq!(builder_body["type"], "metaAndAssetCtxs");
    assert_eq!(builder_body["dex"], "xyz");
}

#[tokio::test]
async fn builder_dex_does_not_duplicate_core_spot_feed() {
    let builder = Hyperliquid::new(HyperliquidConfig {
        market: HyperliquidMarket::XYZ,
        ..HyperliquidConfig::default()
    })
    .expect("builder config");

    let rows = builder
        .get_spot_tickers(None)
        .await
        .expect("builder spot fanout should be an empty successful result");
    assert!(rows.is_empty());
}

#[tokio::test]
async fn flat_account_positions_skip_high_weight_market_context() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({
            "type": "clearinghouseState",
            "user": USER
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "marginSummary": {
                "accountValue": "50.0",
                "totalNtlPos": "0.0",
                "totalRawUsd": "50.0",
                "totalMarginUsed": "0.0"
            },
            "crossMarginSummary": {
                "accountValue": "50.0",
                "totalNtlPos": "0.0",
                "totalRawUsd": "50.0",
                "totalMarginUsed": "0.0"
            },
            "crossMaintenanceMarginUsed": "0.0",
            "withdrawable": "50.0",
            "assetPositions": []
        })))
        .expect(1)
        .mount(&server)
        .await;
    let adapter = Hyperliquid::new(HyperliquidConfig {
        credentials: Some(HyperliquidCredentials {
            user_address: USER.to_owned(),
            private_key: None,
            vault_address: None,
        }),
        base_url_override: Some(server.uri()),
        ..HyperliquidConfig::default()
    })
    .expect("flat account adapter");

    let rows = ExchangeAdapter::get_positions(&adapter, None)
        .await
        .expect("flat account position read");

    assert!(rows.is_empty());
    assert_eq!(
        server.received_requests().await.unwrap_or_default().len(),
        1
    );
}

const USER: &str = "0x0000000000000000000000000000000000000000";
