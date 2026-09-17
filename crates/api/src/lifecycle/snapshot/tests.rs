use super::top_window::{apply_top_window_delta, TopWindowState};
use super::*;
use shared_types::{
    MarketDataCoverage, MarketDataHealth, MarketDataQuality, MarketDataSnapshotOperation,
    MarketDataSnapshotStatus, MarketDataSnapshotStatusRow, MarketDataSourceKind,
};

#[test]
fn market_event_scan_uses_a_quarter_duty_cycle_instead_of_five_second_floor() {
    let now = std::time::Instant::now();
    let interval = std::time::Duration::from_secs(5);
    let recent = now
        .checked_sub(std::time::Duration::from_millis(400))
        .unwrap_or(now);
    let scan_elapsed = std::time::Duration::from_millis(240);

    assert_eq!(
        event_scan_delay(interval, recent, scan_elapsed, now),
        std::time::Duration::from_millis(560)
    );
    assert!(event_scan_delay(
        interval,
        now.checked_sub(std::time::Duration::from_millis(1_000))
            .unwrap_or(now),
        scan_elapsed,
        now
    )
    .is_zero());
}

#[test]
fn market_event_scan_has_fast_floor_and_periodic_ceiling() {
    let now = std::time::Instant::now();
    let interval = std::time::Duration::from_secs(5);
    let recent = now
        .checked_sub(std::time::Duration::from_millis(100))
        .unwrap_or(now);

    assert_eq!(
        event_scan_delay(interval, recent, std::time::Duration::from_millis(20), now),
        std::time::Duration::from_millis(150)
    );
    assert_eq!(
        event_scan_delay(interval, recent, std::time::Duration::from_secs(2), now),
        std::time::Duration::from_millis(4_900)
    );
}

#[test]
fn attach_market_data_problems_keeps_only_aggregate_scan_status() {
    let market_data = MarketDataCache::default();
    market_data.record_runtime_unsupported(
        "okx",
        crate::services::market_data::cache::MARKET_OP_REST_PERP_TICKERS,
        crate::services::market_data::MarketSource::RestBaseline,
        1,
        "unsupported",
    );
    let mut meta = shared_types::OpportunityScanMeta {
        market_data_problem_count: 2,
        degraded_venues: vec!["bybit:perp_tickers".into(), "bybit:metadata".into()],
        market_data_status: Some(MarketDataSnapshotStatus {
            observed_at_ms: 1,
            rows: vec![
                MarketDataSnapshotStatusRow {
                    venue: crate::services::market_data::cache::MARKET_AGGREGATE_VENUE.into(),
                    operation: MarketDataSnapshotOperation::PerpTickers,
                    health: MarketDataHealth {
                        quality: MarketDataQuality::RateLimited,
                        source: MarketDataSourceKind::RestBaseline,
                        freshness_ms: None,
                        retry_after_ms: Some(2_000),
                        last_error: Some("rate limited".into()),
                        observed_at_ms: 1,
                        coverage: Some(MarketDataCoverage::new(1, 0)),
                        problem: None,
                    },
                },
                MarketDataSnapshotStatusRow {
                    venue: "bybit".into(),
                    operation: MarketDataSnapshotOperation::Metadata,
                    health: MarketDataHealth {
                        quality: MarketDataQuality::Unsupported,
                        source: MarketDataSourceKind::RestBaseline,
                        freshness_ms: None,
                        retry_after_ms: None,
                        last_error: Some(
                            "adapter has no separate metadata cache to refresh".into(),
                        ),
                        observed_at_ms: 1,
                        coverage: Some(MarketDataCoverage::new(1, 0)),
                        problem: None,
                    },
                },
                MarketDataSnapshotStatusRow {
                    venue: "hyperliquid:xyz".into(),
                    operation: MarketDataSnapshotOperation::SpotTicks,
                    health: MarketDataHealth {
                        quality: MarketDataQuality::Missing,
                        source: MarketDataSourceKind::RestBaseline,
                        freshness_ms: None,
                        retry_after_ms: None,
                        last_error: Some("fanout produced no rows".into()),
                        observed_at_ms: 1,
                        coverage: Some(MarketDataCoverage::new(1, 0)),
                        problem: None,
                    },
                },
            ],
        }),
        ..shared_types::OpportunityScanMeta::default()
    };

    attach_market_data_problems(&market_data, &mut meta);

    assert_eq!(meta.market_data_problem_count, 1);
    assert_eq!(meta.degraded_venues, vec!["all:perp_tickers"]);
}

#[test]
fn top_id_delta_reports_changed_and_removed_window_ids() {
    let mut top_window = TopWindowState::default();
    let mut first = stream_event(&["a", "b", "c"]);

    apply_top_window_delta(
        &mut first,
        product_rows(&[("a", 1.0), ("b", 1.0), ("c", 1.0)]),
        &mut top_window,
    );

    assert_eq!(first.changed_ids, vec!["a", "b", "c"]);
    assert!(first.removed_ids.is_empty());
    assert_eq!(top_window.ids, vec!["a", "b", "c"]);

    let mut second = stream_event(&["b", "c", "d"]);

    apply_top_window_delta(
        &mut second,
        product_rows(&[("b", 1.0), ("c", 2.0), ("d", 1.0)]),
        &mut top_window,
    );

    assert_eq!(second.changed_ids, vec!["c", "d"]);
    assert_eq!(second.removed_ids, vec!["a"]);
    assert_eq!(top_window.ids, vec!["b", "c", "d"]);
}

#[test]
fn stream_payload_metrics_record_runtime_size_and_delta_counts() {
    let metrics = crate::metrics::Metrics::new();
    let mut event = stream_event(&["a", "b"]);
    event.changed_ids = vec!["b".into()];
    event.removed_ids = vec!["old".into()];

    record_stream_payload_metrics(&metrics, &event, 321);

    let snap = metrics.snapshot();
    assert_eq!(snap.ws_arbitrage_payload_bytes, 321);
    assert_eq!(snap.ws_arbitrage_top_ids, 2);
    assert_eq!(snap.ws_arbitrage_changed_ids, 1);
    assert_eq!(snap.ws_arbitrage_changed_rows, 0);
    assert_eq!(snap.ws_arbitrage_removed_ids, 1);
}

#[test]
fn snapshot_payload_value_reports_serialized_size() -> anyhow::Result<()> {
    let event = stream_event(&["a", "b"]);

    let (_payload, payload_bytes) = snapshot_payload_value(&event).map_err(anyhow::Error::msg)?;

    assert!(payload_bytes > 0);
    Ok(())
}

#[test]
fn snapshot_task_outcome_keeps_publish_failure_visible() -> anyhow::Result<()> {
    let outcome = snapshot_task_outcome(
        false,
        Err(format!(
            "{ARBITRAGE_STREAM_SERIALIZE_FAILED}: encode payload failed"
        )),
    );

    let error = match outcome {
        Ok(()) => return Err(anyhow::anyhow!("publish failure must fail task outcome")),
        Err(error) => error,
    };
    assert!(error.contains("opportunity history append failed"));
    assert!(error.contains(ARBITRAGE_STREAM_SERIALIZE_FAILED));
    Ok(())
}

fn stream_event(ids: &[&str]) -> shared_types::OpportunityStreamEvent {
    let cached_at = chrono::Utc::now();
    shared_types::OpportunityStreamEvent {
        event: shared_types::OpportunityStreamEventKind::SnapshotInvalidated,
        snapshot_id: "test-snapshot".into(),
        scope_meta: shared_types::OpportunityQueryScopeMeta::default(),
        changed_ids: Vec::new(),
        changed_rows: Vec::new(),
        removed_ids: Vec::new(),
        top_ids: ids.iter().map(|id| (*id).to_owned()).collect(),
        windows: Vec::new(),
        main_p0_counts: shared_types::OpportunityCountBreakdown::default(),
        registry_counts: shared_types::OpportunityCountBreakdown::default(),
        meta: shared_types::OpportunityScanMeta::default(),
        status: shared_types::OpportunityEnvelopeStatus::Fresh,
        scope: shared_types::OpportunityEnvelopeScope::MainP0,
        query_key: "test".into(),
        source: "test".into(),
        cached_at,
        observed_at_ms: cached_at.timestamp_millis(),
        freshness_ms: None,
        retry_after_ms: None,
        error: None,
        partial_failures: Vec::new(),
    }
}

fn product_rows(rows: &[(&str, f64)]) -> HashMap<String, shared_types::OpportunityListRow> {
    rows.iter()
        .map(|(id, net_yield)| ((*id).to_owned(), product_row(id, *net_yield)))
        .collect()
}

fn product_row(id: &str, net_yield: f64) -> shared_types::OpportunityListRow {
    shared_types::OpportunityListRow {
        id: id.to_owned(),
        symbol: "BTC".into(),
        strategy_kind: Some(shared_types::StrategyKind::PerpCross),
        strategy_category: Some(shared_types::StrategyCategory::Futures),
        type_label: "永续跨所".into(),
        spot_leg_mode: None,
        long_leg: product_leg("binance"),
        short_leg: product_leg("okx"),
        metrics: shared_types::OpportunityListMetrics {
            score: 0.0,
            risk_level: shared_types::RiskLevel::Medium,
            net_single_yield: net_yield,
            annualized_funding_bps: None,
            one_cycle_net_bps: Some(net_yield * 10_000.0),
            time_to_settlement_ms: 1_000,
            settlement_countdown_seconds: Some(1),
            liquidity_score: 0.0,
        },
        cost: shared_types::OpportunityListCost::default(),
        execution: shared_types::OpportunityListExecution {
            eligible: false,
            blockers: Vec::new(),
            optimal_position: 0.0,
            max_position: 0.0,
        },
        data_source: "test".into(),
        updated_at: chrono::Utc::now(),
    }
}

fn product_leg(venue: &str) -> shared_types::OpportunityListLeg {
    shared_types::OpportunityListLeg {
        venue: venue.into(),
        action: venue.into(),
        price: Some(100.0),
        market_evidence: None,
        funding: None,
    }
}
