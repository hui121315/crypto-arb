use super::super::*;
use super::*;

#[tokio::test]
async fn confirm_missing_preview_terminalizes_action_run_and_preserves_identity(
) -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    let mut headers = HeaderMap::new();
    crate::middleware::audit::insert_verified_bearer_actor(&mut headers, "hedge-test-token", None);
    let actor = crate::middleware::audit::extract_actor(&headers);
    let request = HedgeConfirmRequest {
        idempotency_key: "missing-preview-key".to_owned(),
        ticket_id: Some("missing-ticket".to_owned()),
    };

    let first = common::request_id::scope("req-missing-preview".to_owned(), async {
        Box::pin(confirm_hedge(
            State(state.clone()),
            headers.clone(),
            Path("opp-missing".to_owned()),
            Json(request.clone()),
        ))
        .await
    })
    .await;
    let error = first
        .err()
        .ok_or_else(|| anyhow::anyhow!("missing preview succeeded"))?;
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
    assert_eq!(error.code(), codes::HEDGE_PREVIEW_NOT_FOUND);
    let error_context = match &error {
        AppError::Domain { details, .. } => details
            .as_ref()
            .and_then(|details| details.get("confirmContext")),
        _ => None,
    };
    assert_eq!(
        error_context.and_then(|context| context.get("opportunityId")),
        Some(&serde_json::json!("opp-missing"))
    );
    assert_eq!(
        error_context.and_then(|context| context.get("ticketId")),
        Some(&serde_json::json!("missing-ticket"))
    );
    assert_eq!(
        error_context.and_then(|context| context.get("idempotencyKey")),
        Some(&serde_json::json!("missing-preview-key"))
    );

    let runs: Vec<ActionRun> = action_runs::recent(&state)
        .into_iter()
        .filter(|run| run.kind == ActionRunKind::HedgeConfirm)
        .collect();
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert_eq!(run.status, ActionRunStatus::Failed);
    assert_eq!(run.request_id.as_deref(), Some("req-missing-preview"));
    assert_eq!(run.idempotency_key.as_deref(), Some("missing-preview-key"));
    assert_eq!(run.actor, actor);
    assert_eq!(
        run.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::HEDGE_PREVIEW_NOT_FOUND)
    );
    assert_eq!(
        run.problem
            .as_ref()
            .and_then(|problem| problem.details.as_ref())
            .and_then(|details| details.get("confirmContext"))
            .and_then(|context| context.get("ticketId")),
        Some(&serde_json::json!("missing-ticket"))
    );

    let replay = Box::pin(confirm_hedge(
        State(state.clone()),
        headers,
        Path("opp-missing".to_owned()),
        Json(request),
    ))
    .await;
    let replay_error = replay
        .err()
        .ok_or_else(|| anyhow::anyhow!("failed preview replay succeeded"))?;
    assert_eq!(replay_error.code(), codes::HEDGE_PREVIEW_NOT_FOUND);
    assert_eq!(
        action_runs::recent(&state)
            .into_iter()
            .filter(|run| run.kind == ActionRunKind::HedgeConfirm)
            .count(),
        1
    );
    Ok(())
}
