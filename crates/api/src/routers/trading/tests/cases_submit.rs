#![allow(clippy::panic)]
use super::super::*;
use super::fixtures::*;
use tokio::sync::broadcast::error::TryRecvError;

#[tokio::test]
async fn submit_order_requires_client_order_id() {
    let state = test_state().await;
    let result = submit_order(
        State(state),
        HeaderMap::new(),
        ApiJson(submit_payload("manual-a", None, "dry_run")),
    )
    .await;

    let error = match result {
        Ok(record) => panic!("missing clientOrderId unexpectedly submitted: {record:?}"),
        Err(error) => error,
    };

    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error.code(), codes::SUBMIT_ORDER_INVALID);
}

#[tokio::test]
async fn submit_order_replays_existing_client_order_id_without_second_order_event() {
    let state = test_state().await;
    let mut orders = state.ws_hub().subscribe(realtime::channels::ORDERS);

    let first = submitted_record(&state, "manual-a", "client-a").await;
    match orders.try_recv() {
        Ok(_) => {}
        other => panic!("first submit did not publish an order event: {other:?}"),
    }

    let replay = submitted_record(&state, "manual-b", "client-a").await;

    assert_eq!(replay.intent.id, first.intent.id);
    assert_eq!(replay.intent.client_order_id, "client-a");
    assert!(matches!(orders.try_recv(), Err(TryRecvError::Empty)));
    assert_eq!(
        action_runs::recent(&state)
            .into_iter()
            .filter(|run| run.kind == ActionRunKind::TradingOrderSubmit)
            .count(),
        1
    );
}

#[tokio::test]
async fn submit_order_replays_succeeded_payload_when_order_record_missing() {
    let source_state = test_state().await;
    let record = submitted_record(&source_state, "manual-payload", "client-payload").await;
    let run = find_action_run(&source_state, ActionRunKind::TradingOrderSubmit);

    let replay_state = test_state().await;
    replay_state.action_runs().insert(run.id.clone(), run);
    let replay = submitted_record(&replay_state, "manual-replay", "client-payload").await;

    assert_eq!(replay.intent.id, record.intent.id);
    assert_eq!(replay.intent.client_order_id, "client-payload");
    assert_eq!(
        action_runs::recent(&replay_state)
            .into_iter()
            .filter(|run| run.kind == ActionRunKind::TradingOrderSubmit)
            .count(),
        1
    );
}

#[tokio::test]
async fn cancel_order_replays_existing_order_id_without_second_order_event() {
    let state = test_state().await;
    state
        .trading_service()
        .update_risk_config(|config| config.live_trading_enabled = true);
    let mut orders = state.ws_hub().subscribe(realtime::channels::ORDERS);

    let first = submitted_live_record(&state, "manual-cancel", "client-cancel").await;
    match orders.try_recv() {
        Ok(_) => {}
        other => panic!("submit did not publish an order event: {other:?}"),
    }

    let cancelled = cancelled_record(&state, &first.intent.id).await;
    assert_eq!(cancelled.intent.id, first.intent.id);
    assert_eq!(
        cancelled.state,
        shared_types::LiveOrderState::CancelRequested
    );
    assert_eq!(recv_order_event(&mut orders), "order_cancel_requested");

    let replay = cancelled_record(&state, &first.intent.id).await;

    assert_eq!(replay.intent.id, first.intent.id);
    assert_eq!(replay.state, shared_types::LiveOrderState::CancelRequested);
    assert!(matches!(orders.try_recv(), Err(TryRecvError::Empty)));
    assert_eq!(
        action_runs::recent(&state)
            .into_iter()
            .filter(|run| run.kind == ActionRunKind::TradingOrderCancel)
            .count(),
        1
    );
    assert_eq!(
        find_action_run(&state, ActionRunKind::TradingOrderCancel).message,
        "order cancel accepted; awaiting finality"
    );
}

#[tokio::test]
async fn cancel_order_replays_failed_order_id_without_second_action_run() {
    let state = test_state().await;
    let first = cancel_order(
        State(state.clone()),
        HeaderMap::new(),
        Path("missing-order".to_owned()),
    )
    .await;
    let error = match first {
        Ok(record) => panic!("missing order unexpectedly cancelled: {record:?}"),
        Err(error) => error,
    };
    assert_eq!(error.status(), StatusCode::NOT_FOUND);

    let replay = cancel_order(
        State(state.clone()),
        HeaderMap::new(),
        Path("missing-order".to_owned()),
    )
    .await;
    let error = match replay {
        Ok(record) => panic!("failed cancel unexpectedly replayed as success: {record:?}"),
        Err(error) => error,
    };

    assert_eq!(error.status(), StatusCode::CONFLICT);
    assert_eq!(error.code(), codes::ACTION_RUN_REPLAY_FAILED);
    assert_eq!(
        action_runs::recent(&state)
            .into_iter()
            .filter(|run| run.kind == ActionRunKind::TradingOrderCancel)
            .count(),
        1
    );
}

#[tokio::test]
async fn cancel_order_accepts_new_explicit_key_after_failed_attempt() {
    let state = test_state().await;
    let first_headers = idempotency_headers(HEADER_IDEMPOTENCY_KEY, "cancel-retry-1");
    let first = cancel_order(
        State(state.clone()),
        first_headers.clone(),
        Path("missing-retry-order".to_owned()),
    )
    .await;
    assert!(matches!(first, Err(error) if error.status() == StatusCode::NOT_FOUND));

    let replay = cancel_order(
        State(state.clone()),
        first_headers,
        Path("missing-retry-order".to_owned()),
    )
    .await;
    assert!(matches!(replay, Err(error) if error.code() == codes::ACTION_RUN_REPLAY_FAILED));

    let retry = cancel_order(
        State(state.clone()),
        idempotency_headers(HEADER_IDEMPOTENCY_KEY, "cancel-retry-2"),
        Path("missing-retry-order".to_owned()),
    )
    .await;
    assert!(matches!(retry, Err(error) if error.status() == StatusCode::NOT_FOUND));

    let mut runs = action_runs::recent(&state)
        .into_iter()
        .filter(|run| run.kind == ActionRunKind::TradingOrderCancel)
        .collect::<Vec<_>>();
    runs.sort_by(|a, b| a.idempotency_key.cmp(&b.idempotency_key));
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].idempotency_key.as_deref(), Some("cancel-retry-1"));
    assert_eq!(runs[1].idempotency_key.as_deref(), Some("cancel-retry-2"));
}
