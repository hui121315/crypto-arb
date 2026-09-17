use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ExecutionRunQuery {
    pub(crate) run_id: Option<String>,
    pub(crate) ticket_id: Option<String>,
    pub(crate) opportunity_id: Option<String>,
}

pub(crate) fn record(state: &AppState, mut run: ExecutionRun) -> ExecutionRun {
    merge_existing_evidence(state, &mut run);
    append_internal_transition(&mut run);
    refresh_workflow_view(&mut run);
    state.execution_run_store().append(&run);
    append_run_finality(
        state,
        &run,
        OrderUpdateSource::Internal,
        None,
        None,
        run.updated_at_ms,
    );
    state
        .execution_runs()
        .insert(run.run_id.clone(), run.clone());
    run
}

pub(crate) fn recent(state: &AppState) -> Vec<ExecutionRun> {
    let mut rows = recent_rows(state);
    sort_recent(&mut rows);
    rows.truncate(RECENT_RUN_LIMIT);
    rows
}

pub(crate) fn recent_by_query(state: &AppState, query: &ExecutionRunQuery) -> Vec<ExecutionRun> {
    let mut rows = recent_rows(state);
    rows.retain(|run| query.matches(run));
    sort_recent(&mut rows);
    rows.truncate(RECENT_RUN_LIMIT);
    rows
}

pub(super) fn recent_rows(state: &AppState) -> Vec<ExecutionRun> {
    state
        .execution_runs()
        .iter()
        .map(|row| {
            let mut run = row.value().clone();
            refresh_workflow_view(&mut run);
            run
        })
        .collect()
}

pub(crate) fn refresh_workflow_view(run: &mut ExecutionRun) {
    run.evidence.schema_version = run
        .evidence
        .schema_version
        .max(shared_types::EXECUTION_RUN_EVIDENCE_SCHEMA_VERSION);
    run.evidence.hedge_ticket_view = Some(shared_types::HedgeTicketView::from_execution_run(run));
}

impl ExecutionRunQuery {
    pub(crate) fn has_filter(&self) -> bool {
        self.run_id.is_some() || self.ticket_id.is_some() || self.opportunity_id.is_some()
    }

    pub(super) fn matches(&self, run: &ExecutionRun) -> bool {
        optional_id_matches(&self.run_id, &run.run_id)
            && optional_id_matches(&self.ticket_id, &run.ticket_id)
            && optional_id_matches(&self.opportunity_id, &run.opportunity_id)
    }
}

pub(super) fn optional_id_matches(expected: &Option<String>, actual: &str) -> bool {
    match expected {
        Some(expected) => actual == expected,
        None => true,
    }
}
