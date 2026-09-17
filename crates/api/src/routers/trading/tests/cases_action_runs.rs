#![allow(clippy::panic)]
use super::super::*;
use super::fixtures::*;
use shared_types::ActionMutationChange;

#[tokio::test]
async fn submit_order_action_run_detail_restores_order_payload() {
    let state = test_state().await;
    let record = submitted_record(&state, "manual-detail", "client-detail").await;
    let run = find_action_run(&state, ActionRunKind::TradingOrderSubmit);

    let detail = match get_action_run(State(state), Path(run.id.clone())).await {
        Ok(Json(detail)) => detail,
        Err(error) => panic!("action run detail failed: {error}"),
    };
    let restored = action_runs::replay_payload::<OrderRecord>(&detail)
        .unwrap_or_else(|error| panic!("action run detail did not keep order payload: {error}"));

    assert_eq!(detail.id, run.id);
    assert_eq!(
        restored.intent.client_order_id,
        record.intent.client_order_id
    );
    assert_eq!(restored.intent.id, record.intent.id);
}

#[tokio::test]
async fn adapter_select_action_run_detail_restores_status_payload() {
    let state = test_state().await;

    let Json(response) = select_adapter(
        State(state.clone()),
        HeaderMap::new(),
        Json(SelectAdapterPayload {
            adapter_id: "mock".to_owned(),
        }),
    )
    .await
    .unwrap_or_else(|error| panic!("adapter select failed: {error}"));
    let run = find_action_run(&state, ActionRunKind::TradingAdapterSelect);

    let restored = action_runs::replay_payload::<TradingStatusResponse>(&run)
        .unwrap_or_else(|error| panic!("adapter select action payload missing: {error}"));

    assert_eq!(restored.adapter, "mock");
    assert_eq!(restored.adapter, response.adapter);
    assert_eq!(response.action_run_id.as_deref(), Some(run.id.as_str()));
    assert!(response.mutation.is_none());
    assert!(run.mutation.is_none());
}

#[tokio::test]
async fn adapter_select_replays_explicit_idempotency_key_without_second_action_run() {
    let state = test_state().await;
    let headers = idempotency_headers(HEADER_IDEMPOTENCY_KEY, "adapter-select-1");

    let Json(first) = select_adapter(
        State(state.clone()),
        headers.clone(),
        Json(SelectAdapterPayload {
            adapter_id: "mock".to_owned(),
        }),
    )
    .await
    .unwrap_or_else(|error| panic!("adapter select failed: {error}"));
    let Json(replayed) = select_adapter(
        State(state.clone()),
        headers,
        Json(SelectAdapterPayload {
            adapter_id: "live_router".to_owned(),
        }),
    )
    .await
    .unwrap_or_else(|error| panic!("adapter replay failed: {error}"));

    assert_eq!(replayed, first);
    assert_eq!(
        replayed.idempotency_key.as_deref(),
        Some("adapter-select-1")
    );
    assert_eq!(
        action_runs::recent(&state)
            .into_iter()
            .filter(|run| run.kind == ActionRunKind::TradingAdapterSelect)
            .count(),
        1
    );
}

#[tokio::test]
async fn trading_mutation_diff_captures_adapter_environment_and_risk_changes() {
    let state = test_state().await;
    let service = state.trading_service();
    let before = status_response(service, &service.risk_config());
    let mut after = before.clone();
    after.adapter = "live_router".to_owned();
    after.environment = ExecutionEnvironment::Live;
    after.risk.live_trading_enabled = true;

    let diff = trading_mutation_diff(&before, &after)
        .unwrap_or_else(|| panic!("adapter mutation diff missing"));

    assert!(diff.changes.iter().any(|change| matches!(
        change,
        ActionMutationChange::Adapter { before, after }
            if before == "mock" && after == "live_router"
    )));
    assert!(diff.changes.iter().any(|change| matches!(
        change,
        ActionMutationChange::ExecutionEnvironment { before, after }
            if before == "paper" && after == "live"
    )));
    assert!(diff.changes.iter().any(|change| matches!(
        change,
        ActionMutationChange::LiveTradingEnabled {
            before: false,
            after: true
        }
    )));
}

#[tokio::test]
async fn fee_snapshot_action_run_detail_restores_snapshot_payload() {
    let state = test_state().await;
    let now_ms = common::time::now_ms();
    let snapshot = crate::services::fees::standard_fee_snapshot(
        "binance",
        "BTCUSDT",
        shared_types::FeeProduct::Perp,
        false,
        now_ms,
    )
    .unwrap_or_else(|| panic!("standard fee snapshot missing"));

    let Json(response) = upsert_fee_snapshot(
        State(state.clone()),
        HeaderMap::new(),
        Json(snapshot.clone()),
    )
    .await
    .unwrap_or_else(|error| panic!("fee snapshot upsert failed: {error}"));
    let run = find_action_run(&state, ActionRunKind::TradingFeeSnapshotUpsert);

    let restored = action_runs::replay_payload::<TradeFeeSnapshot>(&run)
        .unwrap_or_else(|error| panic!("fee snapshot action payload missing: {error}"));

    assert_eq!(restored.venue, snapshot.venue);
    assert_eq!(restored.symbol, snapshot.symbol);
    assert_eq!(restored.product, snapshot.product);
    assert_eq!(restored, response);
}

#[tokio::test]
async fn reconcile_action_run_detail_restores_diff_payload() {
    let state = test_state().await;

    let Json(response) = reconcile_orders(State(state.clone()), HeaderMap::new())
        .await
        .unwrap_or_else(|error| panic!("order reconcile failed: {error}"));
    let run = find_action_run(&state, ActionRunKind::TradingOrderReconcile);

    let restored = action_runs::replay_payload::<Vec<trading::ReconcileDiff>>(&run)
        .unwrap_or_else(|error| panic!("reconcile action payload missing: {error}"));

    assert_eq!(restored, response);
}
