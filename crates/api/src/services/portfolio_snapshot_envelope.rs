use common::AppError;
use shared_types::{
    problem::codes, AccountFieldQualityStatus, ApiProblem, PortfolioSnapshot,
    PortfolioSnapshotEnvelope, PortfolioSnapshotStatus, RuntimeProblem, VenueOperationHealth,
    VenueOperationStatus,
};

mod lifecycle_cache;

pub(crate) use lifecycle_cache::lifecycle_cache_response;

pub(crate) const SOURCE_LIFECYCLE: &str = "portfolio_lifecycle";
pub(crate) const SOURCE_LIFECYCLE_STALE: &str = "portfolio_lifecycle:last_success";
pub(crate) const OP_PORTFOLIO_SNAPSHOT: &str = "portfolio:snapshot";

pub(crate) fn snapshot_envelope(
    snapshot: PortfolioSnapshot,
    source: impl Into<String>,
    observed_at_ms: i64,
) -> PortfolioSnapshotEnvelope {
    let status = if snapshot.degraded {
        PortfolioSnapshotStatus::Degraded
    } else {
        PortfolioSnapshotStatus::Fresh
    };
    snapshot_envelope_with_status(snapshot, status, source, observed_at_ms)
}

pub(crate) fn stale_envelope(
    snapshot: PortfolioSnapshot,
    error: &AppError,
    source: impl Into<String>,
    observed_at_ms: i64,
) -> PortfolioSnapshotEnvelope {
    let mut envelope = snapshot_envelope_with_status(
        snapshot,
        PortfolioSnapshotStatus::Stale,
        source,
        observed_at_ms,
    );
    push_primary_problem(&mut envelope, error_problem(error, SOURCE_LIFECYCLE));
    envelope
}

pub(crate) fn error_envelope(
    error: &AppError,
    source: impl Into<String>,
    observed_at_ms: i64,
) -> PortfolioSnapshotEnvelope {
    let problem = error_problem(error, source.into());
    PortfolioSnapshotEnvelope {
        status: PortfolioSnapshotStatus::Error,
        source: problem
            .source
            .clone()
            .unwrap_or_else(|| SOURCE_LIFECYCLE.to_owned()),
        observed_at_ms,
        snapshot: None,
        problem: Some(problem.clone()),
        problems: vec![problem.clone()],
        operation_health: vec![blocked_operation_health(
            error,
            SOURCE_LIFECYCLE,
            "portfolio snapshot unavailable; no successful snapshot has been published",
            0,
            None,
            observed_at_ms,
        )],
        retry_after_ms: problem.retry_after_ms,
    }
}

pub(crate) fn blocked_operation_health(
    error: &AppError,
    source: impl Into<String>,
    message: impl Into<String>,
    rows: u64,
    freshness_ms: Option<i64>,
    observed_at_ms: i64,
) -> VenueOperationHealth {
    let source = source.into();
    let retry_after_ms = error.retry_after_ms();
    VenueOperationHealth {
        venue: "system".to_owned(),
        operation: OP_PORTFOLIO_SNAPSHOT.to_owned(),
        status: VenueOperationStatus::Blocked,
        source: source.clone(),
        message: message.into(),
        supported: Some(true),
        configured: None,
        requested: Some(1),
        rows: Some(rows),
        freshness_ms,
        retry_after_ms,
        latency_ms: None,
        latency_p95_ms: None,
        error: Some(error.to_string()),
        evidence: None,
        problem: Some(error_problem(error, source)),
        observed_at_ms,
    }
}

pub(crate) fn error_problem(error: &AppError, source: impl Into<String>) -> ApiProblem {
    let mut problem = error.to_api_problem().with_source(source);
    problem.code = codes::PORTFOLIO_SNAPSHOT_UNAVAILABLE.to_owned();
    problem.message = format!("portfolio snapshot unavailable: {}", problem.message);
    problem.details = Some(serde_json::json!({
        "upstreamCode": error.code(),
        "upstreamStatus": error.status().as_u16(),
    }));
    problem
}

fn snapshot_envelope_with_status(
    snapshot: PortfolioSnapshot,
    status: PortfolioSnapshotStatus,
    source: impl Into<String>,
    observed_at_ms: i64,
) -> PortfolioSnapshotEnvelope {
    let source = source.into();
    let mut envelope = PortfolioSnapshotEnvelope {
        status,
        source,
        observed_at_ms,
        snapshot: Some(snapshot),
        problem: None,
        problems: Vec::new(),
        operation_health: Vec::new(),
        retry_after_ms: None,
    };
    envelope.operation_health = envelope
        .snapshot
        .as_ref()
        .map(|snapshot| snapshot.operation_health.clone())
        .unwrap_or_default();
    if let Some(snapshot) = envelope.snapshot.as_ref() {
        for problem in snapshot_api_problems(snapshot) {
            push_problem(&mut envelope, problem);
        }
    }
    for problem in operation_health_problems(&envelope.operation_health) {
        push_problem(&mut envelope, problem);
    }
    let requires_problem = status == PortfolioSnapshotStatus::Degraded
        || envelope
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.degraded);
    if requires_problem && envelope.problems.is_empty() {
        let problem = envelope
            .snapshot
            .as_ref()
            .map(|snapshot| degraded_snapshot_problem(snapshot, &envelope.source))
            .unwrap_or_else(|| {
                ApiProblem::new(
                    codes::PORTFOLIO_SNAPSHOT_DEGRADED,
                    "portfolio snapshot is degraded",
                )
                .with_source(envelope.source.clone())
            });
        push_problem(&mut envelope, problem);
    }
    envelope
}

fn snapshot_api_problems(snapshot: &PortfolioSnapshot) -> Vec<ApiProblem> {
    let mut problems = Vec::new();
    if snapshot.summary.nav_evidence.status != AccountFieldQualityStatus::Actual {
        problems.extend(snapshot.summary.nav_evidence.problem.clone());
    }
    problems.extend(snapshot.problems.iter().map(runtime_api_problem));
    problems.extend(snapshot.account_state.problems.iter().cloned());
    problems.extend(snapshot.account_state.balances.problems.iter().cloned());
    problems.extend(snapshot.account_state.positions.problems.iter().cloned());
    problems.extend(snapshot.account_state.open_orders.problems.iter().cloned());
    problems
}

fn runtime_api_problem(problem: &RuntimeProblem) -> ApiProblem {
    problem.to_api_problem()
}

fn degraded_snapshot_problem(snapshot: &PortfolioSnapshot, source: &str) -> ApiProblem {
    let mut problem = ApiProblem::new(
        codes::PORTFOLIO_SNAPSHOT_DEGRADED,
        "portfolio snapshot contains partial or unverified account data",
    )
    .with_source(source);
    problem.details = Some(serde_json::json!({
        "accountStateStatus": snapshot.account_state.status,
        "balanceStatus": snapshot.account_state.balances.status,
        "positionStatus": snapshot.account_state.positions.status,
        "navStatus": snapshot.summary.nav_evidence.status,
        "fieldQualityIssueCount": snapshot.account_state.field_quality.iter()
            .filter(|row| row.status != AccountFieldQualityStatus::Actual)
            .count(),
        "runtimeProblemCount": snapshot.problems.len(),
    }));
    problem
}

fn operation_health_problems(rows: &[VenueOperationHealth]) -> Vec<ApiProblem> {
    rows.iter().filter_map(|row| row.problem.clone()).collect()
}

fn push_problem(envelope: &mut PortfolioSnapshotEnvelope, problem: ApiProblem) {
    envelope.retry_after_ms = max_retry_after(envelope.retry_after_ms, problem.retry_after_ms);
    if envelope
        .problems
        .iter()
        .any(|existing| same_problem_identity(existing, &problem))
    {
        return;
    }
    if envelope.problem.is_none() {
        envelope.problem = Some(problem.clone());
    }
    envelope.problems.push(problem);
}

fn same_problem_identity(left: &ApiProblem, right: &ApiProblem) -> bool {
    left.code == right.code
        && left.message == right.message
        && left.source == right.source
        && problem_venue(left) == problem_venue(right)
}

fn problem_venue(problem: &ApiProblem) -> Option<&str> {
    problem
        .details
        .as_ref()
        .and_then(|details| details.get("venue"))
        .and_then(serde_json::Value::as_str)
        .filter(|venue| !venue.is_empty())
}

fn push_primary_problem(envelope: &mut PortfolioSnapshotEnvelope, problem: ApiProblem) {
    envelope.retry_after_ms = max_retry_after(envelope.retry_after_ms, problem.retry_after_ms);
    envelope.problems.retain(|existing| existing != &problem);
    envelope.problems.insert(0, problem.clone());
    envelope.problem = Some(problem);
}

fn max_retry_after(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests;
