use super::*;
use axum::body::{to_bytes, Body};
use axum::http::{header, Request};
use common::config::AppConfig;
use tower::ServiceExt;

#[tokio::test]
async fn replenishment_recheck_http_requires_auth_and_returns_a_durable_read_only_run() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("replenishment.jsonl");
    let token = "offline-recheck-token";
    let mut actor_headers = HeaderMap::new();
    audit::insert_verified_bearer_actor(&mut actor_headers, token, None);
    let mut original: shared_types::OnchainReplenishmentRun = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"), "/../../shared-types/fixtures/onchain_replenishment_locked.json"
    ))).unwrap();
    original.authorization.actor = audit::extract_actor(&actor_headers);
    original.status = shared_types::OnchainReplenishmentRunStatus::Paused;
    original.transfers[0].status = shared_types::OnchainReplenishmentTransferStatus::Paused;
    std::fs::write(&path, format!("{}\n", json!({"schemaVersion":1,"run":original}))).unwrap();
    let mut config = AppConfig::default();
    config.history.enabled = false;
    config.security.auth_token = Some(token.into());
    config.storage.onchain_replenishment_ledger_path = Some(path.to_string_lossy().into_owned());
    let state = AppState::new(config).await.unwrap();
    let router = crate::app::build_router(state.clone());
    let request = original.recheck_request().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    for authenticated in [false, true] {
        let mut builder = Request::builder().method("POST").uri("/api/onchain/replenishment/recheck")
            .header(header::CONTENT_TYPE, "application/json");
        if authenticated { builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}")); }
        let response = router.clone().oneshot(builder.body(Body::from(serde_json::to_vec(&request).unwrap())).unwrap()).await.unwrap();
        if !authenticated {
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            continue;
        }
        assert_eq!(response.status(), StatusCode::OK);
        let recovered: shared_types::OnchainReplenishmentRun = serde_json::from_slice(&to_bytes(response.into_body(), 128*1024).await.unwrap()).unwrap();
        assert!(recovered.read_only_recovery);
        assert_eq!(recovered.transfers.len(), 1);
        assert_eq!(recovered.transfers[0].transaction_id, original.transfers[0].transaction_id);
        assert_eq!(recovered.plan, original.plan);
        assert_eq!(onchain_comparison::submit_replenishment(&state, &shared_types::OnchainReplenishmentSubmitRequest { run_id: original.run_id.clone() }, &original.authorization.actor).await.unwrap_err().code(), "ONCHAIN_REPLENISHMENT_READ_ONLY_RECOVERY");
    }
}

#[tokio::test]
async fn replenishment_recovery_http_reports_damage_and_blocks_funds_without_rewriting_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("replenishment.jsonl");
    let original: shared_types::OnchainReplenishmentRun =
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../shared-types/fixtures/onchain_replenishment_locked.json"
        )))
        .unwrap();
    let bytes = format!(
        "{}\n{{\"schemaVersion\":1",
        json!({"schemaVersion":1,"run":original})
    );
    std::fs::write(&path, &bytes).unwrap();
    let mut config = AppConfig::default();
    config.history.enabled = false;
    config.security.auth_token = Some("offline-recovery-token".to_owned());
    config.storage.onchain_replenishment_ledger_path = Some(path.to_string_lossy().into_owned());
    let state = AppState::new(config).await.unwrap();
    let router = crate::app::build_router(state.clone());
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/onchain/replenishment/runs?limit=1")
                .header(header::AUTHORIZATION, "Bearer offline-recovery-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let snapshot: OnchainReplenishmentRunsResponse =
        serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await.unwrap()).unwrap();
    assert_eq!(snapshot.rows.len(), 1);
    assert_eq!(snapshot.rows[0].transfers, original.transfers);
    assert!(snapshot
        .recovery_problem
        .unwrap()
        .contains("第 2 行未完整写入"));

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/onchain/replenishment/submit")
                .header(header::AUTHORIZATION, "Bearer offline-recovery-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"runId":original.run_id}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), 128 * 1024).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("ONCHAIN_REPLENISHMENT_RECOVERY_UNAVAILABLE"));
    onchain_comparison::reconcile_replenishment(&state, common::time::now_ms()).await;
    assert_eq!(std::fs::read_to_string(&path).unwrap(), bytes);
}
