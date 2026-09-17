#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! Hyperliquid 适配器集成测试（wiremock）。

use exchange::{
    ExchangeAdapter, Hyperliquid, HyperliquidConfig, HyperliquidCredentials, LiveTradingAdapter,
};
use serde_json::{json, Value};
use shared_types::IndexCompositionQuality;
use wiremock::matchers::{body_partial_json, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn hl(server_uri: String, credentials: Option<HyperliquidCredentials>) -> Hyperliquid {
    Hyperliquid::new(HyperliquidConfig {
        credentials,
        market: exchange::HyperliquidMarket::Core,
        allow_live_writes: false,
        timeout_secs: 5,
        qps: 100,
        base_url_override: Some(server_uri),
        action_expires_after_ms: None,
    })
    .unwrap()
}

fn hl_with_writes(server_uri: String, credentials: Option<HyperliquidCredentials>) -> Hyperliquid {
    Hyperliquid::new(HyperliquidConfig {
        credentials,
        market: exchange::HyperliquidMarket::Core,
        allow_live_writes: true,
        timeout_secs: 5,
        qps: 100,
        base_url_override: Some(server_uri),
        action_expires_after_ms: None,
    })
    .unwrap()
}

#[tokio::test]
async fn batch_funding_rates_normalize_1h_to_8h() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({"type": "metaAndAssetCtxs"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(meta_asset_ctxs_fixture()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({"type": "predictedFundings"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(predicted_fundings_fixture()))
        .mount(&server)
        .await;

    let h = hl(server.uri(), None);
    let rates = h.get_funding_rates(None).await.expect("ok");
    assert_eq!(rates.len(), 2);

    let btc = rates.iter().find(|r| r.symbol == "BTC").unwrap();
    assert_eq!(btc.exchange, "hyperliquid");
    assert_eq!(btc.funding_interval, 1); // 1h base
                                         // 1h 0.00125% → 8h 标准化 0.01%
    assert!((btc.rate - 0.0000125).abs() < 1e-15);
    assert!((btc.rate_8h - 0.0001).abs() < 1e-12);
    assert!((btc.volume_24h - 1_169_046.294_06).abs() < 1e-9);

    let eth = rates.iter().find(|r| r.symbol == "ETH").unwrap();
    assert!((eth.rate_8h - 0.0001).abs() < 1e-12);
}

#[tokio::test]
async fn get_orderbook_l2book() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({"type": "l2Book", "coin": "BTC"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(l2book_fixture()))
        .mount(&server)
        .await;

    let h = hl(server.uri(), None);
    let ob = h.get_orderbook("BTC", 20).await.expect("ok");
    assert_eq!(ob.symbol, "BTC");
    assert_eq!(ob.exchange, "hyperliquid");
    assert_eq!(ob.bids.len(), 2);
    assert_eq!(ob.asks.len(), 1);
    assert!((ob.bids[0][0] - 113_377.0).abs() < 1e-9);
    assert!((ob.asks[0][0] - 113_397.0).abs() < 1e-9);
    assert_eq!(ob.timestamp, 1_754_450_974_231);
}

#[tokio::test]
async fn hyperliquid_get_tickers_requests_meta_and_asset_ctxs() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({"type": "metaAndAssetCtxs"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(meta_asset_ctxs_fixture()))
        .mount(&server)
        .await;

    let h = hl(server.uri(), None);
    let tickers = h.get_tickers(None).await.expect("tickers");
    let btc = tickers.iter().find(|row| row.symbol == "BTC").unwrap();

    assert_eq!(btc.exchange, "hyperliquid");
    assert_eq!(btc.bid, 14.3047);
    assert_eq!(btc.ask, 14.3444);
}

#[tokio::test]
async fn hyperliquid_get_spot_tickers_requests_spot_meta_and_asset_ctxs() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({"type": "spotMetaAndAssetCtxs"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(spot_meta_asset_ctxs_fixture()))
        .expect(1)
        .mount(&server)
        .await;

    let h = hl(server.uri(), None);
    let ticks = h.get_spot_tickers(None).await.expect("spot tickers");
    let purr = ticks
        .iter()
        .find(|row| row.symbol == "PURR/USDC")
        .expect("PURR/USDC tick");

    assert_eq!(purr.venue, "hyperliquid");
    assert_eq!(purr.last.to_string(), "0.21");
}

#[tokio::test]
async fn hyperliquid_get_funding_rates_requests_meta_and_predicted_fundings() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({"type": "metaAndAssetCtxs"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(meta_asset_ctxs_fixture()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({"type": "predictedFundings"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(predicted_fundings_fixture()))
        .mount(&server)
        .await;

    let h = hl(server.uri(), None);
    let rates = h.get_funding_rates(None).await.expect("funding rates");

    assert!(rates.iter().any(|row| row.symbol == "BTC"));
}

#[tokio::test]
async fn builder_dex_orderbook_uses_prefixed_coin() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(
            json!({"type": "l2Book", "coin": "xyz:CBRS"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "coin": "xyz:CBRS",
            "levels": [
                [{"px": "315.7", "sz": "6.33", "n": 1}],
                [{"px": "315.97", "sz": "30.0", "n": 1}]
            ],
            "time": 1_779_275_975_723_i64
        })))
        .mount(&server)
        .await;

    let h = Hyperliquid::new(HyperliquidConfig {
        market: exchange::HyperliquidMarket::XYZ,
        base_url_override: Some(server.uri()),
        timeout_secs: 5,
        qps: 100,
        ..Default::default()
    })
    .expect("builder dex adapter");
    let ob = h.get_orderbook("CBRS", 20).await.expect("orderbook");

    assert_eq!(ExchangeAdapter::name(&h), "hyperliquid:xyz");
    assert_eq!(h.to_exchange_symbol("CBRS"), "xyz:CBRS");
    assert_eq!(ob.exchange, "hyperliquid:xyz");
    assert_eq!(ob.symbol, "CBRS");
    assert_eq!(ob.bids[0], [315.7, 6.33]);
    assert_eq!(ob.asks[0], [315.97, 30.0]);
}

#[tokio::test]
async fn index_composition_is_unverified_without_fake_components() {
    let server = MockServer::start().await;
    let h = Hyperliquid::new(HyperliquidConfig {
        market: exchange::HyperliquidMarket::XYZ,
        base_url_override: Some(server.uri()),
        timeout_secs: 5,
        qps: 100,
        ..Default::default()
    })
    .expect("builder dex adapter");
    let snapshot = h
        .get_index_composition("CBRS")
        .await
        .expect("unverified snapshot");

    assert_eq!(snapshot.venue, "hyperliquid:xyz");
    assert_eq!(snapshot.symbol, "CBRS");
    assert_eq!(snapshot.index_id, "xyz:CBRS");
    assert_eq!(snapshot.quality, IndexCompositionQuality::Unverified);
    assert!(snapshot.components.is_empty());
    assert!(snapshot.error.unwrap().contains("not a verified"));
}

#[tokio::test]
async fn balance_uses_user_address_no_signature() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({
            "type": "clearinghouseState",
            "user": "0xABCDEF"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "marginSummary": {
                "accountValue": "10000",
                "totalNtlPos": "5000",
                "totalRawUsd": "10000",
                "totalMarginUsed": "1000"
            },
            "crossMarginSummary": {
                "accountValue": "10000",
                "totalNtlPos": "5000",
                "totalRawUsd": "10000",
                "totalMarginUsed": "1000"
            },
            "crossMaintenanceMarginUsed": "0",
            "withdrawable": "9000",
            "assetPositions": []
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({"type": "metaAndAssetCtxs"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(meta_asset_ctxs_fixture()))
        .mount(&server)
        .await;

    let h = hl(
        server.uri(),
        Some(HyperliquidCredentials {
            user_address: "0xABCDEF".into(),
            private_key: None,
            vault_address: None,
        }),
    );
    let balances = h.get_balance(None).await.expect("ok");
    let usdc = balances.get("USDC").unwrap();
    assert!((usdc.total - 10000.0).abs() < 1e-9);
    assert!((usdc.available - 9000.0).abs() < 1e-9);
    assert!((usdc.frozen - 1000.0).abs() < 1e-9);
}

#[tokio::test]
async fn account_role_validation_rejects_agent_address_for_reads() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({
            "type": "userRole",
            "user": ACCOUNT
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "role": "agent",
            "data": {"user": "0x2222222222222222222222222222222222222222"}
        })))
        .mount(&server)
        .await;

    let h = hl(
        server.uri(),
        Some(HyperliquidCredentials {
            user_address: ACCOUNT.into(),
            private_key: None,
            vault_address: None,
        }),
    );
    let error = h
        .validate_account_role_status()
        .await
        .expect_err("agent address must fail account role validation");

    assert!(matches!(error, exchange::ExchangeError::Auth(_)));
    assert!(error.to_string().contains("agent wallet"));
}

#[tokio::test]
async fn agent_approval_validation_accepts_agent_owned_by_account() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({
            "type": "userRole",
            "user": AGENT_FROM_PRIVATE_KEY_ONE
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "role": "agent",
            "data": {"user": ACCOUNT}
        })))
        .mount(&server)
        .await;

    let h = hl(
        server.uri(),
        Some(HyperliquidCredentials {
            user_address: ACCOUNT.into(),
            private_key: Some(PRIVATE_KEY_ONE.into()),
            vault_address: None,
        }),
    );

    assert!(h
        .validate_agent_approval_status()
        .await
        .expect("agent approval probe"));
}

#[tokio::test]
async fn agent_approval_validation_fails_closed_on_mismatched_owner() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({
            "type": "userRole",
            "user": AGENT_FROM_PRIVATE_KEY_ONE
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "role": "agent",
            "data": {"user": "0x2222222222222222222222222222222222222222"}
        })))
        .mount(&server)
        .await;

    let h = hl(
        server.uri(),
        Some(HyperliquidCredentials {
            user_address: ACCOUNT.into(),
            private_key: Some(PRIVATE_KEY_ONE.into()),
            vault_address: None,
        }),
    );

    assert!(!h
        .validate_agent_approval_status()
        .await
        .expect("agent approval probe"));
}

#[tokio::test]
async fn agent_approval_validation_fails_closed_when_signer_is_not_agent() {
    for role in ["missing", "user", "vault", "subAccount"] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/info"))
            .and(body_partial_json(json!({
                "type": "userRole",
                "user": AGENT_FROM_PRIVATE_KEY_ONE
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "role": role
            })))
            .mount(&server)
            .await;

        let h = hl(
            server.uri(),
            Some(HyperliquidCredentials {
                user_address: ACCOUNT.into(),
                private_key: Some(PRIVATE_KEY_ONE.into()),
                vault_address: None,
            }),
        );

        assert!(
            !h.validate_agent_approval_status()
                .await
                .expect("agent approval probe"),
            "role={role} must not be accepted as an approved agent"
        );
    }
}

#[tokio::test]
async fn credential_relation_reads_main_agent_and_vault_facts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({
            "type": "userRole",
            "user": ACCOUNT
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"role": "user"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({
            "type": "userRole",
            "user": AGENT_FROM_PRIVATE_KEY_ONE
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "role": "agent",
            "data": {"user": ACCOUNT}
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({
            "type": "userRole",
            "user": VAULT
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"role": "vault"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({
            "type": "vaultDetails",
            "vaultAddress": VAULT,
            "user": ACCOUNT
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "vaultAddress": VAULT,
            "leader": ACCOUNT
        })))
        .mount(&server)
        .await;

    let h = hl(
        server.uri(),
        Some(HyperliquidCredentials {
            user_address: ACCOUNT.into(),
            private_key: Some(PRIVATE_KEY_ONE.into()),
            vault_address: Some(VAULT.into()),
        }),
    );
    let relation = h.credential_relation().await.expect("credential relation");

    assert_eq!(relation.main_account_role, "user");
    assert_eq!(relation.main_account_owner, None);
    assert_eq!(relation.signer_role, "agent");
    assert_eq!(relation.signer_owner.as_deref(), Some(ACCOUNT));
    assert_eq!(relation.vault_role.as_deref(), Some("vault"));
    assert_eq!(relation.vault_leader.as_deref(), Some(ACCOUNT));
}

#[tokio::test]
async fn account_abstraction_reads_official_account_and_dex_states() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({
            "type": "userAbstraction",
            "user": ACCOUNT
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json("default"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({
            "type": "userDexAbstraction",
            "user": ACCOUNT
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!(false)))
        .expect(1)
        .mount(&server)
        .await;
    let h = hl(
        server.uri(),
        Some(HyperliquidCredentials {
            user_address: ACCOUNT.into(),
            private_key: Some(PRIVATE_KEY_ONE.into()),
            vault_address: None,
        }),
    );

    let state = h
        .account_abstraction_state()
        .await
        .expect("account abstraction state");

    assert_eq!(state.account_address, ACCOUNT);
    assert_eq!(state.user_abstraction, "default");
    assert_eq!(state.user_dex_abstraction.as_deref(), Some("disabled"));
}

#[tokio::test]
async fn safe_noop_permission_probe_uses_official_exchange_endpoint_without_live_writes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/exchange"))
        .and(header_exists("Content-Type"))
        .and(body_partial_json(json!({"action": {"type": "noop"}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "ok"
        })))
        .mount(&server)
        .await;

    let h = hl(
        server.uri(),
        Some(HyperliquidCredentials {
            user_address: ACCOUNT.into(),
            private_key: Some(PRIVATE_KEY_ONE.into()),
            vault_address: None,
        }),
    );

    h.validate_safe_noop_permission()
        .await
        .expect("official noop is a save-time signed action probe");
}

#[tokio::test]
async fn safe_noop_permission_probe_surfaces_exchange_rejection() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/exchange"))
        .and(body_partial_json(json!({"action": {"type": "noop"}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "err",
            "response": "invalid signature"
        })))
        .mount(&server)
        .await;

    let h = hl(
        server.uri(),
        Some(HyperliquidCredentials {
            user_address: ACCOUNT.into(),
            private_key: Some(PRIVATE_KEY_ONE.into()),
            vault_address: None,
        }),
    );
    let error = h
        .validate_safe_noop_permission()
        .await
        .expect_err("exchange rejection must fail the noop probe");

    assert!(matches!(
        error,
        exchange::ExchangeError::Api { code, .. } if code == "noop"
    ));
}

#[tokio::test]
async fn live_balances_include_spot_usdc_without_perp_account() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({
            "type": "clearinghouseState",
            "user": "0x53fde8e60d9164647051193a43ac59b138c41305"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "marginSummary": {
                "accountValue": "0",
                "totalNtlPos": "0",
                "totalRawUsd": "0",
                "totalMarginUsed": "0"
            },
            "crossMarginSummary": {
                "accountValue": "0",
                "totalNtlPos": "0",
                "totalRawUsd": "0",
                "totalMarginUsed": "0"
            },
            "crossMaintenanceMarginUsed": "0",
            "withdrawable": "0",
            "assetPositions": []
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({
            "type": "spotClearinghouseState",
            "user": "0x53fde8e60d9164647051193a43ac59b138c41305"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "balances": [
                {"coin": "USDC", "token": 0, "total": "1.0", "hold": "0.0", "entryNtl": "0.0"}
            ]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({"type": "metaAndAssetCtxs"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(meta_asset_ctxs_fixture()))
        .mount(&server)
        .await;

    let h = hl(
        server.uri(),
        Some(HyperliquidCredentials {
            user_address: "0x53fde8e60d9164647051193a43ac59b138c41305".into(),
            private_key: None,
            vault_address: None,
        }),
    );

    let balances = h.get_balances(None).await.expect("live balances");

    assert!(balances.iter().any(|row| row.venue == "hyperliquid:spot"
        && row.currency == "USDC"
        && (row.available - 1.0).abs() < 1e-9));
}

#[tokio::test]
async fn account_read_keeps_spot_truth_when_perp_margin_fails() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({"type": "clearinghouseState"})))
        .respond_with(ResponseTemplate::new(503).set_body_string("perp unavailable"))
        .expect(3)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({"type": "spotClearinghouseState"})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                serde_json::from_str::<Value>(include_str!(
                    "../fixtures/hyperliquid/info_spot_clearinghouse_state_account_balance.json"
                ))
                .expect("spot fixture"),
            ),
        )
        .expect(1)
        .mount(&server)
        .await;
    let h = hl(
        server.uri(),
        Some(HyperliquidCredentials {
            user_address: ACCOUNT.into(),
            private_key: None,
            vault_address: None,
        }),
    );

    let read = h
        .get_account_read(Some("USDC"))
        .await
        .expect("partial read");

    assert!(read
        .balances
        .iter()
        .any(|row| row.venue == "hyperliquid:spot" && row.currency == "USDC"));
    assert!(read
        .issues
        .iter()
        .any(|issue| issue.venue == "hyperliquid" && issue.operation == "perp_margin"));
}

#[tokio::test]
async fn account_read_keeps_perp_margin_when_spot_truth_fails() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({"type": "clearinghouseState"})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                serde_json::from_str::<Value>(include_str!(
                    "../fixtures/hyperliquid/info_clearinghouse_state_account_balance.json"
                ))
                .expect("perp fixture"),
            ),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({"type": "spotClearinghouseState"})))
        .respond_with(ResponseTemplate::new(503).set_body_string("spot unavailable"))
        .expect(3)
        .mount(&server)
        .await;
    let h = hl(
        server.uri(),
        Some(HyperliquidCredentials {
            user_address: ACCOUNT.into(),
            private_key: None,
            vault_address: None,
        }),
    );

    let read = h
        .get_account_read(Some("USDC"))
        .await
        .expect("partial read");

    assert!(read
        .balances
        .iter()
        .any(|row| row.venue == "hyperliquid" && row.currency == "USDC"));
    assert_eq!(read.summaries.len(), 1);
    assert!(read
        .issues
        .iter()
        .any(|issue| { issue.venue == "hyperliquid:spot" && issue.operation == "spot_truth" }));
}

#[tokio::test]
async fn positions_signed_szi_to_long_short() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({
            "type": "clearinghouseState",
            "user": "0xABC"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "marginSummary": {
                "accountValue": "10000",
                "totalNtlPos": "5000",
                "totalRawUsd": "10000",
                "totalMarginUsed": "100"
            },
            "crossMarginSummary": {
                "accountValue": "10000",
                "totalNtlPos": "5000",
                "totalRawUsd": "10000",
                "totalMarginUsed": "100"
            },
            "crossMaintenanceMarginUsed": "0",
            "withdrawable": "9000",
            "assetPositions": [
                {
                    "type": "oneWay",
                    "position": {
                        "coin": "BTC",
                        "szi": "0.001",
                        "entryPx": "30000",
                        "leverage": {"type": "cross", "value": 10},
                        "liquidationPx": "27000",
                        "marginUsed": "10",
                        "unrealizedPnl": "0.1"
                    }
                },
                {
                    "type": "oneWay",
                    "position": {
                        "coin": "ETH",
                        "szi": "-0.5",
                        "entryPx": "2000",
                        "leverage": {"type": "cross", "value": 5},
                        "liquidationPx": "2500",
                        "marginUsed": "10",
                        "unrealizedPnl": "0.5"
                    }
                }
            ]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({"type": "metaAndAssetCtxs"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(meta_asset_ctxs_fixture()))
        .mount(&server)
        .await;

    let h = hl(
        server.uri(),
        Some(HyperliquidCredentials {
            user_address: "0xABC".into(),
            private_key: None,
            vault_address: None,
        }),
    );
    let positions = ExchangeAdapter::get_positions(&h, None).await.expect("ok");
    assert_eq!(positions.len(), 2);
    let btc = positions.iter().find(|p| p.symbol == "BTC").unwrap();
    assert_eq!(btc.side, "long");
    assert!((btc.quantity - 0.001).abs() < 1e-12);
    assert!((btc.leverage - 10.0).abs() < 1e-9);

    let eth = positions.iter().find(|p| p.symbol == "ETH").unwrap();
    assert_eq!(eth.side, "short");
    assert!((eth.quantity - 0.5).abs() < 1e-9);
}

#[tokio::test]
async fn missing_user_returns_auth_error() {
    let server = MockServer::start().await;
    let h = hl(server.uri(), None);
    let err = h.get_balance(None).await.expect_err("auth");
    assert!(matches!(err, exchange::ExchangeError::Auth(_)));
}

#[tokio::test]
async fn get_order_derives_public_client_id_to_official_cloid() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(body_partial_json(json!({
            "type": "orderStatus",
            "user": "0xABCDEF",
            "oid": "0xeb9a1d290f7f8020d7658c7985e7223c"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "unknownOid"
        })))
        .mount(&server)
        .await;

    let h = hl_with_writes(
        server.uri(),
        Some(HyperliquidCredentials {
            user_address: "0xABCDEF".into(),
            private_key: None,
            vault_address: None,
        }),
    );
    let order = h
        .get_order("BTC", "client-order-1")
        .await
        .expect("orderStatus accepts derived cloid");

    assert!(order.is_none());
}

const ACCOUNT: &str = "0x1111111111111111111111111111111111111111";
const VAULT: &str = "0x2222222222222222222222222222222222222222";
const PRIVATE_KEY_ONE: &str = "0000000000000000000000000000000000000000000000000000000000000001";
const AGENT_FROM_PRIVATE_KEY_ONE: &str = "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf";

fn l2book_fixture() -> Value {
    serde_json::from_str(include_str!("../fixtures/hyperliquid/l2book_btc.json")).unwrap()
}

fn meta_asset_ctxs_fixture() -> Value {
    serde_json::from_str(include_str!(
        "../fixtures/hyperliquid/meta_and_asset_ctxs_btc_eth.json"
    ))
    .unwrap()
}

fn spot_meta_asset_ctxs_fixture() -> Value {
    serde_json::from_str(include_str!(
        "../fixtures/hyperliquid/spot_meta_and_asset_ctxs_purr_hfun.json"
    ))
    .unwrap()
}

fn predicted_fundings_fixture() -> Value {
    serde_json::from_str(include_str!(
        "../fixtures/hyperliquid/predicted_fundings_avax.json"
    ))
    .unwrap()
}
