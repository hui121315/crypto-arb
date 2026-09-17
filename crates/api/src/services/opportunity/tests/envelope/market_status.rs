use super::*;

#[test]
fn envelope_keeps_hyperliquid_builder_spot_gap_out_of_scan_failures() {
    let cached_at = Utc::now();
    let meta = OpportunityScanMeta {
        market_data_problem_count: 0,
        market_data_status: Some(shared_types::MarketDataSnapshotStatus {
            observed_at_ms: 1,
            rows: vec![shared_types::MarketDataSnapshotStatusRow {
                venue: "hyperliquid:xyz".into(),
                operation: shared_types::MarketDataSnapshotOperation::SpotTicks,
                health: shared_types::MarketDataHealth {
                    quality: shared_types::MarketDataQuality::Missing,
                    source: shared_types::MarketDataSourceKind::RestBaseline,
                    freshness_ms: None,
                    retry_after_ms: None,
                    last_error: Some("fanout produced no rows".into()),
                    observed_at_ms: 1,
                    coverage: Some(shared_types::MarketDataCoverage::new(1, 0)),
                    problem: Some(
                        ApiProblem::new("MARKET_DATA_MISSING", "fanout produced no rows")
                            .with_source("rest_baseline"),
                    ),
                },
            }],
        }),
        ..OpportunityScanMeta::default()
    };

    let env = envelope(OpportunityEnvelopeInput {
        opportunities: Vec::new(),
        source_rows: &[],
        filtered_rows: &[],
        meta,
        cached_at,
        source: "snapshot",
        status: OpportunityEnvelopeStatus::Fresh,
        scope: OpportunityEnvelopeScope::MainP0,
        query_key: "test".into(),
        retry_after_ms: None,
        error: None,
        query_problems: Vec::new(),
    });

    assert_eq!(env.status, OpportunityEnvelopeStatus::Fresh);
    assert!(env.partial_failures.is_empty());
}

#[test]
fn envelope_keeps_bounded_ws_warmup_out_of_scan_failures() {
    let cached_at = Utc::now();
    let meta = OpportunityScanMeta {
        market_data_status: Some(shared_types::MarketDataSnapshotStatus {
            observed_at_ms: 1,
            rows: vec![shared_types::MarketDataSnapshotStatusRow {
                venue: "kucoin".into(),
                operation: shared_types::MarketDataSnapshotOperation::WsTicker,
                health: shared_types::MarketDataHealth {
                    quality: shared_types::MarketDataQuality::Unverified,
                    source: shared_types::MarketDataSourceKind::WsPush,
                    freshness_ms: None,
                    retry_after_ms: Some(7_000),
                    last_error: Some("awaiting first websocket event".into()),
                    observed_at_ms: 1,
                    coverage: Some(shared_types::MarketDataCoverage::new(8, 7)),
                    problem: Some(
                        ApiProblem::new("MARKET_DATA_WARMING", "awaiting first websocket event")
                            .with_source("ws_push")
                            .with_retry_after_ms(Some(7_000)),
                    ),
                },
            }],
        }),
        ..OpportunityScanMeta::default()
    };

    let env = envelope(OpportunityEnvelopeInput {
        opportunities: Vec::new(),
        source_rows: &[],
        filtered_rows: &[],
        meta,
        cached_at,
        source: "snapshot",
        status: OpportunityEnvelopeStatus::Fresh,
        scope: OpportunityEnvelopeScope::MainP0,
        query_key: "test".into(),
        retry_after_ms: None,
        error: None,
        query_problems: Vec::new(),
    });

    assert_eq!(env.status, OpportunityEnvelopeStatus::Fresh);
    assert!(env.partial_failures.is_empty());
}

#[test]
fn envelope_keeps_venue_discovery_failure_out_of_global_list_health() {
    let cached_at = Utc::now();
    let meta = OpportunityScanMeta {
        market_data_status: Some(shared_types::MarketDataSnapshotStatus {
            observed_at_ms: 1,
            rows: vec![shared_types::MarketDataSnapshotStatusRow {
                venue: "binance".into(),
                operation: shared_types::MarketDataSnapshotOperation::SpotTicks,
                health: shared_types::MarketDataHealth {
                    quality: shared_types::MarketDataQuality::Missing,
                    source: shared_types::MarketDataSourceKind::RestBaseline,
                    freshness_ms: None,
                    retry_after_ms: None,
                    last_error: Some("discovery shard timed out".into()),
                    observed_at_ms: 1,
                    coverage: Some(shared_types::MarketDataCoverage::new(1, 0)),
                    problem: Some(
                        ApiProblem::new("MARKET_DATA_MISSING", "discovery shard timed out")
                            .with_source("rest_baseline"),
                    ),
                },
            }],
        }),
        ..OpportunityScanMeta::default()
    };

    let env = envelope(OpportunityEnvelopeInput {
        opportunities: Vec::new(),
        source_rows: &[],
        filtered_rows: &[],
        meta,
        cached_at,
        source: "snapshot",
        status: OpportunityEnvelopeStatus::Fresh,
        scope: OpportunityEnvelopeScope::MainP0,
        query_key: "test".into(),
        retry_after_ms: None,
        error: None,
        query_problems: Vec::new(),
    });

    assert_eq!(env.status, OpportunityEnvelopeStatus::Fresh);
    assert!(env.partial_failures.is_empty());
}
