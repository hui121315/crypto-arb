use super::*;

pub(super) fn sql_ledger_storage_configured(snapshot: &SqlLedgerStorageSnapshot) -> bool {
    snapshot.migration.configured
        || snapshot.writer_configured
        || snapshot.replayed_events > 0
        || snapshot.replayed_order_snapshots > 0
}

pub(in crate::services::review) fn sql_ledger_storage_health(
    snapshot: &SqlLedgerStorageSnapshot,
    row_count: usize,
    observed_at_ms: i64,
) -> VenueOperationHealth {
    let status = sql_ledger_storage_status(snapshot);
    let message = sql_ledger_storage_message(snapshot);
    let error = sql_ledger_storage_error(snapshot);
    let problem = (status != VenueOperationStatus::Ok).then(|| {
        storage_problem(
            codes::TRADING_SQL_LEDGER_UNAVAILABLE,
            &message,
            SQL_LEDGER_STORAGE_SOURCE,
        )
    });
    VenueOperationHealth {
        venue: REVIEW_STORAGE_VENUE.to_owned(),
        operation: review_storage_operation(ReviewDataSource::ExecutionLedger),
        status,
        source: SQL_LEDGER_STORAGE_SOURCE.to_owned(),
        message,
        supported: Some(true),
        configured: Some(snapshot.writer_configured),
        requested: Some(row_count as u64),
        rows: Some(snapshot.replayed_events as u64),
        freshness_ms: sql_ledger_storage_last_observed(snapshot)
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

fn sql_ledger_storage_status(snapshot: &SqlLedgerStorageSnapshot) -> VenueOperationStatus {
    if !snapshot.migration.configured {
        return VenueOperationStatus::Warn;
    }
    if !snapshot.migration.applied
        || snapshot.replay_event_failures > 0
        || snapshot.replay_snapshot_failures > 0
        || snapshot.replay_run_finality_failures > 0
        || snapshot.event_append_failures > 0
        || snapshot.snapshot_append_failures > 0
        || snapshot.run_finality_append_failures > 0
    {
        return VenueOperationStatus::Blocked;
    }
    if !snapshot.writer_configured || snapshot.replay_query_failures > 0 {
        return VenueOperationStatus::Warn;
    }
    VenueOperationStatus::Ok
}

fn sql_ledger_storage_message(snapshot: &SqlLedgerStorageSnapshot) -> String {
    format!(
        "Trading SQL ledger migration_applied={} writer={} replayed_events={} replayed_orders={} replayed_balances={} replayed_run_finality={} append_ok={} append_failed={} run_finality_append_ok={} run_finality_append_failed={} replay_query_ok={} replay_query_failed={}",
        snapshot.migration.applied,
        snapshot.writer_configured,
        snapshot.replayed_events,
        snapshot.replayed_order_snapshots,
        snapshot.replayed_balance_events,
        snapshot.replayed_run_finality_events,
        snapshot.event_append_successes,
        snapshot.event_append_failures,
        snapshot.run_finality_append_successes,
        snapshot.run_finality_append_failures,
        snapshot.replay_query_successes,
        snapshot.replay_query_failures
    )
}

fn sql_ledger_storage_error(snapshot: &SqlLedgerStorageSnapshot) -> Option<String> {
    let mut parts = Vec::new();
    if !snapshot.migration.applied {
        parts.push("migration_not_applied".to_owned());
    }
    for (label, count) in [
        ("replay_event_failures", snapshot.replay_event_failures),
        (
            "replay_snapshot_failures",
            snapshot.replay_snapshot_failures,
        ),
        (
            "replay_run_finality_failures",
            snapshot.replay_run_finality_failures,
        ),
        ("event_append_failures", snapshot.event_append_failures),
        (
            "snapshot_append_failures",
            snapshot.snapshot_append_failures,
        ),
        (
            "run_finality_append_failures",
            snapshot.run_finality_append_failures,
        ),
        ("replay_query_failures", snapshot.replay_query_failures),
    ] {
        if count > 0 {
            parts.push(format!("{label}={count}"));
        }
    }
    (!parts.is_empty()).then(|| parts.join(","))
}

fn sql_ledger_storage_last_observed(snapshot: &SqlLedgerStorageSnapshot) -> Option<i64> {
    [snapshot.last_append_at_ms, snapshot.last_replay_at_ms]
        .into_iter()
        .flatten()
        .max()
}
