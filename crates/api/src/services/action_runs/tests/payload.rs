use super::*;
use shared_types::problem::codes;

#[tokio::test]
async fn finish_result_with_payload_stores_replay_payload() {
    let state = test_state().await;
    let run = begin(
        &state,
        ActionRunStart {
            kind: ActionRunKind::VenueCredentialsUpdate,
            actor: "a".to_owned(),
            target: Some("okx".to_owned()),
            idempotency_key: Some("cred-1".to_owned()),
            message: "accepted".to_owned(),
        },
    )
    .unwrap_or_else(|error| panic!("action run begin failed: {error}"));
    let payload = json!({
        "venue": "okx",
        "configuredCount": 3,
    });

    if let Err(error) = finish_result_with_payload(&state, &run.id, Ok(payload.clone()), "saved") {
        panic!("store payload failed: {error}");
    }
    let Some(stored) = state
        .action_runs()
        .get(&run.id)
        .map(|entry| entry.value().clone())
    else {
        panic!("stored action run missing");
    };
    let replayed = replay_payload::<serde_json::Value>(&stored)
        .unwrap_or_else(|error| panic!("replay payload missing: {error}"));

    assert_eq!(replayed, payload);
    assert_eq!(stored.status, ActionRunStatus::Succeeded);
}

#[tokio::test]
async fn finish_status_with_payload_stores_failed_business_payload() {
    let state = test_state().await;
    let run = begin(
        &state,
        ActionRunStart {
            kind: ActionRunKind::PortfolioClosePair,
            actor: "a".to_owned(),
            target: Some("binance:BTCUSDT".to_owned()),
            idempotency_key: Some("close-1".to_owned()),
            message: "accepted".to_owned(),
        },
    )
    .unwrap_or_else(|error| panic!("action run begin failed: {error}"));
    let payload = json!({
        "status": "partially_submitted",
        "submittedOrderCount": 1,
        "failedLegCount": 1,
    });

    finish_status_with_payload(
        &state,
        &run.id,
        ActionRunStatus::Failed,
        "partial close",
        Some(ApiProblem::new(codes::CLOSE_RUN_PARTIAL, "partial close")),
        &payload,
    )
    .unwrap_or_else(|error| panic!("store status payload failed: {error}"));
    let Some(stored) = state
        .action_runs()
        .get(&run.id)
        .map(|entry| entry.value().clone())
    else {
        panic!("stored action run missing");
    };
    let replayed = replay_payload::<serde_json::Value>(&stored)
        .unwrap_or_else(|error| panic!("replay payload missing: {error}"));

    assert_eq!(replayed, payload);
    assert_eq!(stored.status, ActionRunStatus::Failed);
}

#[tokio::test]
async fn replay_payload_reports_missing_payload_problem() {
    let state = test_state().await;
    let run = begin(
        &state,
        ActionRunStart {
            kind: ActionRunKind::TradingOrderSubmit,
            actor: "a".to_owned(),
            target: Some("manual-order".to_owned()),
            idempotency_key: Some("submit-1".to_owned()),
            message: "accepted".to_owned(),
        },
    )
    .unwrap_or_else(|error| panic!("action run begin failed: {error}"));

    let error = replay_payload::<serde_json::Value>(&run)
        .err()
        .unwrap_or_else(|| panic!("missing payload should fail"));

    assert_eq!(error.code(), codes::ACTION_RUN_REPLAY_UNAVAILABLE);
    assert!(error.to_string().contains("replay payload"));
    assert_replay_reason(&error, "missing_result");
}

#[tokio::test]
async fn payload_encode_failure_marks_action_run_failed() {
    let state = test_state().await;
    let run = begin(
        &state,
        ActionRunStart {
            kind: ActionRunKind::TradingOrderSubmit,
            actor: "a".to_owned(),
            target: Some("manual-order".to_owned()),
            idempotency_key: Some("submit-1".to_owned()),
            message: "accepted".to_owned(),
        },
    )
    .unwrap_or_else(|error| panic!("action run begin failed: {error}"));

    let result = finish_result_with_payload(&state, &run.id, Ok(FailingPayload), "done");

    assert!(result.is_err());
    let stored = state
        .action_runs()
        .get(&run.id)
        .map(|entry| entry.value().clone())
        .unwrap_or_else(|| panic!("stored action run missing"));
    assert_eq!(stored.status, ActionRunStatus::Failed);
    let problem = stored
        .problem
        .as_ref()
        .unwrap_or_else(|| panic!("encode problem missing"));
    assert_eq!(problem.code, codes::ACTION_RUN_REPLAY_UNAVAILABLE);
    let details = problem
        .details
        .as_ref()
        .unwrap_or_else(|| panic!("encode problem details missing"));
    assert_eq!(
        details.get("reason").and_then(|value| value.as_str()),
        Some("encode_failed")
    );
}

#[tokio::test]
async fn status_payload_encode_failure_marks_action_run_failed() {
    let state = test_state().await;
    let run = begin(
        &state,
        ActionRunStart {
            kind: ActionRunKind::PortfolioClosePair,
            actor: "a".to_owned(),
            target: Some("manual-close".to_owned()),
            idempotency_key: Some("close-1".to_owned()),
            message: "accepted".to_owned(),
        },
    )
    .unwrap_or_else(|error| panic!("action run begin failed: {error}"));

    let result = finish_status_with_payload(
        &state,
        &run.id,
        ActionRunStatus::Succeeded,
        "done",
        None,
        &FailingPayload,
    );

    assert!(result.is_err());
    let stored = state
        .action_runs()
        .get(&run.id)
        .map(|entry| entry.value().clone())
        .unwrap_or_else(|| panic!("stored action run missing"));
    assert_eq!(stored.status, ActionRunStatus::Failed);
    let problem = stored
        .problem
        .as_ref()
        .unwrap_or_else(|| panic!("status encode problem missing"));
    assert_eq!(problem.code, codes::ACTION_RUN_REPLAY_UNAVAILABLE);
}

struct FailingPayload;

impl Serialize for FailingPayload {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        Err(serde::ser::Error::custom("payload refused serialization"))
    }
}
