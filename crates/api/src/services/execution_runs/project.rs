use super::*;
use crate::services::execution_run_store::ProjectionAppend;
use trading::EXECUTION_RUN_PROJECTOR;

pub(crate) fn project_order_update(state: &AppState, record: &OrderRecord) -> Vec<ExecutionRun> {
    if record.intent.source != OrderSource::ArbitragePreview {
        return Vec::new();
    }
    let mut updated = Vec::new();
    for mut entry in state.execution_runs().iter_mut() {
        let run = entry.value_mut();
        if apply_order_update(run, record) {
            run.updated_at_ms = common::time::now_ms();
            append_order_update_evidence(run, record);
            refresh_workflow_view(run);
            state.execution_run_store().append(run);
            append_run_finality_from_order(state, run, record);
            updated.push(run.clone());
        }
    }
    updated
}

pub(crate) fn project_unsubmitted_leg_failure(
    state: &AppState,
    order_id: &str,
    checked_at_ms: i64,
) -> Vec<(ExecutionRun, String)> {
    let mut updated = Vec::new();
    for mut entry in state.execution_runs().iter_mut() {
        let run = entry.value_mut();
        let Some(venue) = mark_unsubmitted_leg_failed(run, order_id) else {
            continue;
        };
        run.finality_checked_at_ms = Some(
            run.finality_checked_at_ms
                .unwrap_or_default()
                .max(checked_at_ms),
        );
        run.updated_at_ms = run.updated_at_ms.max(checked_at_ms);
        refresh_workflow_view(run);
        state.execution_run_store().append(run);
        append_run_finality(
            state,
            run,
            OrderUpdateSource::Internal,
            None,
            Some(order_id),
            checked_at_ms,
        );
        updated.push((run.clone(), venue));
    }
    updated
}

fn mark_unsubmitted_leg_failed(run: &mut ExecutionRun, order_id: &str) -> Option<String> {
    if !matches!(
        run.state,
        ExecutionRunState::FirstLegPartial
            | ExecutionRunState::UnwindRequired
            | ExecutionRunState::Unwinding
            | ExecutionRunState::FailedSafe
            | ExecutionRunState::Closed
    ) {
        return None;
    }
    for leg in [&mut run.long_leg, &mut run.short_leg] {
        if leg.state == LiveOrderState::Created && leg.order_ids.iter().any(|id| id == order_id) {
            leg.state = LiveOrderState::Failed;
            leg.finality_source = Some(OrderUpdateSource::Internal);
            return Some(leg.exchange.clone());
        }
    }
    None
}

pub(crate) fn project_ledger_event_update(
    state: &AppState,
    event: &ExecutionLedgerEvent,
) -> Vec<ExecutionRun> {
    match project_ledger_event_update_durable(state, event) {
        Ok(updated) => updated,
        Err(error) => {
            tracing::warn!(
                event_id = %event.event_id,
                %error,
                "failed to durably project execution run ledger event"
            );
            Vec::new()
        }
    }
}

pub(crate) fn project_ledger_event_update_durable(
    state: &AppState,
    event: &ExecutionLedgerEvent,
) -> std::io::Result<Vec<ExecutionRun>> {
    let store = state.execution_run_store();
    let _projection_guard = store.lock_projection();
    if store.has_projection_receipt(EXECUTION_RUN_PROJECTOR, &event.event_id) {
        return Ok(Vec::new());
    }
    let mut updated = Vec::new();
    for entry in state.execution_runs().iter() {
        let mut projected = entry.value().clone();
        if apply_ledger_event_update(&mut projected, event) {
            projected.updated_at_ms = common::time::now_ms();
            append_ledger_event_evidence(&mut projected, event);
            refresh_workflow_view(&mut projected);
            updated.push(projected);
        }
    }
    if updated.is_empty() {
        return Ok(Vec::new());
    }
    let append = store.append_projected(&updated, EXECUTION_RUN_PROJECTOR, &event.event_id)?;
    if append == ProjectionAppend::AlreadyReceived {
        return Ok(Vec::new());
    }
    for run in &updated {
        state
            .execution_runs()
            .insert(run.run_id.clone(), run.clone());
        append_run_finality_from_ledger_event(state, run, event);
    }
    Ok(updated)
}

pub(super) fn append_run_finality_from_order(
    state: &AppState,
    run: &ExecutionRun,
    record: &OrderRecord,
) {
    append_run_finality(
        state,
        run,
        record.last_update_source,
        None,
        Some(record.intent.id.as_str()),
        record.updated_at_ms,
    );
}

pub(super) fn append_run_finality_from_ledger_event(
    state: &AppState,
    run: &ExecutionRun,
    event: &ExecutionLedgerEvent,
) {
    append_run_finality(
        state,
        run,
        event.source,
        Some(event.event_id.as_str()),
        Some(event.order.identity.internal_order_id.as_str()),
        event.occurred_at_ms,
    );
}

pub(super) fn append_run_finality(
    state: &AppState,
    run: &ExecutionRun,
    source: OrderUpdateSource,
    source_event_id: Option<&str>,
    source_order_event_id: Option<&str>,
    occurred_at_ms: i64,
) {
    match trading::SqlRunFinalityLedgerEvent::from_execution_run(
        run,
        source,
        source_event_id,
        source_order_event_id,
        occurred_at_ms,
    ) {
        Ok(event) => {
            state.trading_service().append_run_finality_event(event);
        }
        Err(error) => tracing::warn!(
            run_id = %run.run_id,
            error = %error,
            "failed to encode execution run finality SQL event"
        ),
    }
}
