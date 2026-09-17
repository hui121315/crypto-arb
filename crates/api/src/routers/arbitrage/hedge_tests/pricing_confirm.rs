use super::super::*;
use super::*;

#[test]
fn preview_short_price_does_not_fallback_to_long_price() {
    let mut opp = blocked_opportunity();
    opp.long_price = Some(200.0);
    opp.short_price = None;
    let mut ticket = ticket_with(Vec::new(), true);
    ticket.short_leg.reference_price = None;
    ticket.short_leg.open_vwap_price = None;
    let req = HedgePreviewRequest {
        opportunity_id: opp.id.clone(),
        opportunity_snapshot_id: None,
        capital_usd: 500.0,
        leverage: 1.0,
        long_price: None,
        short_price: None,
        long_notional_usd: None,
        short_notional_usd: None,
        execution_params: None,
    };

    let detail = preview_short_price(&req, &ticket, &opp)
        .err()
        .map(|err| err.to_string())
        .unwrap_or_default();

    assert!(detail.contains("shortPrice"));
}

#[test]
fn preview_missing_long_price_is_hard_rejected() {
    let mut opp = blocked_opportunity();
    opp.long_price = None;
    opp.short_price = Some(201.0);
    let mut ticket = ticket_with(Vec::new(), true);
    ticket.long_leg.reference_price = None;
    ticket.long_leg.open_vwap_price = None;
    let req = HedgePreviewRequest {
        opportunity_id: opp.id.clone(),
        opportunity_snapshot_id: None,
        capital_usd: 500.0,
        leverage: 1.0,
        long_price: None,
        short_price: None,
        long_notional_usd: None,
        short_notional_usd: None,
        execution_params: None,
    };

    let detail = preview_long_price(&req, &ticket, &opp)
        .err()
        .map(|err| err.to_string())
        .unwrap_or_default();

    assert!(detail.contains("longPrice"));
}

#[test]
fn ticket_blocked_maps_to_domain_code_and_status() {
    let ticket = ticket_with(vec!["深度不足".into()], true);
    let result = validate_ticket_ready(&ticket);
    assert!(result.is_err());
    if let Err(err) = result {
        assert_eq!(err.code(), "HEDGE_TICKET_BLOCKED");
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
    }
}

#[test]
fn confirm_action_status_succeeds_only_after_second_leg_submission_or_hedged() {
    let mut response =
        confirm_response_with_run(shared_types::ExecutionRunState::SecondLegSubmitted);
    assert_eq!(confirm_action_status(&response), ActionRunStatus::Succeeded);

    response.execution_run = Some(execution_run_with_state(
        shared_types::ExecutionRunState::Hedged,
    ));
    assert_eq!(confirm_action_status(&response), ActionRunStatus::Succeeded);
}

#[test]
fn confirm_action_status_fails_for_unwind_or_error_response() {
    let mut response = confirm_response_with_run(shared_types::ExecutionRunState::Unwinding);
    assert_eq!(confirm_action_status(&response), ActionRunStatus::Failed);

    response.execution_run = Some(execution_run_with_state(
        shared_types::ExecutionRunState::SecondLegSubmitted,
    ));
    response.error = Some("second leg failed".into());

    assert_eq!(confirm_action_status(&response), ActionRunStatus::Failed);
    let problem = confirm_action_problem(&response);
    assert_eq!(
        problem.as_ref().map(|item| item.code.as_str()),
        Some(codes::HEDGE_CONFIRM_NOT_HEDGED)
    );
}

#[test]
fn confirm_action_problem_prefers_typed_unwind_problem() {
    let mut response = confirm_response_with_run(shared_types::ExecutionRunState::UnwindRequired);
    response.error = Some("fallback string".into());
    response.problem = Some(shared_types::ApiProblem::new(
        codes::HEDGE_UNWIND_SUBMIT_FAILED,
        "typed unwind failure",
    ));

    let problem = confirm_action_problem(&response);

    assert_eq!(
        problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::HEDGE_UNWIND_SUBMIT_FAILED)
    );
    assert_eq!(
        problem.as_ref().map(|problem| problem.message.as_str()),
        Some("typed unwind failure")
    );
}

#[tokio::test]
async fn confirm_finish_stores_payload_for_replay_without_hot_run() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    let action_run = action_runs::begin(
        &state,
        action_runs::ActionRunStart {
            kind: ActionRunKind::HedgeConfirm,
            actor: "test".to_owned(),
            target: Some("opp-test".to_owned()),
            idempotency_key: Some("hedge-test".to_owned()),
            message: "accepted".to_owned(),
        },
    )?;
    let response = confirm_response_with_run(shared_types::ExecutionRunState::SecondLegSubmitted);

    let returned = finish_confirm_action(&state, &action_run, response)?;
    let stored = action_runs::get(&state, &action_run.id)
        .ok_or_else(|| anyhow::anyhow!("hedge action run missing"))?;
    let replayed = replay_confirm_response(&state, &stored, "hedge-test")?;

    assert_eq!(returned.status, shared_types::HedgeConfirmStatus::Submitted);
    assert_eq!(stored.status, ActionRunStatus::Succeeded);
    assert!(stored.result.is_some());
    assert_eq!(
        replayed
            .execution_run
            .as_ref()
            .map(|run| run.run_id.as_str()),
        Some("run-hedge-test")
    );
    Ok(())
}

#[tokio::test]
async fn confirm_replay_failed_action_without_run_replays_original_problem() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    let action_run = action_runs::begin(
        &state,
        action_runs::ActionRunStart {
            kind: ActionRunKind::HedgeConfirm,
            actor: "test".to_owned(),
            target: Some("opp-1".to_owned()),
            idempotency_key: Some("hedge-missing".to_owned()),
            message: "accepted".to_owned(),
        },
    )?;
    action_runs::finish_status(
        &state,
        &action_run.id,
        ActionRunStatus::Failed,
        "pre trade failed",
        Some(
            shared_types::ApiProblem::new(codes::HEDGE_PRE_TRADE_REJECTED, "pre trade blocked")
                .with_status(StatusCode::BAD_REQUEST.as_u16()),
        ),
    )?;
    let stored = action_runs::get(&state, &action_run.id)
        .ok_or_else(|| anyhow::anyhow!("hedge action run missing"))?;

    let error = replay_confirm_response(&state, &stored, "hedge-missing")
        .err()
        .ok_or_else(|| {
            anyhow::anyhow!("failed hedge action without run must not replay success")
        })?;

    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error.code(), codes::HEDGE_PRE_TRADE_REJECTED);
    Ok(())
}

#[tokio::test]
async fn confirm_replay_succeeded_action_without_run_is_unavailable() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    let action_run = action_runs::begin(
        &state,
        action_runs::ActionRunStart {
            kind: ActionRunKind::HedgeConfirm,
            actor: "test".to_owned(),
            target: Some("opp-1".to_owned()),
            idempotency_key: Some("hedge-missing".to_owned()),
            message: "accepted".to_owned(),
        },
    )?;
    action_runs::finish_status(
        &state,
        &action_run.id,
        ActionRunStatus::Succeeded,
        "submitted",
        None,
    )?;
    let stored = action_runs::get(&state, &action_run.id)
        .ok_or_else(|| anyhow::anyhow!("hedge action run missing"))?;

    let error = replay_confirm_response(&state, &stored, "hedge-missing")
        .err()
        .ok_or_else(|| anyhow::anyhow!("missing execution run must fail replay"))?;

    assert_eq!(error.status(), StatusCode::CONFLICT);
    assert_eq!(error.code(), codes::ACTION_RUN_REPLAY_UNAVAILABLE);
    Ok(())
}
