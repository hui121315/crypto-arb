use super::*;

#[derive(Debug)]
pub(super) struct OpportunityEnvelopeParts {
    pub(super) entry: Arc<SnapshotEntry<OpportunityScanReport>>,
    pub(super) snapshot_id: Option<String>,
    pub(super) source: &'static str,
    pub(super) status: OpportunityEnvelopeStatus,
    pub(super) retry_after_ms: Option<u64>,
    pub(super) error: Option<ApiProblem>,
}

pub(super) fn legacy_query_problems(limit: opportunity::OpportunityWideLimit) -> Vec<ApiProblem> {
    let mut problems = limit.into_query_problems();
    problems.push(opportunity::legacy_wide_endpoint_problem());
    problems
}

pub(super) fn response_parts(
    state: &AppState,
    _fresh: bool,
    _fast: bool,
) -> OpportunityEnvelopeParts {
    if let Some(parts) = current_snapshot(state, current_snapshot_source(state)) {
        return parts;
    }
    warming_response("warming")
}

fn current_snapshot_source(state: &AppState) -> &'static str {
    if state.arbitrage_scan_lock().try_lock().is_err() {
        "snapshot-refresh-inflight"
    } else {
        "snapshot"
    }
}

fn current_snapshot(state: &AppState, source: &'static str) -> Option<OpportunityEnvelopeParts> {
    let snapshot = state.opportunity_index().read()?;
    Some(OpportunityEnvelopeParts {
        entry: snapshot.entry(),
        snapshot_id: Some(snapshot.snapshot_id().to_owned()),
        source,
        status: OpportunityEnvelopeStatus::Fresh,
        retry_after_ms: None,
        error: None,
    })
}

fn warming_response(source: &'static str) -> OpportunityEnvelopeParts {
    let error = opportunity::warming_error();
    OpportunityEnvelopeParts {
        entry: Arc::new(SnapshotEntry {
            value: OpportunityScanReport::default(),
            cached_at: chrono::Utc::now(),
        }),
        snapshot_id: None,
        source,
        status: OpportunityEnvelopeStatus::Warming,
        retry_after_ms: error.retry_after_ms,
        error: Some(error),
    }
}
