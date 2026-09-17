use super::*;

mod events;
mod manual_terminal;

pub(super) use events::{append_close_run_finality, append_close_run_finality_from_order};
pub(crate) use events::{
    project_finality_problem, project_ledger_event_update, project_ledger_event_update_durable,
    project_order_update,
};
pub(crate) use manual_terminal::record_manual_terminal_evidence;
#[cfg(test)]
pub(super) use manual_terminal::record_manual_terminal_evidence_with_persist;

pub(crate) fn record(state: &AppState, mut run: CloseRun) -> CloseRun {
    replay_existing_order_ledger(state, &mut run);
    project_embedded_order_records(&mut run);
    refresh_run_summary(&mut run);
    state.close_run_store().append(&run);
    append_close_run_finality(
        state,
        &run,
        OrderUpdateSource::Internal,
        None,
        None,
        run.updated_at_ms,
    );
    state.close_runs().insert(run.id.clone(), run.clone());
    publish_immediate_execution_close(state, &run);
    run
}

fn project_embedded_order_records(run: &mut CloseRun) {
    let records = run
        .legs
        .iter()
        .filter(|leg| {
            matches!(
                leg.status,
                CloseLegStatus::Submitted | CloseLegStatus::Accepted
            )
        })
        .filter_map(|leg| leg.order.clone())
        .collect::<Vec<_>>();
    for record in records {
        apply_order_update(run, &record);
    }
}

fn publish_immediate_execution_close(state: &AppState, run: &CloseRun) {
    for execution_run in crate::services::execution_runs::project_close_run_update(state, run) {
        if let Err(error) = crate::services::ws_publish::publish_execution_run_event(
            state,
            "execution_run_closed",
            &execution_run,
        ) {
            tracing::warn!(
                close_run_id = %run.id,
                execution_run_id = %execution_run.run_id,
                %error,
                "failed to publish immediate execution close projection"
            );
        }
    }
}

fn replay_existing_order_ledger(state: &AppState, run: &mut CloseRun) {
    let order_ids = run
        .legs
        .iter()
        .filter_map(|leg| leg.order.as_ref())
        .map(|order| order.intent.id.clone())
        .collect::<Vec<_>>();
    for order_id in order_ids {
        let mut events = state
            .trading_service()
            .list_execution_ledger_events_by_query(&trading::ExecutionLedgerQuery {
                internal_order_id: Some(order_id),
                exchange_order_id: None,
                hedge_group_id: None,
                run_id: None,
                ticket_id: None,
                leg_role: None,
                from_ms: None,
                to_ms: None,
                limit: 32,
            });
        events.sort_by(|left, right| {
            left.occurred_at_ms
                .cmp(&right.occurred_at_ms)
                .then_with(|| close_replay_rank(left).cmp(&close_replay_rank(right)))
                .then_with(|| left.event_id.cmp(&right.event_id))
        });
        for event in events {
            let update = CloseLedgerUpdate::from_event(&event);
            let _ = apply_close_ledger_event(run, &event, update.as_ref());
        }
    }
}

fn close_replay_rank(event: &ExecutionLedgerEvent) -> u8 {
    match &event.payload {
        ExecutionLedgerPayload::OrderState {
            state: LiveOrderState::Cancelled | LiveOrderState::Rejected | LiveOrderState::Failed,
            ..
        } => 2,
        ExecutionLedgerPayload::OrderState { .. } => 0,
        ExecutionLedgerPayload::FillSnapshot(_) => 1,
        ExecutionLedgerPayload::FeeSnapshot(_)
        | ExecutionLedgerPayload::FundingPayment(_)
        | ExecutionLedgerPayload::Slippage(_)
        | ExecutionLedgerPayload::OrderbookEvidence(_) => 3,
    }
}

pub(crate) async fn submit_compensation_order(
    state: &AppState,
    close_run_id: &str,
    request: &CloseRunCompensationRequest,
    action_run: &ActionRun,
) -> Result<CloseRun, AppError> {
    let plan = compensation_submit_plan(state, close_run_id, request, action_run)?;
    validate_compensation_submit_runtime(state, &plan.intent).await?;
    let order = state
        .trading_service()
        .submit(plan.intent)
        .await
        .map_err(crate::trading_errors::map_trading_error)?;
    record_compensation_order_for_candidate(
        state,
        &plan.close_run_id,
        plan.candidate_index,
        &order,
        Some(action_run.id.clone()),
    )
}
