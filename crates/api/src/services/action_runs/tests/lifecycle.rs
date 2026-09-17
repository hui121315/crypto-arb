use super::super::lifecycle::find_by_idempotency_key;
use super::*;
use shared_types::problem::codes;
use shared_types::ResourceStatus;

#[tokio::test]
async fn terminal_action_run_update_wakes_internal_consumers() {
    let state = test_state().await;
    let mut activity = state
        .ws_hub()
        .subscribe_activity(realtime::channels::ACTION_RUN_ACTIVITY);
    let run = begin(
        &state,
        ActionRunStart {
            kind: ActionRunKind::PortfolioCloseCompensation,
            actor: "system:test".to_owned(),
            target: Some("close-1".to_owned()),
            idempotency_key: None,
            message: "accepted".to_owned(),
        },
    )
    .unwrap_or_else(|error| panic!("action run begin failed: {error}"));

    finish_status(&state, &run.id, ActionRunStatus::Succeeded, "done", None)
        .unwrap_or_else(|error| panic!("action run finish failed: {error}"));

    tokio::time::timeout(std::time::Duration::from_millis(50), activity.changed())
        .await
        .unwrap_or_else(|error| panic!("activity wait timed out: {error}"))
        .unwrap_or_else(|error| panic!("activity sender closed: {error}"));
}

#[tokio::test]
async fn action_run_captures_request_id_and_failure_problem() {
    let state = test_state().await;

    common::request_id::scope("req-action-1".to_owned(), async {
        let run = begin(
            &state,
            ActionRunStart {
                kind: ActionRunKind::TradingKillSwitch,
                actor: "127.0.0.1".to_owned(),
                target: Some("trading".to_owned()),
                idempotency_key: None,
                message: "accepted".to_owned(),
            },
        )
        .unwrap_or_else(|error| panic!("action run begin failed: {error}"));

        let result: Result<(), AppError> = fail_response(
            &state,
            &run.id,
            AppError::domain(
                axum::http::StatusCode::BAD_REQUEST,
                codes::RISK_BLOCKED,
                "blocked",
            ),
        );

        assert!(result.is_err());
        let stored = state.action_runs().get(&run.id).map(|entry| entry.clone());
        assert_eq!(
            stored.as_ref().map(|item| item.status),
            Some(ActionRunStatus::Failed)
        );
        assert_eq!(
            stored
                .as_ref()
                .and_then(|item| item.problem.as_ref())
                .and_then(|problem| problem.request_id.as_deref()),
            Some("req-action-1")
        );
    })
    .await;
}

#[tokio::test]
async fn recent_returns_newest_first() {
    let state = test_state().await;
    let first = begin(
        &state,
        ActionRunStart {
            kind: ActionRunKind::TradingOrderSubmit,
            actor: "a".to_owned(),
            target: Some("order-a".to_owned()),
            idempotency_key: None,
            message: "first".to_owned(),
        },
    )
    .unwrap_or_else(|error| panic!("first action run begin failed: {error}"));
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let second = begin(
        &state,
        ActionRunStart {
            kind: ActionRunKind::TradingOrderCancel,
            actor: "a".to_owned(),
            target: Some("order-b".to_owned()),
            idempotency_key: None,
            message: "second".to_owned(),
        },
    )
    .unwrap_or_else(|error| panic!("second action run begin failed: {error}"));
    finish_status(&state, &first.id, ActionRunStatus::Succeeded, "done", None)
        .unwrap_or_else(|error| panic!("first action run finish failed: {error}"));
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    finish_status(&state, &second.id, ActionRunStatus::Succeeded, "done", None)
        .unwrap_or_else(|error| panic!("second action run finish failed: {error}"));

    let ids: Vec<String> = recent(&state).into_iter().map(|run| run.id).collect();
    let first_pos = position_of(&ids, &first.id);
    let second_pos = position_of(&ids, &second.id);

    assert!(second_pos <= first_pos);
}

#[tokio::test]
async fn recent_envelope_reports_bounded_history_as_partial() {
    let state = test_state().await;
    for index in 0..=RECENT_ACTION_RUNS {
        begin(
            &state,
            ActionRunStart {
                kind: ActionRunKind::TradingOrderSubmit,
                actor: "audit".to_owned(),
                target: Some(format!("order-{index}")),
                idempotency_key: None,
                message: "accepted".to_owned(),
            },
        )
        .unwrap_or_else(|error| panic!("action run begin failed: {error}"));
    }

    let envelope = recent_envelope(&state);
    let coverage = envelope
        .coverage
        .as_ref()
        .unwrap_or_else(|| panic!("missing action run coverage"));

    assert_eq!(envelope.status, ResourceStatus::Partial);
    assert_eq!(
        envelope.data.as_ref().map(Vec::len),
        Some(RECENT_ACTION_RUNS)
    );
    assert_eq!(coverage.expected, RECENT_ACTION_RUNS as u64 + 1);
    assert_eq!(coverage.observed, RECENT_ACTION_RUNS as u64);
    assert!(coverage.truncated);
    assert_eq!(
        envelope
            .primary_problem()
            .map(|problem| problem.code.as_str()),
        Some("ACTION_RUN_HISTORY_BOUNDED")
    );
}

#[tokio::test]
async fn begin_idempotent_replays_existing_key() {
    let state = test_state().await;
    let start = ActionRunStart {
        kind: ActionRunKind::TradingOrderSubmit,
        actor: "a".to_owned(),
        target: Some("manual-order".to_owned()),
        idempotency_key: Some("client-1".to_owned()),
        message: "accepted".to_owned(),
    };

    let first = begin_idempotent(&state, start.clone())
        .unwrap_or_else(|error| panic!("first idempotent begin failed: {error}"));
    let second = begin_idempotent(&state, start)
        .unwrap_or_else(|error| panic!("second idempotent begin failed: {error}"));

    assert!(!first.is_replayed());
    assert!(second.is_replayed());
    assert_eq!(first.run().id, second.run().id);
    assert_eq!(state.action_runs().len(), 1);
    assert!(
        find_by_idempotency_key(&state, ActionRunKind::TradingOrderSubmit, "client-1").is_some()
    );
}

#[tokio::test]
async fn get_returns_action_run_by_id() {
    let state = test_state().await;
    let run = begin(
        &state,
        ActionRunStart {
            kind: ActionRunKind::TradingOrderSubmit,
            actor: "a".to_owned(),
            target: Some("manual-order".to_owned()),
            idempotency_key: Some("client-1".to_owned()),
            message: "accepted".to_owned(),
        },
    )
    .unwrap_or_else(|error| panic!("action run begin failed: {error}"));

    let stored = get(&state, &run.id);
    let missing = get(&state, "missing");

    assert_eq!(
        stored.as_ref().map(|item| item.id.as_str()),
        Some(run.id.as_str())
    );
    assert!(missing.is_none());
}
