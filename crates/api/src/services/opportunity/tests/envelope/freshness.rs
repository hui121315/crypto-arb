use super::super::*;

#[test]
fn slow_scan_uses_a_bounded_measured_freshness_window() {
    let observed_at = Utc::now();
    let observed_at_ms = observed_at.timestamp_millis();
    let last_scan_ms = 35_000;
    let stale_after_ms = snapshot_health::snapshot_stale_after_ms(last_scan_ms);
    assert_eq!(stale_after_ms, 57_500);
    assert_eq!(snapshot_health::snapshot_stale_after_ms(u64::MAX), 120_000);

    let boundary_cached_at = observed_at - chrono::Duration::milliseconds(stale_after_ms);
    let boundary = snapshot_health::classify_snapshot(
        OpportunityEnvelopeStatus::Fresh,
        &boundary_cached_at,
        observed_at_ms,
        last_scan_ms,
    );
    assert_eq!(boundary.status, OpportunityEnvelopeStatus::Fresh);
    assert!(boundary.problem.is_none());

    let stale = snapshot_health::classify_snapshot(
        OpportunityEnvelopeStatus::Fresh,
        &(boundary_cached_at - chrono::Duration::milliseconds(1)),
        observed_at_ms,
        last_scan_ms,
    );
    assert_eq!(stale.status, OpportunityEnvelopeStatus::Stale);
    let details = stale
        .problem
        .and_then(|problem| problem.details)
        .unwrap_or_default();
    assert_eq!(details["staleAfterMs"].as_i64(), Some(stale_after_ms));
    assert_eq!(details["lastScanMs"].as_u64(), Some(last_scan_ms));
}
