use super::*;
use crate::services::snapshots::ARBITRAGE_SNAPSHOT_REFRESH_INTERVAL_MS;

pub(super) const SNAPSHOT_STALE_AFTER_MS: i64 = 2 * WARMING_RETRY_AFTER_MS as i64;
const SNAPSHOT_MAX_STALE_AFTER_MS: i64 = 120_000;

pub(super) struct SnapshotClassification {
    pub(super) status: OpportunityEnvelopeStatus,
    pub(super) freshness_ms: i64,
    pub(super) problem: Option<ApiProblem>,
}

pub(super) fn classify_snapshot(
    status: OpportunityEnvelopeStatus,
    cached_at: &DateTime<Utc>,
    observed_at_ms: i64,
    last_scan_ms: u64,
) -> SnapshotClassification {
    let cached_at_ms = cached_at.timestamp_millis();
    let freshness_ms = observed_at_ms.saturating_sub(cached_at_ms).max(0);
    let stale_after_ms = snapshot_stale_after_ms(last_scan_ms);
    let becomes_stale = freshness_ms > stale_after_ms
        && !matches!(
            status,
            OpportunityEnvelopeStatus::Warming | OpportunityEnvelopeStatus::Error
        );
    let problem = becomes_stale.then(|| {
        stale_snapshot_problem(
            cached_at_ms,
            observed_at_ms,
            freshness_ms,
            stale_after_ms,
            last_scan_ms,
        )
    });
    SnapshotClassification {
        status: if becomes_stale {
            OpportunityEnvelopeStatus::Stale
        } else {
            status
        },
        freshness_ms,
        problem,
    }
}

pub(super) fn snapshot_stale_after_ms(last_scan_ms: u64) -> i64 {
    let jitter_ms = (last_scan_ms / 2).max(ARBITRAGE_SNAPSHOT_REFRESH_INTERVAL_MS);
    let adaptive_ms = last_scan_ms
        .saturating_add(ARBITRAGE_SNAPSHOT_REFRESH_INTERVAL_MS)
        .saturating_add(jitter_ms);
    i64::try_from(adaptive_ms)
        .unwrap_or(i64::MAX)
        .clamp(SNAPSHOT_STALE_AFTER_MS, SNAPSHOT_MAX_STALE_AFTER_MS)
}

fn stale_snapshot_problem(
    cached_at_ms: i64,
    observed_at_ms: i64,
    freshness_ms: i64,
    stale_after_ms: i64,
    last_scan_ms: u64,
) -> ApiProblem {
    let mut problem = ApiProblem::new(
        codes::OPPORTUNITY_SNAPSHOT_STALE,
        "opportunity snapshot exceeded its measured refresh window; retaining cached rows",
    )
    .with_retry_after_ms(Some(WARMING_RETRY_AFTER_MS))
    .with_source("arbitrage-snapshot");
    problem.details = Some(serde_json::json!({
        "cachedAtMs": cached_at_ms,
        "observedAtMs": observed_at_ms,
        "freshnessMs": freshness_ms,
        "staleAfterMs": stale_after_ms,
        "lastScanMs": last_scan_ms,
        "refreshIntervalMs": ARBITRAGE_SNAPSHOT_REFRESH_INTERVAL_MS,
    }));
    problem
}
