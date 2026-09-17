use super::*;
use axum::body::{to_bytes, Body};
use axum::http::{header, Request};
use common::config::AppConfig;
use tower::ServiceExt;

const TOKEN: &str = "offline-cross-chain-recheck-test";

fn paused_run(actor: &str) -> OnchainCrossChainRun {
    let mut run: OnchainCrossChainRun = serde_json::from_str(r#"{
        "runId":"recheck-fixture","idempotencyKey":"existing-operation","status":"paused",
        "authorization":{"actor":"replace","authorizedAtMs":1,"validUntilMs":2,"confirmationVersion":"v1"},
        "activePosition":1,"createdAtMs":1,"updatedAtMs":2,"nextAction":"verify existing transfer",
        "problem":"receipt temporarily unavailable",
        "build":{"buildId":"build","provider":"fixture","sourceChain":"base","peerChain":"arbitrum",
            "legs":[],"initialQuoteAmountRaw":"100","finalQuoteAmountRaw":"101",
            "quoteObservedAtMs":1,"builtAtMs":1,"validUntilMs":2,
            "atomic":false,"monitorOnly":false,"previewReady":true,"submitReady":true},
        "legs":[{"position":1,"kind":"source_swap","clientActionId":"original-action",
            "status":"paused","attempts":1,"plannedInputAmountRaw":"100","actualInputAmountRaw":"100",
            "minimumOutputAmountRaw":"98","sourceTransactionId":"0xoriginal",
            "swapExecution":{"executionId":"original-execution","position":1,"kind":"source_swap",
                "provider":"fixture","chain":"base","walletAddress":"0xwallet",
                "inputToken":"0xinput","outputToken":"0xoutput","inputAmountRaw":"100",
                "quotedOutputAmountRaw":"99","minimumOutputAmountRaw":"98",
                "transaction":{"kind":"evm_call","chain_id":8453,"from":"0xwallet",
                    "to":"0xrouter","data":"0x1234","value":"0x0","gas":"0x5208"},
                "quoteObservedAtMs":1,"validUntilMs":2,"rebuildAfterPosition":0,"officialDocsUrl":"https://example.test"}}]
    }"#).unwrap();
    run.authorization.actor = actor.to_owned();
    run
}

fn request(authenticated: bool, body: &str) -> Request<Body> {
    let builder = Request::builder()
        .method("POST")
        .uri("/api/onchain/cross-chain/recheck")
        .header(header::CONTENT_TYPE, "application/json");
    let builder = if authenticated {
        builder.header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
    } else {
        builder
    };
    builder.body(Body::from(body.to_owned())).unwrap()
}

#[tokio::test]
async fn cross_chain_recheck_http_is_authenticated_durable_and_never_rebroadcasts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cross-chain.jsonl");
    let mut headers = HeaderMap::new();
    audit::insert_verified_bearer_actor(&mut headers, TOKEN, None);
    let original = paused_run(&audit::extract_actor(&headers));
    std::fs::write(
        &path,
        format!("{}\n", json!({"schemaVersion":1,"run":original})),
    )
    .unwrap();
    let mut config = AppConfig::default();
    config.history.enabled = false;
    config.security.auth_token = Some(TOKEN.into());
    config.storage.onchain_cross_chain_ledger_path = Some(path.to_string_lossy().into_owned());
    let state = AppState::new(config).await.unwrap();
    let router = crate::app::build_router(state.clone());
    let body = r#"{"runId":"recheck-fixture","expectedPosition":1}"#;
    assert_eq!(
        router
            .clone()
            .oneshot(request(false, body))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(router
        .clone()
        .oneshot(request(true, r#"{"runId":"recheck-fixture"}"#))
        .await
        .unwrap()
        .status()
        .is_client_error());
    let response = router.clone().oneshot(request(true, body)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 128 * 1024).await.unwrap();
    let queued: OnchainCrossChainRun = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        queued.status,
        shared_types::OnchainCrossChainRunStatus::AwaitingSourceFinality
    );
    assert_eq!(
        queued.legs[0].source_transaction_id,
        original.legs[0].source_transaction_id
    );
    assert_eq!(queued.legs[0].attempts, 1);
    assert_eq!(
        queued.legs[0].swap_execution,
        original.legs[0].swap_execution
    );
    assert!(queued.legs[0].last_checked_at_ms.is_none());
    let replay = router.oneshot(request(true, body)).await.unwrap();
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(
        state
            .onchain_cross_chain_runs()
            .run(&queued.run_id, common::time::now_ms())
            .unwrap(),
        queued
    );
    assert_eq!(std::fs::read_to_string(path).unwrap().lines().count(), 2);
}
