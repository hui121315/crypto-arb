use super::{
    push_primary_problem, OP_PORTFOLIO_SNAPSHOT, SOURCE_LIFECYCLE, SOURCE_LIFECYCLE_STALE,
};
use chrono::{DateTime, Utc};
use realtime::SnapshotEntry;
use shared_types::{
    problem::codes, ApiProblem, PortfolioSnapshotEnvelope, PortfolioSnapshotStatus,
    VenueOperationHealth, VenueOperationStatus,
};
use std::time::Duration;

pub(crate) fn lifecycle_cache_response(
    entry: Option<&SnapshotEntry<PortfolioSnapshotEnvelope>>,
    refresh_interval: Duration,
    now: DateTime<Utc>,
) -> PortfolioSnapshotEnvelope {
    let retry_after_ms = duration_ms(refresh_interval);
    let Some(entry) = entry else {
        return warming_envelope(now.timestamp_millis(), retry_after_ms);
    };
    let freshness_ms = now
        .signed_duration_since(entry.cached_at)
        .num_milliseconds()
        .max(0);
    let stale_after_ms = i64::try_from(retry_after_ms)
        .unwrap_or(i64::MAX)
        .saturating_mul(2);
    if freshness_ms <= stale_after_ms || entry.value.snapshot.is_none() {
        return entry.value.clone();
    }
    stale_cached_envelope(
        entry.value.clone(),
        freshness_ms,
        retry_after_ms,
        now.timestamp_millis(),
    )
}

fn warming_envelope(now_ms: i64, retry_after_ms: u64) -> PortfolioSnapshotEnvelope {
    let problem = ApiProblem::new(
        codes::PORTFOLIO_SNAPSHOT_UNAVAILABLE,
        "portfolio lifecycle is warming; no snapshot has been published",
    )
    .with_status(503)
    .with_source(SOURCE_LIFECYCLE)
    .with_retry_after_ms(Some(retry_after_ms));
    let health = cache_health(
        &problem,
        "portfolio lifecycle is warming",
        0,
        None,
        retry_after_ms,
        now_ms,
    );
    PortfolioSnapshotEnvelope {
        status: PortfolioSnapshotStatus::Error,
        source: SOURCE_LIFECYCLE.to_owned(),
        observed_at_ms: now_ms,
        snapshot: None,
        problem: Some(problem.clone()),
        problems: vec![problem],
        operation_health: vec![health],
        retry_after_ms: Some(retry_after_ms),
    }
}

fn stale_cached_envelope(
    mut envelope: PortfolioSnapshotEnvelope,
    freshness_ms: i64,
    retry_after_ms: u64,
    now_ms: i64,
) -> PortfolioSnapshotEnvelope {
    let mut problem = ApiProblem::new(
        codes::PORTFOLIO_SNAPSHOT_STALE,
        "portfolio lifecycle has not published a fresh snapshot",
    )
    .with_status(503)
    .with_source(SOURCE_LIFECYCLE_STALE)
    .with_retry_after_ms(Some(retry_after_ms));
    problem.details = Some(serde_json::json!({
        "freshnessMs": freshness_ms,
        "refreshIntervalMs": retry_after_ms,
    }));
    let row_count = envelope
        .snapshot
        .as_ref()
        .map_or(0, |snapshot| snapshot.positions.len() as u64);
    let health = cache_health(
        &problem,
        "portfolio lifecycle stalled; serving the last published snapshot",
        row_count,
        Some(freshness_ms),
        retry_after_ms,
        now_ms,
    );
    if let Some(snapshot) = envelope.snapshot.as_mut() {
        snapshot.degraded = true;
        push_health(&mut snapshot.operation_health, health.clone());
    }
    envelope.status = PortfolioSnapshotStatus::Stale;
    envelope.source = SOURCE_LIFECYCLE_STALE.to_owned();
    push_health(&mut envelope.operation_health, health);
    push_primary_problem(&mut envelope, problem);
    envelope
}

fn cache_health(
    problem: &ApiProblem,
    message: &str,
    rows: u64,
    freshness_ms: Option<i64>,
    retry_after_ms: u64,
    now_ms: i64,
) -> VenueOperationHealth {
    let source = problem.source.as_deref().unwrap_or(SOURCE_LIFECYCLE);
    VenueOperationHealth {
        venue: "system".to_owned(),
        operation: OP_PORTFOLIO_SNAPSHOT.to_owned(),
        status: VenueOperationStatus::Blocked,
        source: source.to_owned(),
        message: message.to_owned(),
        supported: Some(true),
        configured: None,
        requested: Some(1),
        rows: Some(rows),
        freshness_ms,
        retry_after_ms: Some(retry_after_ms),
        latency_ms: None,
        latency_p95_ms: None,
        error: Some(problem.message.clone()),
        evidence: None,
        problem: Some(problem.clone()),
        observed_at_ms: now_ms,
    }
}

fn push_health(rows: &mut Vec<VenueOperationHealth>, health: VenueOperationHealth) {
    if rows.iter().any(|row| {
        row.venue == health.venue
            && row.operation == health.operation
            && row.source == health.source
    }) {
        return;
    }
    rows.push(health);
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
