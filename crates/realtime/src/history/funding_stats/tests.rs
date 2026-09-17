use super::*;

#[test]
fn stats_use_one_three_nine_funding_cycles() {
    let latest_ms = 72 * HOUR_MS;
    let rows = vec![
        row(latest_ms - 72 * HOUR_MS, 1.0, 8),
        row(latest_ms - 24 * HOUR_MS, 2.0, 8),
        row(latest_ms - 8 * HOUR_MS, 4.0, 8),
        row(latest_ms, 8.0, 8),
    ];

    let stats = build_rows(rows, latest_ms + 1);

    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].base_interval_hours, 8);
    assert_eq!(stats[0].windows[0].window_hours, 8);
    assert_eq!(stats[0].windows[1].window_hours, 24);
    assert_eq!(stats[0].windows[2].window_hours, 72);
    assert_eq!(stats[0].windows[0].sample_count, 2);
    assert_eq!(stats[0].windows[2].sample_count, 4);
    assert_eq!(stats[0].evidence.sample_count, 4);
    assert_eq!(stats[0].evidence.source, FUNDING_DIFF_SOURCE);
    assert_eq!(stats[0].windows[0].evidence.sample_count, 2);
    assert_eq!(
        stats[0].windows[0].sample_health,
        FundingDiffSampleHealth::Ok
    );
    assert_eq!(
        stats[0].windows[2].sample_health,
        FundingDiffSampleHealth::Thin
    );
}

#[test]
fn one_hour_contracts_keep_short_windows() {
    let latest_ms = 9 * HOUR_MS;
    let rows = vec![
        row(latest_ms - 3 * HOUR_MS, 1.0, 1),
        row(latest_ms - HOUR_MS, 2.0, 1),
        row(latest_ms, 3.0, 1),
    ];

    let stats = build_rows(rows, latest_ms + 1);

    assert_eq!(stats[0].base_interval_hours, 1);
    assert_eq!(stats[0].windows[0].window_hours, 1);
    assert_eq!(stats[0].windows[1].window_hours, 3);
    assert_eq!(stats[0].windows[2].window_hours, 9);
    assert_eq!(stats[0].windows[2].current_percentile, 100);
}

#[test]
fn stale_windows_carry_health_problem() {
    let latest_ms = 9 * HOUR_MS;
    let rows = vec![row(latest_ms, 3.0, 1)];

    let stats = build_rows(rows, latest_ms + 10 * HOUR_MS);

    assert_eq!(
        stats[0].windows[0].sample_health,
        FundingDiffSampleHealth::Stale
    );
    assert!(stats[0].windows[0].problem.is_some());
    assert!(!stats[0].windows[0].evidence.is_usable());
    assert_eq!(
        stats[0].windows[0]
            .problem_detail
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some(codes::FUNDING_DIFF_WINDOW_DEGRADED)
    );
    assert!(stats[0].problem.is_some());
    assert_eq!(
        stats[0]
            .problem_detail
            .as_ref()
            .and_then(|problem| problem.source.as_deref()),
        Some(FUNDING_DIFF_SOURCE)
    );
}

#[test]
fn empty_window_fails_closed_without_percentiles() {
    let window = stats_from_values(3, 24, f64::NAN, 1_000, 0, &[]);

    assert_eq!(window.sample_count, 0);
    assert_eq!(window.sample_health, FundingDiffSampleHealth::Empty);
    assert_eq!(
        window.evidence.sample_health,
        FundingDiffSampleHealth::Empty
    );
    assert_eq!(window.evidence.sample_count, 0);
    assert!(!window.evidence.is_usable());
    assert_empty_percentiles(&window);
    assert_eq!(
        window.problem.as_deref(),
        Some("funding diff window has no samples")
    );
    assert_eq!(
        window.problem_detail.as_ref().map(|p| p.code.as_str()),
        Some(codes::FUNDING_DIFF_WINDOW_DEGRADED)
    );
}

#[test]
fn projector_merges_only_new_rows_and_replaces_duplicate_timestamps() {
    let latest_ms = 8 * HOUR_MS;
    let mut projector = FundingDiffStatsProjector::default();
    projector.replace(vec![
        row(latest_ms - 8 * HOUR_MS, 1.0, 8),
        row(latest_ms, 2.0, 8),
    ]);

    let stats = projector.apply(&[row(latest_ms + 8 * HOUR_MS, 4.0, 8)], latest_ms + 1);
    assert_eq!(stats[0].latest_diff_bps, 4.0);
    assert_eq!(stats[0].windows[1].sample_count, 3);

    let stats = projector.apply(&[row(latest_ms + 8 * HOUR_MS, 8.0, 8)], latest_ms + 2);
    assert_eq!(stats[0].latest_diff_bps, 8.0);
    assert_eq!(stats[0].windows[1].sample_count, 3);
    assert_eq!(projector.retained_sample_count(), 3);
}

#[test]
fn projector_retains_only_the_longest_funding_window() {
    let latest_ms = 80 * HOUR_MS;
    let mut projector = FundingDiffStatsProjector::default();
    projector.replace(vec![
        row(0, 1.0, 8),
        row(latest_ms - 72 * HOUR_MS, 2.0, 8),
        row(latest_ms, 3.0, 8),
    ]);

    let stats = projector.snapshot(latest_ms + 1);

    assert_eq!(projector.retained_sample_count(), 2);
    assert_eq!(stats[0].windows[2].sample_count, 2);
}

fn assert_empty_percentiles(window: &FundingDiffWindowStats) {
    assert_eq!(window.mean_diff_bps, 0.0);
    assert_eq!(window.p50_diff_bps, 0.0);
    assert_eq!(window.p75_diff_bps, 0.0);
    assert_eq!(window.p90_diff_bps, 0.0);
    assert_eq!(window.p95_diff_bps, 0.0);
    assert_eq!(window.stddev_diff_bps, 0.0);
    assert_eq!(window.positive_ratio, 0.0);
    assert_eq!(window.current_percentile, 0);
}

fn row(occurred_at_ms: i64, gross_diff_bps: f64, interval_hours: u32) -> FundingDiffRow {
    FundingDiffRow {
        occurred_at_ms,
        symbol: "MU".into(),
        long_exchange: "km".into(),
        short_exchange: "kucoin".into(),
        long_rate_8h: 0.0,
        short_rate_8h: 0.0,
        gross_diff_bps,
        long_next_funding_ms: occurred_at_ms + i64::from(interval_hours) * HOUR_MS,
        short_next_funding_ms: occurred_at_ms + i64::from(interval_hours) * HOUR_MS,
        window_alignment_minutes: 0,
        long_interval_hours: interval_hours,
        short_interval_hours: interval_hours,
        min_volume_24h: 1_000_000.0,
    }
}
