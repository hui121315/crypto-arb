use super::*;

#[path = "storage_health/sql.rs"]
mod sql;
use sql::sql_ledger_storage_configured;
pub(super) use sql::sql_ledger_storage_health;

pub(super) fn review_storage_health(
    source: ReviewDataSource,
    row_count: usize,
    observed_at_ms: i64,
) -> VenueOperationHealth {
    let operation = review_storage_operation(source);
    let message = format!("{operation} 使用内存投影，尚未接入持久化复盘存储");
    let code = match source {
        ReviewDataSource::ExecutionLedger => codes::EXECUTION_LEDGER_STORAGE_UNAVAILABLE,
        ReviewDataSource::MissedOpportunityStore => codes::HISTORY_STORE_UNAVAILABLE,
    };
    let problem = storage_problem(code, &message, &operation);
    VenueOperationHealth {
        venue: REVIEW_STORAGE_VENUE.to_owned(),
        operation,
        status: VenueOperationStatus::Warn,
        source: REVIEW_STORAGE_SOURCE.to_owned(),
        message,
        supported: Some(true),
        configured: Some(false),
        requested: Some(row_count as u64),
        rows: Some(row_count as u64),
        freshness_ms: Some(0),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: None,
        evidence: None,
        problem: Some(problem),
        observed_at_ms,
    }
}

pub(super) fn with_review_storage_health<T>(
    mut envelope: ReviewEnvelope<T>,
    storage_health: VenueOperationHealth,
) -> ReviewEnvelope<T> {
    if storage_health_degrades_envelope(&storage_health) {
        envelope.status = ListStatus::Degraded;
        if let Some(problem) = storage_health.problem.clone() {
            let duplicate = envelope
                .problems
                .iter()
                .any(|current| current.code == problem.code && current.source == problem.source);
            if !duplicate {
                envelope.problems.push(problem);
            }
        }
    }
    envelope.storage_health = Some(storage_health);
    envelope
}

fn storage_health_degrades_envelope(health: &VenueOperationHealth) -> bool {
    health.status != VenueOperationStatus::Ok
        && !(health.status == VenueOperationStatus::Warn && health.configured == Some(false))
}

pub(super) fn with_trading_ledger_storage_health<T>(
    envelope: ReviewEnvelope<T>,
    service: &TradingService,
    observed_at_ms: i64,
) -> ReviewEnvelope<T> {
    let sql_snapshot = service.sql_ledger_storage_snapshot();
    if sql_ledger_storage_configured(&sql_snapshot) {
        let row_count = envelope.row_count;
        return with_review_storage_health(
            envelope,
            sql_ledger_storage_health(&sql_snapshot, row_count, observed_at_ms),
        );
    }
    with_execution_ledger_storage_health(envelope, service, observed_at_ms)
}

fn with_execution_ledger_storage_health<T>(
    envelope: ReviewEnvelope<T>,
    service: &TradingService,
    observed_at_ms: i64,
) -> ReviewEnvelope<T> {
    let snapshot = service.execution_ledger_storage_snapshot();
    let row_count = envelope.row_count;
    with_review_storage_health(
        envelope,
        execution_ledger_storage_health(&snapshot, row_count, observed_at_ms),
    )
}

pub(super) fn execution_ledger_storage_health(
    snapshot: &ExecutionLedgerStorageSnapshot,
    row_count: usize,
    observed_at_ms: i64,
) -> VenueOperationHealth {
    let status = execution_ledger_storage_status(snapshot);
    let message = execution_ledger_storage_message(snapshot);
    let error = execution_ledger_storage_error(snapshot);
    let code = if snapshot.configured {
        codes::EXECUTION_LEDGER_STORAGE_IO_FAILED
    } else {
        codes::EXECUTION_LEDGER_STORAGE_UNAVAILABLE
    };
    let problem = (status != VenueOperationStatus::Ok)
        .then(|| storage_problem(code, &message, EXECUTION_LEDGER_STORAGE_SOURCE));
    VenueOperationHealth {
        venue: REVIEW_STORAGE_VENUE.to_owned(),
        operation: review_storage_operation(ReviewDataSource::ExecutionLedger),
        status,
        source: EXECUTION_LEDGER_STORAGE_SOURCE.to_owned(),
        message,
        supported: Some(true),
        configured: Some(snapshot.configured),
        requested: Some(row_count as u64),
        rows: Some(snapshot.event_count as u64),
        freshness_ms: execution_ledger_storage_last_observed(snapshot)
            .map(|observed_at| observed_at_ms.saturating_sub(observed_at)),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error,
        evidence: None,
        problem,
        observed_at_ms,
    }
}

fn execution_ledger_storage_status(
    snapshot: &ExecutionLedgerStorageSnapshot,
) -> VenueOperationStatus {
    if !snapshot.configured {
        return VenueOperationStatus::Warn;
    }
    if snapshot.replay_failures > 0 || snapshot.append_failures > 0 {
        return VenueOperationStatus::Blocked;
    }
    if snapshot.query_failures > 0 {
        return VenueOperationStatus::Warn;
    }
    VenueOperationStatus::Ok
}

fn execution_ledger_storage_message(snapshot: &ExecutionLedgerStorageSnapshot) -> String {
    if !snapshot.configured {
        return "ExecutionLedger JSONL 未配置，复盘仍使用进程内投影".to_owned();
    }
    let path = snapshot.path.as_deref().unwrap_or("unknown path");
    format!(
        "ExecutionLedger JSONL path={path}; replayed={} append_ok={} append_failed={} query_ok={} query_failed={}",
        snapshot.replayed_events,
        snapshot.append_successes,
        snapshot.append_failures,
        snapshot.query_successes,
        snapshot.query_failures
    )
}

fn execution_ledger_storage_error(snapshot: &ExecutionLedgerStorageSnapshot) -> Option<String> {
    let mut parts = Vec::new();
    if snapshot.replay_failures > 0 {
        parts.push(format!("replay_failures={}", snapshot.replay_failures));
    }
    if snapshot.append_failures > 0 {
        parts.push(format!("append_failures={}", snapshot.append_failures));
    }
    if snapshot.query_failures > 0 {
        parts.push(format!("query_failures={}", snapshot.query_failures));
    }
    (!parts.is_empty()).then(|| parts.join(","))
}

fn execution_ledger_storage_last_observed(
    snapshot: &ExecutionLedgerStorageSnapshot,
) -> Option<i64> {
    [snapshot.last_append_at_ms, snapshot.last_query_at_ms]
        .into_iter()
        .flatten()
        .max()
}

fn review_storage_operation(source: ReviewDataSource) -> String {
    match source {
        ReviewDataSource::ExecutionLedger => "storage:review_execution_ledger",
        ReviewDataSource::MissedOpportunityStore => "storage:review_missed_opportunities",
    }
    .to_owned()
}

fn storage_problem(code: &'static str, message: &str, source: &str) -> ApiProblem {
    ApiProblem::new(code, message).with_source(source)
}
