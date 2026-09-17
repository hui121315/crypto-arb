use super::*;
use crate::services::close_run_store::ProjectionAppend;
use trading::CLOSE_RUN_PROJECTOR;

pub(crate) fn project_order_update(state: &AppState, record: &OrderRecord) -> Vec<CloseRun> {
    let mut updated = Vec::new();
    for mut entry in state.close_runs().iter_mut() {
        let run = entry.value_mut();
        if apply_order_update(run, record) {
            run.updated_at_ms = common::time::now_ms();
            refresh_action_run_payload(state, run);
            state.close_run_store().append(run);
            append_close_run_finality_from_order(state, run, record);
            updated.push(run.clone());
        }
    }
    updated
}

pub(crate) fn project_finality_problem(
    state: &AppState,
    order_id: &str,
    problem: &ApiProblem,
    checked_at_ms: i64,
) -> Vec<CloseRun> {
    let mut updated = Vec::new();
    for mut entry in state.close_runs().iter_mut() {
        let run = entry.value_mut();
        if apply_finality_problem(run, order_id, problem, checked_at_ms) {
            refresh_action_run_payload(state, run);
            state.close_run_store().append(run);
            append_close_run_finality(
                state,
                run,
                OrderUpdateSource::OrderQuery,
                None,
                Some(order_id),
                checked_at_ms,
            );
            updated.push(run.clone());
        }
    }
    updated
}

pub(crate) fn project_ledger_event_update(
    state: &AppState,
    event: &ExecutionLedgerEvent,
) -> Vec<CloseRun> {
    match project_ledger_event_update_durable(state, event) {
        Ok(updated) => updated,
        Err(error) => {
            tracing::warn!(
                event_id = %event.event_id,
                %error,
                "failed to durably project close run ledger event"
            );
            Vec::new()
        }
    }
}

pub(crate) fn project_ledger_event_update_durable(
    state: &AppState,
    event: &ExecutionLedgerEvent,
) -> std::io::Result<Vec<CloseRun>> {
    let store = state.close_run_store();
    let _projection_guard = store.lock_projection();
    if store.has_projection_receipt(CLOSE_RUN_PROJECTOR, &event.event_id) {
        return Ok(Vec::new());
    }
    let update = CloseLedgerUpdate::from_event(event);
    let mut updated = Vec::new();
    for entry in state.close_runs().iter() {
        let mut projected = entry.value().clone();
        if apply_close_ledger_event(&mut projected, event, update.as_ref()) {
            projected.updated_at_ms = common::time::now_ms();
            updated.push(projected);
        }
    }
    if updated.is_empty() {
        return Ok(Vec::new());
    }
    let append = store.append_projected(&updated, CLOSE_RUN_PROJECTOR, &event.event_id)?;
    if append == ProjectionAppend::AlreadyReceived {
        return Ok(Vec::new());
    }
    for run in &updated {
        state.close_runs().insert(run.id.clone(), run.clone());
        refresh_action_run_payload(state, run);
        append_close_run_finality_from_ledger_event(state, run, event);
    }
    Ok(updated)
}

pub(in crate::services::close_runs) fn append_close_run_finality_from_order(
    state: &AppState,
    run: &CloseRun,
    record: &OrderRecord,
) {
    append_close_run_finality(
        state,
        run,
        record.last_update_source,
        None,
        Some(record.intent.id.as_str()),
        record.updated_at_ms,
    );
}

pub(in crate::services::close_runs) fn append_close_run_finality_from_ledger_event(
    state: &AppState,
    run: &CloseRun,
    event: &ExecutionLedgerEvent,
) {
    append_close_run_finality(
        state,
        run,
        event.source,
        Some(event.event_id.as_str()),
        Some(event.order.identity.internal_order_id.as_str()),
        event.occurred_at_ms,
    );
}

pub(in crate::services::close_runs) fn append_close_run_finality(
    state: &AppState,
    run: &CloseRun,
    source: OrderUpdateSource,
    source_event_id: Option<&str>,
    source_order_event_id: Option<&str>,
    occurred_at_ms: i64,
) {
    match encode_close_run_finality(
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
            close_run_id = %run.id,
            error = %error,
            "failed to encode close run finality SQL event"
        ),
    }
}

pub(super) fn encode_close_run_finality(
    run: &CloseRun,
    source: OrderUpdateSource,
    source_event_id: Option<&str>,
    source_order_event_id: Option<&str>,
    occurred_at_ms: i64,
) -> Result<trading::SqlRunFinalityLedgerEvent, String> {
    trading::SqlRunFinalityLedgerEvent::from_close_run(
        run,
        source,
        source_event_id,
        source_order_event_id,
        occurred_at_ms,
    )
}
