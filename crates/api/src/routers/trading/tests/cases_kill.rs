#![allow(clippy::panic)]
use super::super::*;
use super::fixtures::*;
use shared_types::ActionMutationChange;
use tokio::sync::broadcast::error::TryRecvError;

#[tokio::test]
async fn get_action_run_returns_detail_by_id() {
    let state = test_state().await;
    let run = action_runs::begin(
        &state,
        ActionRunStart {
            kind: ActionRunKind::TradingKillSwitch,
            actor: "tester".to_owned(),
            target: Some("kill-switch".to_owned()),
            idempotency_key: Some("detail-1".to_owned()),
            message: "accepted".to_owned(),
        },
    )
    .unwrap_or_else(|error| panic!("action run begin failed: {error}"));

    let detail = match get_action_run(State(state.clone()), Path(run.id.clone())).await {
        Ok(Json(detail)) => detail,
        Err(error) => panic!("action run detail failed: {error}"),
    };
    let missing = get_action_run(State(state), Path("missing".to_owned())).await;

    assert_eq!(detail.id, run.id);
    assert_eq!(detail.idempotency_key.as_deref(), Some("detail-1"));
    assert!(matches!(missing, Err(error) if error.status() == StatusCode::NOT_FOUND));
}

#[tokio::test]
async fn set_kill_switch_rejects_missing_reason_and_records_failed_action() {
    let state = test_state().await;

    let result = set_kill_switch(
        State(state.clone()),
        HeaderMap::new(),
        ApiJson(kill_switch_request(true, false, 0, " ")),
    )
    .await;

    let error = match result {
        Ok(response) => panic!("missing reason unexpectedly toggled: {response:?}"),
        Err(error) => error,
    };
    let run = find_action_run(&state, ActionRunKind::TradingKillSwitch);

    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error.code(), codes::KILL_SWITCH_REQUEST_INVALID);
    assert_eq!(run.status, ActionRunStatus::Failed);
    assert_eq!(
        run.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::KILL_SWITCH_REQUEST_INVALID)
    );
}

#[tokio::test]
async fn set_kill_switch_action_run_keeps_summary_payload() {
    let state = test_state().await;

    let Json(response) = match set_kill_switch(
        State(state.clone()),
        HeaderMap::new(),
        ApiJson(kill_switch_request(
            true,
            false,
            0,
            "positions.kill_switch.enable",
        )),
    )
    .await
    {
        Ok(response) => response,
        Err(error) => panic!("kill switch update failed: {error}"),
    };
    let run = find_action_run(&state, ActionRunKind::TradingKillSwitch);
    let stored = action_runs::replay_payload::<shared_types::KillSwitchResponse>(&run)
        .unwrap_or_else(|error| panic!("kill switch response payload missing: {error}"));

    assert!(response.status.risk.kill_switch_active);
    assert!(!response.summary.previous_active);
    assert!(response.summary.active);
    assert_eq!(response.summary.open_order_count, 0);
    assert_eq!(response.summary.reason, "positions.kill_switch.enable");
    assert_eq!(run.target.as_deref(), Some("kill-switch:on"));
    assert_eq!(stored.summary.reason, response.summary.reason);
    assert_eq!(run.mutation, response.status.mutation);
    assert_eq!(
        response.status.action_run_id.as_deref(),
        Some(run.id.as_str())
    );
    assert!(response
        .status
        .mutation
        .as_ref()
        .is_some_and(|mutation| mutation.changes.iter().any(|change| matches!(
            change,
            ActionMutationChange::KillSwitchActive {
                before: false,
                after: true
            }
        ))));
}

#[tokio::test]
async fn set_kill_switch_replays_existing_idempotency_key_without_second_risk_event() {
    let state = test_state().await;
    let mut risk_events = state.ws_hub().subscribe(realtime::channels::RISK_ALERTS);
    let headers = idempotency_headers(HEADER_IDEMPOTENCY_KEY, "kill-switch-replay-1");

    let Json(first) = match set_kill_switch(
        State(state.clone()),
        headers.clone(),
        ApiJson(kill_switch_request(
            true,
            false,
            0,
            "positions.kill_switch.enable",
        )),
    )
    .await
    {
        Ok(response) => response,
        Err(error) => panic!("kill switch update failed: {error}"),
    };
    match risk_events.try_recv() {
        Ok(_) => {}
        other => panic!("first kill switch did not publish risk event: {other:?}"),
    }

    let Json(replay) = match set_kill_switch(
        State(state.clone()),
        headers,
        ApiJson(kill_switch_request(
            true,
            false,
            0,
            "positions.kill_switch.enable",
        )),
    )
    .await
    {
        Ok(response) => response,
        Err(error) => panic!("kill switch replay failed: {error}"),
    };
    let run = find_action_run(&state, ActionRunKind::TradingKillSwitch);

    assert_eq!(replay.action_run_id, first.action_run_id);
    assert_eq!(replay.summary.reason, first.summary.reason);
    assert!(matches!(risk_events.try_recv(), Err(TryRecvError::Empty)));
    assert_eq!(run.idempotency_key.as_deref(), Some("kill-switch-replay-1"));
    assert_eq!(
        action_runs::recent(&state)
            .into_iter()
            .filter(|run| run.kind == ActionRunKind::TradingKillSwitch)
            .count(),
        1
    );
}

#[tokio::test]
async fn set_kill_switch_replays_failed_idempotency_key_without_second_action_run() {
    let state = test_state().await;
    let headers = idempotency_headers(HEADER_IDEMPOTENCY_KEY, "kill-switch-failed-1");
    let first = set_kill_switch(
        State(state.clone()),
        headers.clone(),
        ApiJson(kill_switch_request(true, false, 0, " ")),
    )
    .await;
    let error = match first {
        Ok(response) => panic!("missing reason unexpectedly toggled: {response:?}"),
        Err(error) => error,
    };
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);

    let replay = set_kill_switch(
        State(state.clone()),
        headers,
        ApiJson(kill_switch_request(
            true,
            false,
            0,
            "positions.kill_switch.enable",
        )),
    )
    .await;
    let error = match replay {
        Ok(response) => panic!("failed key unexpectedly replayed as success: {response:?}"),
        Err(error) => error,
    };

    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error.code(), codes::KILL_SWITCH_REQUEST_INVALID);
    if let AppError::Domain {
        details: Some(details),
        ..
    } = &error
    {
        assert_eq!(details["replayed"], true);
        assert_eq!(
            details["idempotencyKey"],
            serde_json::json!("kill-switch-failed-1")
        );
        assert_eq!(
            details["originalProblem"]["code"],
            serde_json::json!(codes::KILL_SWITCH_REQUEST_INVALID)
        );
    } else {
        panic!("kill switch replay did not carry original problem details: {error:?}");
    }
    assert_eq!(
        action_runs::recent(&state)
            .into_iter()
            .filter(|run| run.kind == ActionRunKind::TradingKillSwitch)
            .count(),
        1
    );
}

#[tokio::test]
async fn kill_switch_explicit_x_idempotency_header_is_used() {
    let state = test_state().await;

    let response = set_kill_switch(
        State(state.clone()),
        idempotency_headers(HEADER_X_IDEMPOTENCY_KEY, "kill-switch-x-1"),
        ApiJson(kill_switch_request(
            true,
            false,
            0,
            "positions.kill_switch.enable",
        )),
    )
    .await;
    let Json(_) = match response {
        Ok(response) => response,
        Err(error) => panic!("kill switch update failed: {error}"),
    };
    let run = find_action_run(&state, ActionRunKind::TradingKillSwitch);

    assert_eq!(run.idempotency_key.as_deref(), Some("kill-switch-x-1"));
}
