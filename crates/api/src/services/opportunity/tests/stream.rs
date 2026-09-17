use super::super::paging::snapshot_id;
use super::*;

#[test]
fn stream_event_publishes_counts_without_rows() {
    let cached_at = Utc::now();
    let scan_started_at = cached_at - chrono::Duration::milliseconds(10);
    let rows = vec![
        dto("perp", StrategyKind::PerpCross, true),
        dto("spot", StrategyKind::SpotPerp, false),
        dto("spot-cross", StrategyKind::SpotCross, true),
    ];

    let event = stream_event(OpportunityStreamEventInput {
        source_rows: &rows,
        meta: OpportunityScanMeta {
            candidate_count: 3,
            emitted_count: 3,
            scan_started_at: Some(scan_started_at),
            funding_row_evidence: vec![funding_row_evidence()],
            ..OpportunityScanMeta::default()
        },
        cached_at,
        snapshot_id: None,
        source: "snapshot",
        status: OpportunityEnvelopeStatus::Fresh,
        scope: OpportunityEnvelopeScope::MainP0,
        query_key: "scope=main_p0".into(),
        retry_after_ms: None,
        error: None,
        full_window_rows: false,
    });

    assert_eq!(event.main_p0_counts.total_count, 3);
    assert_eq!(event.registry_counts.total_count, 3);
    assert_eq!(event.scope_meta.global_total_count, 3);
    assert_eq!(event.scope_meta.filtered_count, 2);
    assert_eq!(event.snapshot_id, snapshot_id(cached_at, &event.meta));
    assert_eq!(event.meta.scan_started_at, Some(scan_started_at));
    assert!(event.meta.funding_row_evidence.is_empty());
    assert_eq!(event.top_ids, vec!["perp", "spot-cross"]);
    assert_eq!(event.windows.len(), P0_EXECUTABLE_STRATEGY_KINDS.len() + 1);
}

#[test]
fn stream_event_caps_live_rows_to_the_product_first_page() {
    let rows = (0..(OPPORTUNITY_PRODUCT_PAGE_SIZE + 7))
        .map(|index| dto(&format!("opp-{index:03}"), StrategyKind::PerpCross, true))
        .collect::<Vec<_>>();
    let event = event(&rows, false);

    assert_eq!(event.top_ids.len(), OPPORTUNITY_PRODUCT_PAGE_SIZE);
    assert_eq!(event.top_ids.first().map(String::as_str), Some("opp-000"));
    assert_eq!(event.top_ids.last().map(String::as_str), Some("opp-049"));
}

#[test]
fn stream_replay_includes_strategy_rows_outside_the_combined_first_page() -> Result<(), &'static str>
{
    let mut rows = (0..(OPPORTUNITY_PRODUCT_PAGE_SIZE + 5))
        .map(|index| {
            let mut row = dto(&format!("perp-{index:03}"), StrategyKind::PerpCross, true);
            row.net_single_yield = 100.0 - index as f64;
            row
        })
        .collect::<Vec<_>>();
    let mut spot_cross = dto("spot-cross-only", StrategyKind::SpotCross, true);
    spot_cross.net_single_yield = 0.000_01;
    rows.push(spot_cross);

    let event = event(&rows, true);

    assert!(!event.top_ids.iter().any(|id| id == "spot-cross-only"));
    let window = event
        .windows
        .iter()
        .find(|window| window.strategy_kind == Some(StrategyKind::SpotCross))
        .ok_or("spot-cross stream window missing")?;
    assert_eq!(window.ids, ["spot-cross-only"]);
    assert!(event
        .changed_rows
        .iter()
        .any(|row| row.id == "spot-cross-only"));
    assert_eq!(event.changed_ids.len(), OPPORTUNITY_PRODUCT_PAGE_SIZE + 1);
    Ok(())
}

#[test]
fn stream_event_hides_a_candidate_that_turns_negative_after_confirmation_costs(
) -> Result<(), &'static str> {
    let mut row = dto("confirmed-negative", StrategyKind::SpotCross, true);
    row.net_single_yield = -0.000_01;
    if let Some(cost) = row.execution_cost.as_mut() {
        cost.one_cycle.net_bps = -0.1;
        cost.one_cycle.covers_round_trip_cost = false;
    }

    let event = event(&[row], true);

    assert!(event.top_ids.is_empty());
    assert!(event.changed_rows.is_empty());
    let spot_window = event
        .windows
        .iter()
        .find(|window| window.strategy_kind == Some(StrategyKind::SpotCross))
        .ok_or("spot-cross window missing")?;
    assert!(spot_window.ids.is_empty());
    assert_eq!(spot_window.scope_meta.strategy_scope_count, 1);
    assert_eq!(spot_window.scope_meta.filtered_count, 0);
    Ok(())
}

#[test]
fn stream_event_promotes_partial_failure_retry_after() {
    let event = stream_event(OpportunityStreamEventInput {
        source_rows: &[],
        meta: rate_limited_market_meta(2_000),
        cached_at: Utc::now(),
        snapshot_id: None,
        source: "snapshot",
        status: OpportunityEnvelopeStatus::Fresh,
        scope: OpportunityEnvelopeScope::MainP0,
        query_key: "scope=main_p0".into(),
        retry_after_ms: None,
        error: None,
        full_window_rows: false,
    });

    assert_eq!(event.status, OpportunityEnvelopeStatus::Degraded);
    assert_eq!(event.retry_after_ms, Some(2_000));
    assert_eq!(event.partial_failures[0].retry_after_ms, Some(2_000));
    assert!(event.meta.market_data_status.is_none());
}

fn event(rows: &[ArbitrageOpportunityDto], full_window_rows: bool) -> OpportunityStreamEvent {
    stream_event(OpportunityStreamEventInput {
        source_rows: rows,
        meta: OpportunityScanMeta::default(),
        cached_at: Utc::now(),
        snapshot_id: None,
        source: "snapshot",
        status: OpportunityEnvelopeStatus::Fresh,
        scope: OpportunityEnvelopeScope::MainP0,
        query_key: "scope=main_p0".into(),
        retry_after_ms: None,
        error: None,
        full_window_rows,
    })
}

fn funding_row_evidence() -> shared_types::MarketDataRowEvidence {
    shared_types::MarketDataRowEvidence {
        venue: "binance".into(),
        symbol: "BTCUSDT".into(),
        operation: shared_types::MarketDataSnapshotOperation::FundingRates,
        health: shared_types::MarketDataHealth {
            quality: shared_types::MarketDataQuality::Fresh,
            source: shared_types::MarketDataSourceKind::RestBaseline,
            freshness_ms: Some(1),
            retry_after_ms: None,
            last_error: None,
            observed_at_ms: 1,
            coverage: Some(shared_types::MarketDataCoverage::new(1, 1)),
            problem: None,
        },
    }
}
