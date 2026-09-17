use super::workflow_view;
use axum::http::StatusCode;
use common::AppError;
use shared_types::{
    problem::codes, ExecutionGuard, HedgePreflightStatus, HedgePreviewResponse, HedgeTicketView,
};

pub(crate) fn apply_final_margin_guard(
    preview: &mut HedgePreviewResponse,
    guard: ExecutionGuard,
) -> Result<(), AppError> {
    let long_venue = preview.long_leg.exchange.clone();
    let short_venue = preview.short_leg.exchange.clone();
    record_final_margin_evidence(
        &mut preview.ticket.guards,
        &mut preview.workflow_view,
        &long_venue,
        &short_venue,
        guard,
    )
}

fn record_final_margin_evidence(
    guards: &mut [ExecutionGuard],
    workflow_view: &mut HedgeTicketView,
    long_venue: &str,
    short_venue: &str,
    guard: ExecutionGuard,
) -> Result<(), AppError> {
    let final_status = guard
        .preflight_outcome
        .as_ref()
        .map(|outcome| outcome.status);
    if guard.key != "margin_balance"
        || !guard.passed
        || !matches!(
            final_status,
            Some(HedgePreflightStatus::Passed | HedgePreflightStatus::Skipped)
        )
    {
        return Err(final_margin_evidence_error(
            "final margin evidence must be a passed or skipped margin_balance guard",
        ));
    }
    let Some(index) = guards.iter().position(|current| current.key == guard.key) else {
        return Err(final_margin_evidence_error(
            "ticket is missing preview-time margin evidence",
        ));
    };
    workflow_view::refresh_margin_evidence(workflow_view, &guard, long_venue, short_venue)
        .map_err(final_margin_evidence_error)?;
    guards[index] = guard;
    Ok(())
}

fn final_margin_evidence_error(message: impl Into<String>) -> AppError {
    AppError::domain(
        StatusCode::CONFLICT,
        codes::HEDGE_TICKET_BLOCKED,
        message.into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{
        HedgeLegRole, HedgePreflightOperation, HedgePreflightScope, HedgeTicketLegView,
        MarginPreflightOutcome, ResourceStatus,
    };

    #[test]
    fn final_margin_evidence_replaces_preview_guard_and_workflow_health() -> anyhow::Result<()> {
        let preview_outcome = MarginPreflightOutcome {
            status: HedgePreflightStatus::Passed,
            checked_at_ms: 1,
            ..MarginPreflightOutcome::default()
        };
        let final_outcome = MarginPreflightOutcome {
            status: HedgePreflightStatus::Passed,
            checked_at_ms: 2,
            scope: HedgePreflightScope {
                venues: vec!["okx".into(), "binance".into()],
                operations: vec![HedgePreflightOperation::MarginBalance],
                ..HedgePreflightScope::default()
            },
            source: Some("account_state.margin_facts".into()),
            request_id: Some("req-confirm-margin".into()),
            ..MarginPreflightOutcome::default()
        };
        let mut guards = vec![ExecutionGuard {
            key: "margin_balance".into(),
            label: "保证金余额".into(),
            passed: true,
            detail: "预览通过".into(),
            preflight_outcome: Some(preview_outcome),
        }];
        let mut workflow = HedgeTicketView {
            long_leg: Some(HedgeTicketLegView {
                role: HedgeLegRole::Long,
                venue: "okx".into(),
                ..HedgeTicketLegView::default()
            }),
            short_leg: Some(HedgeTicketLegView {
                role: HedgeLegRole::Short,
                venue: "binance".into(),
                ..HedgeTicketLegView::default()
            }),
            ..HedgeTicketView::default()
        };
        let final_guard = ExecutionGuard {
            key: "margin_balance".into(),
            label: "保证金余额".into(),
            passed: true,
            detail: "确认终检通过".into(),
            preflight_outcome: Some(final_outcome),
        };

        record_final_margin_evidence(&mut guards, &mut workflow, "okx", "binance", final_guard)?;

        assert_eq!(guards.len(), 1);
        assert_eq!(guards[0].detail, "确认终检通过");
        let long = workflow
            .long_leg
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("final margin workflow lost the long leg"))?;
        assert_eq!(long.balance.status, ResourceStatus::Ready);
        assert_eq!(long.balance.observed_at_ms, Some(2));
        assert_eq!(
            long.balance.request_id.as_deref(),
            Some("req-confirm-margin")
        );
        Ok(())
    }
}
