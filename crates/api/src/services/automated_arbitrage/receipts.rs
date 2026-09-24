use crate::state::AppState;
use common::AppError;
use shared_types::AutomationExecutionReceipt;

pub(crate) fn execution_receipt(
    state: &AppState,
    run_id: &str,
) -> Result<AutomationExecutionReceipt, AppError> {
    let run = state
        .execution_runs()
        .get(run_id)
        .map(|row| row.value().clone())
        .ok_or_else(|| AppError::NotFound(format!("execution-run: {run_id}")))?;
    let mut close_runs = state
        .close_runs()
        .iter()
        .filter(|row| AutomationExecutionReceipt::matches_close(&run, row.value()))
        .map(|row| row.value().clone())
        .collect::<Vec<_>>();
    close_runs.sort_by(|a, b| {
        b.updated_at_ms
            .cmp(&a.updated_at_ms)
            .then_with(|| a.id.cmp(&b.id))
    });
    let close_run_total = close_runs.len();
    close_runs.truncate(32);
    Ok(AutomationExecutionReceipt {
        mode: recorded_mode(state, &run),
        run,
        close_runs,
        close_run_total,
        observed_at_ms: common::time::now_ms(),
    })
}

fn recorded_mode(
    state: &AppState,
    run: &shared_types::ExecutionRun,
) -> Option<shared_types::ExecutionMode> {
    let order_mode = |leg: &shared_types::ExecutionRunLeg| {
        let identity = leg.identity.as_ref()?;
        let order = state
            .trading_service()
            .get_order(&identity.internal_order_id)?;
        // Compiled orders use native symbols; the run keeps the canonical symbol.
        let recorded_identity = order.identity_snapshot();
        (shared_types::venue_names_equal(&order.intent.exchange, &leg.exchange)
            && recorded_identity.internal_order_id == identity.internal_order_id
            && recorded_identity.public_client_order_id == identity.public_client_order_id
            && recorded_identity.product == identity.product)
        .then_some(order.intent.mode)
    };
    let long = order_mode(&run.long_leg)?;
    let short = order_mode(&run.short_leg)?;
    if long != short {
        return None;
    }
    Some(long)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use shared_types::{CloseRun, ExecutionRun};

    #[tokio::test]
    async fn receipt_joins_exact_run_ticket_and_opportunity_without_external_reads(
    ) -> anyhow::Result<()> {
        use axum::{
            body::{to_bytes, Body},
            http::{Request, StatusCode},
        };
        use tower::ServiceExt;
        let mut config = common::config::AppConfig::default();
        config.security.auth_token = Some("offline-automation-receipt-test".into());
        config.history.enabled = false;
        let state = AppState::new(config).await?;
        assert!(matches!(
            execution_receipt(&state, "missing"),
            Err(AppError::NotFound(_))
        ));
        let leg = |role| {
            json!({"role":role,"exchange":"fixture","symbol":"SOL","orderIds":[],
            "state":"filled","targetQuantity":1.0,"filledQuantity":1.0,"targetNotionalUsd":10.0})
        };
        let run: ExecutionRun = serde_json::from_value(
            json!({"runId":"run-a","ticketId":"ticket-a","opportunityId":"opp-a",
            "state":"hedged","longLeg":leg("long"),"shortLeg":leg("short"),"netExposureUsd":0.0,
            "statusReason":"fixture","createdAtMs":1,"updatedAtMs":2}),
        )?;
        state.execution_runs().insert(run.run_id.clone(), run);
        let pair = json!({"source":"execution_run","runId":"run-a","ticketId":"ticket-a","opportunityId":"opp-a",
            "venue":"fixture","symbol":"SOL","side":"long","partnerVenue":"fixture-b","partnerSymbol":"SOL",
            "partnerSide":"short","legFilledQuantity":1.0,"partnerFilledQuantity":1.0,"matchedNotionalUsd":10.0,"updatedAtMs":3});
        let template = json!({"id":"close-a","scope":"pair","status":"submitted","snapshotVersion":"fixture",
            "expectedLegCount":2,"legs":[{"venue":"fixture","symbol":"SOL","side":"long","status":"accepted",
                "quantity":1.0,"markPrice":10.0,"notionalUsd":10.0,"pairEvidence":pair}],
            "submittedOrderCount":1,"failedLegCount":0,"nakedExposureUsd":0.0,"message":"fixture","startedAtMs":3,"updatedAtMs":4});
        for (index, field) in [None, Some("runId"), Some("ticketId"), Some("opportunityId")]
            .into_iter()
            .enumerate()
        {
            let mut value = template.clone();
            value["id"] = json!(format!("close-{index}"));
            if let Some(field) = field {
                value["legs"][0]["pairEvidence"][field] = json!("unrelated");
            }
            let close: CloseRun = serde_json::from_value(value)?;
            state.close_runs().insert(close.id.clone(), close);
        }
        let receipt = execution_receipt(&state, "run-a")?;
        assert_eq!(receipt.close_run_total, 1);
        assert_eq!(receipt.close_runs[0].id, "close-0");
        assert_eq!(receipt.run.run_id, "run-a");
        assert!(receipt.observed_at_ms > 0);
        let event = shared_types::ExecutionRunEvent {
            event: "close_run_updated".into(),
            execution_run: None,
            close_run: Some(receipt.close_runs[0].clone()),
            timestamp_ms: 4,
        };
        let decoded: shared_types::ExecutionRunEvent =
            serde_json::from_value(serde_json::to_value(event)?)?;
        assert_eq!(
            decoded.close_run.as_ref().map(|run| run.id.as_str()),
            Some("close-0")
        );
        let router = crate::app::build_router(state.clone());
        let request = |id, authenticated| {
            let builder = Request::builder().uri(format!("/api/automation/execution-runs/{id}"));
            let builder = if authenticated {
                builder.header("Authorization", "Bearer offline-automation-receipt-test")
            } else {
                builder
            };
            builder.body(Body::empty()).unwrap()
        };
        assert_eq!(
            router
                .clone()
                .oneshot(request("run-a", false))
                .await?
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            router
                .clone()
                .oneshot(request("missing", true))
                .await?
                .status(),
            StatusCode::NOT_FOUND
        );
        let response = router.oneshot(request("run-a", true)).await?;
        assert_eq!(response.status(), StatusCode::OK);
        let result: AutomationExecutionReceipt =
            serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await?)?;
        assert_eq!(result.run.run_id, "run-a");
        assert_eq!(result.close_runs.len(), 1);
        Ok(())
    }
}
