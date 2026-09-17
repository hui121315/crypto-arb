use super::*;
use crate::services::onchain_cross_chain_run_store::recovery;
use std::sync::atomic::{AtomicUsize, Ordering};

async fn fixture() -> (tempfile::TempDir, AppState, Run, Request) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recovery.jsonl");
    let (store, run) = recovery::tests::pending_bridge(&path);
    let mut report = recovery::tests::received(&run);
    let token = run.build.legs[0].to_token.clone();
    report.receiving_token = Some(token.clone());
    let resolution = report.token_resolution.as_mut().unwrap();
    resolution.address = token.clone();
    let identity = resolution.identity.as_mut().unwrap();
    identity.address = token;
    identity.symbol = "TKN".into();
    report.receipt.as_mut().unwrap().basis = recovery::basis(&run, &run.legs[1], &report).unwrap();
    let run = store
        .record_bridge_recovery(&run.run_id, 2, report, 3000)
        .unwrap();
    assert!(run
        .accounting
        .as_ref()
        .unwrap()
        .disposition
        .as_ref()
        .unwrap()
        .blockers
        .is_empty());
    drop(store);
    let mut config = common::config::AppConfig::default();
    config.history.enabled = false;
    config.storage.onchain_cross_chain_ledger_path = Some(path.to_string_lossy().into_owned());
    config.security.auth_token = Some("offline-recovery-preview".into());
    let state = AppState::new(config).await.unwrap();
    state
        .onchain_monitor()
        .update_config(
            &shared_types::OnchainComparisonConfigPatch {
                chain: Some("ethereum".into()),
                wallet_address: Some(
                    run.legs[0]
                        .swap_execution
                        .as_ref()
                        .unwrap()
                        .wallet_address
                        .clone(),
                ),
                ..Default::default()
            },
            4000,
        )
        .unwrap();
    let request = Request {
        run_id: run.run_id.clone(),
        expected_run_updated_at_ms: run.updated_at_ms,
        asset_index: 0,
        amount_exact: "50".into(),
    };
    (dir, state, run, request)
}

struct FakeIo<'a> {
    balance: u128,
    missing_fee: bool,
    expired: bool,
    readiness_problem: Option<&'static str>,
    change: Option<(&'a AppState, &'a Run)>,
    quotes: AtomicUsize,
}
impl Default for FakeIo<'_> {
    fn default() -> Self {
        Self {
            balance: 200_000_000,
            missing_fee: false,
            expired: false,
            readiness_problem: None,
            change: None,
            quotes: AtomicUsize::new(0),
        }
    }
}
#[async_trait::async_trait]
impl PreviewIo for FakeIo<'_> {
    async fn balance(
        &self,
        _: &Config,
        _: &str,
    ) -> Result<wallet_inventory::ExactAssetBalance, String> {
        Ok(wallet_inventory::ExactAssetBalance {
            amount_raw: self.balance,
            source: "fixture_rpc",
        })
    }
    async fn quote(
        &self,
        request: &lifi::RecoveryQuoteRequest,
    ) -> Result<lifi::RecoveryQuote, String> {
        self.quotes.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.amount_raw, "50000000");
        if let Some((state, run)) = self.change {
            state
                .onchain_cross_chain_runs()
                .request_recheck(&run.run_id, "tester", 2, common::time::now_ms())
                .unwrap();
        }
        let now = common::time::now_ms();
        Ok(lifi::RecoveryQuote {
            route_id: "read-only-quote".into(),
            output_raw: "49000000".into(),
            minimum_raw: "48500000".into(),
            fee_usd: (!self.missing_fee).then_some(0.1),
            gas_usd: Some(0.02),
            duration_seconds: Some(4),
            transaction: serde_json::from_value(
                serde_json::json!({"kind":"evm_call","chain_id":1,"from":request.from_wallet,
                "to":format!("0x{:040x}",9),"data":"0x1234","value":"0x0","gas":"0x5208"}),
            )
            .unwrap(),
            observed_at_ms: now,
            valid_until_ms: if self.expired { now - 1 } else { now + 55_000 },
        })
    }
    async fn readiness(
        &self,
        _: &Config,
        _: &str,
        _: &str,
        _: &lifi::RecoveryQuote,
    ) -> Result<(), String> {
        self.readiness_problem
            .map_or(Ok(()), |problem| Err(problem.into()))
    }
}

#[tokio::test]
async fn recovery_preview_caps_amount_preserves_journal_and_returns_new_quote() {
    let (dir, state, run, request) = fixture().await;
    let before = std::fs::read(dir.path().join("recovery.jsonl")).unwrap();
    let io = FakeIo::default();
    let result = prepare(&state, &request, "tester", &io).await.unwrap();
    assert!(result.quote_ready, "{:?}", result.blockers);
    assert_eq!(result.input_amount_raw, "50000000");
    assert_eq!(result.balance_amount_raw.as_deref(), Some("200000000"));
    assert_eq!(
        result.minimum_output_amount_raw.as_deref(),
        Some("48500000")
    );
    assert!(!result.submit_ready);
    assert!(result.requires_live_authorization);
    assert_eq!(
        state
            .onchain_cross_chain_runs()
            .run(&run.run_id, common::time::now_ms())
            .unwrap(),
        run
    );
    let after = std::fs::read(dir.path().join("recovery.jsonl")).unwrap();
    assert!(after.starts_with(&before));
    assert!(after.len() > before.len());
    assert!(result.plan_id.is_some());
    assert_eq!(state.onchain_cross_chain_runs().runs(128, common::time::now_ms()).recovery_plans.len(), 1);
    let mut oversized = request.clone();
    oversized.amount_exact = "99".into();
    assert_eq!(
        prepare(&state, &oversized, "tester", &io)
            .await
            .unwrap_err()
            .code(),
        "ONCHAIN_RECOVERY_AMOUNT_EXCEEDED"
    );
    assert_eq!(io.quotes.load(Ordering::SeqCst), 1);
    assert!(select(&run, &request, "other-actor").is_err());
    let mut stale = request;
    stale.expected_run_updated_at_ms -= 1;
    assert!(select(&run, &stale, "tester").is_err());
}

#[tokio::test]
async fn recovery_preview_stops_on_insufficient_balance_unknown_cost_expiry_and_changed_source() {
    let (_dir, state, run, request) = fixture().await;
    let io = FakeIo {
        balance: 1,
        ..Default::default()
    };
    let result = prepare(&state, &request, "tester", &io).await.unwrap();
    assert!(!result.quote_ready);
    assert_eq!(io.quotes.load(Ordering::SeqCst), 0);
    assert!(result
        .blockers
        .iter()
        .any(|problem| problem.contains("余额低于")));
    for io in [
        FakeIo {
            missing_fee: true,
            ..Default::default()
        },
        FakeIo {
            expired: true,
            ..Default::default()
        },
        FakeIo {
            readiness_problem: Some("Gas 或代币授权不足"),
            ..Default::default()
        },
    ] {
        let result = prepare(&state, &request, "tester", &io).await.unwrap();
        assert!(!result.quote_ready);
        assert!(!result.blockers.is_empty());
    }
    let io = FakeIo {
        change: Some((&state, &run)),
        ..Default::default()
    };
    assert_eq!(
        prepare(&state, &request, "tester", &io)
            .await
            .unwrap_err()
            .code(),
        "ONCHAIN_RECOVERY_SOURCE_CHANGED"
    );
}

#[test]
fn recovery_preview_exact_amount_never_rounds_or_uses_scientific_notation() {
    assert_eq!(to_raw("0.000001", 6).unwrap(), 1);
    for value in ["0", "-1", "1e3", "NaN", "0.0000001", "1.2.3"] {
        assert!(to_raw(value, 6).is_err());
    }
    assert!(to_raw("1", 29).is_err());
    assert!(to_raw("9999999999999999999999.0000001", 6).is_err());
}

#[tokio::test]
async fn recovery_preview_http_route_requires_auth_and_original_actor_before_io() {
    use axum::{
        body::Body,
        http::{header, Request as HttpRequest},
    };
    use tower::ServiceExt;
    let (_dir, state, _, request) = fixture().await;
    let router = crate::app::build_router(state);
    for (token, expected) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some("offline-recovery-preview"), StatusCode::FORBIDDEN),
    ] {
        let mut builder = HttpRequest::builder()
            .method("POST")
            .uri("/api/onchain/cross-chain/recovery/preview")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let response = router
            .clone()
            .oneshot(
                builder
                    .body(Body::from(serde_json::to_vec(&request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
}
