use chrono::Utc;
use shared_types::arbitrage::{OpportunityListFilterMeta, OpportunityListRequestMeta};
use shared_types::{
    MarketDataCoverage, MarketDataHealth, MarketDataQuality, MarketDataSourceKind,
    OpportunityCountBreakdown, OpportunityEnvelopeScope, OpportunityEnvelopeStatus,
    OpportunityLegMarketEvidence, OpportunityListCost, OpportunityListEnvelope,
    OpportunityListExecution, OpportunityListLeg, OpportunityListMetrics, OpportunityListPage,
    OpportunityListRow, OpportunityListSortKey, OpportunityQueryScopeMeta, OpportunityScanMeta,
    OpportunityStreamEvent, OpportunityStreamEventKind, RiskLevel, StrategyCategory, StrategyKind,
};
use std::collections::HashMap;

const LIST_PAGE_ROWS: usize = 120;
const DELTA_ROWS: usize = 25;
const STREAM_EVENT_MAX_BYTES: usize = 12 * 1024;
const STREAM_DELTA_MAX_BYTES: usize = 48 * 1024;
const LIST_PAGE_MAX_BYTES: usize = 180 * 1024;

#[test]
fn opportunity_stream_event_fits_hot_path_payload_budget() -> serde_json::Result<()> {
    let event = stream_event();
    let value = serde_json::to_value(&event)?;
    assert!(value.get("opportunities").is_none());
    assert!(value.get("rows").is_none());

    let bytes = serde_json::to_vec(&event)?;
    assert!(
        bytes.len() <= STREAM_EVENT_MAX_BYTES,
        "stream event is {} bytes; budget is {} bytes",
        bytes.len(),
        STREAM_EVENT_MAX_BYTES
    );
    Ok(())
}

#[test]
fn opportunity_stream_delta_rows_fit_hot_path_payload_budget() -> serde_json::Result<()> {
    let mut event = stream_event();
    event.changed_ids = (0..DELTA_ROWS)
        .map(|idx| format!("perp-cross-{idx}"))
        .collect();
    event.changed_rows = (0..DELTA_ROWS).map(list_row).collect();

    let value = serde_json::to_value(&event)?;
    assert!(value.get("opportunities").is_none());
    assert_eq!(
        value["changedRows"].as_array().map(Vec::len),
        Some(DELTA_ROWS)
    );

    let bytes = serde_json::to_vec(&event)?;
    assert!(
        bytes.len() <= STREAM_DELTA_MAX_BYTES,
        "stream delta event is {} bytes; budget is {} bytes",
        bytes.len(),
        STREAM_DELTA_MAX_BYTES
    );
    Ok(())
}

#[test]
fn opportunity_list_page_fits_hot_path_payload_budget() -> serde_json::Result<()> {
    let envelope = list_envelope(LIST_PAGE_ROWS);
    let value = serde_json::to_value(&envelope)?;
    assert!(value.get("opportunities").is_none());
    assert_eq!(value["rows"].as_array().map(Vec::len), Some(LIST_PAGE_ROWS));

    let bytes = serde_json::to_vec(&envelope)?;
    assert!(
        bytes.len() <= LIST_PAGE_MAX_BYTES,
        "list page is {} bytes; budget is {} bytes",
        bytes.len(),
        LIST_PAGE_MAX_BYTES
    );
    Ok(())
}

fn stream_event() -> OpportunityStreamEvent {
    let cached_at = Utc::now();
    OpportunityStreamEvent {
        event: OpportunityStreamEventKind::SnapshotInvalidated,
        snapshot_id: "snapshot:250:240".into(),
        scope_meta: scope_meta(LIST_PAGE_ROWS),
        changed_ids: Vec::new(),
        changed_rows: Vec::new(),
        removed_ids: Vec::new(),
        top_ids: (0..LIST_PAGE_ROWS)
            .map(|idx| format!("perp-cross-{idx}"))
            .collect(),
        windows: Vec::new(),
        main_p0_counts: counts(LIST_PAGE_ROWS),
        registry_counts: counts(LIST_PAGE_ROWS),
        meta: scan_meta(),
        status: OpportunityEnvelopeStatus::Fresh,
        scope: OpportunityEnvelopeScope::MainP0,
        query_key: "scope=main_p0;strategy=perp_cross;symbol=*".into(),
        source: "snapshot".into(),
        cached_at,
        observed_at_ms: cached_at.timestamp_millis(),
        freshness_ms: Some(100),
        retry_after_ms: None,
        error: None,
        partial_failures: Vec::new(),
    }
}

fn list_envelope(rows: usize) -> OpportunityListEnvelope {
    let cached_at = Utc::now();
    OpportunityListEnvelope {
        rows: (0..rows).map(list_row).collect(),
        page: OpportunityListPage {
            page_size: rows,
            start_offset: 0,
            returned_count: rows,
            total_rows: 240,
            has_next_page: true,
            next_cursor: Some("v1:120:0123456789abcdef".into()),
            previous_cursor: None,
            last_cursor: Some("v1:120:0123456789abcdef".into()),
            sort_key: OpportunityListSortKey::Score,
            snapshot_id: "snapshot:250:240".into(),
        },
        request_meta: OpportunityListRequestMeta {
            fast: true,
            fresh: false,
            filter: OpportunityListFilterMeta {
                scope: OpportunityEnvelopeScope::MainP0,
                strategy_kinds: vec![StrategyKind::PerpCross],
                symbol: None,
                min_yield: None,
            },
            sort_key: OpportunityListSortKey::Score,
            requested_page_size: Some(rows),
            applied_page_size: rows,
            max_page_size: LIST_PAGE_ROWS,
        },
        scope_meta: scope_meta(rows),
        main_p0_counts: counts(240),
        registry_counts: counts(240),
        meta: scan_meta(),
        status: OpportunityEnvelopeStatus::Fresh,
        scope: OpportunityEnvelopeScope::MainP0,
        query_key: "scope=main_p0;strategy=perp_cross;symbol=*;pageSize=120".into(),
        source: "snapshot".into(),
        cached_at,
        observed_at_ms: cached_at.timestamp_millis(),
        freshness_ms: Some(100),
        retry_after_ms: None,
        error: None,
        partial_failures: Vec::new(),
        instrument_coverage_diagnostics: String::new(),
    }
}

fn list_row(idx: usize) -> OpportunityListRow {
    OpportunityListRow {
        id: format!("perp-cross-{idx}"),
        symbol: format!("SYM{idx}"),
        strategy_kind: Some(StrategyKind::PerpCross),
        strategy_category: Some(StrategyCategory::Futures),
        type_label: "Perp Cross".into(),
        spot_leg_mode: None,
        long_leg: leg("binance", "long", 100.0),
        short_leg: leg("okx", "short", 100.2),
        metrics: OpportunityListMetrics {
            score: 92.0,
            risk_level: RiskLevel::Low,
            net_single_yield: 0.18,
            annualized_funding_bps: Some(320.0),
            one_cycle_net_bps: Some(12.0),
            time_to_settlement_ms: 2_400_000,
            settlement_countdown_seconds: Some(2_400),
            liquidity_score: 0.82,
        },
        cost: OpportunityListCost {
            verified: true,
            gross_edge_bps: 18.0,
            total_cost_bps: 6.0,
            wear_bps: 2.0,
            one_cycle_net_bps: Some(12.0),
            one_cycle_covers_cost: true,
            breakeven_periods: 1,
            breakeven_hours: 8.0,
            recommended_hold_hours: 8.0,
            net_bps_at_recommended_hold: 12.0,
            fee_evidence_count: 2,
            fee_evidence_complete: true,
            fee_evidence_ids: vec![
                format!("fee:binance:perp:{idx}"),
                format!("fee:okx:perp:{idx}"),
            ],
            one_cycle_penalty: 0.0,
        },
        execution: OpportunityListExecution {
            eligible: true,
            blockers: Vec::new(),
            optimal_position: 1_000.0,
            max_position: 2_500.0,
        },
        data_source: "market-data-cache".into(),
        updated_at: Utc::now(),
    }
}

fn leg(venue: &str, action: &str, price: f64) -> OpportunityListLeg {
    OpportunityListLeg {
        venue: venue.into(),
        action: action.into(),
        price: Some(price),
        market_evidence: Some(OpportunityLegMarketEvidence {
            venue: venue.into(),
            symbol: "BTCUSDT".into(),
            price: Some(price),
            health: MarketDataHealth {
                quality: MarketDataQuality::Fresh,
                source: MarketDataSourceKind::WsPush,
                freshness_ms: Some(50),
                retry_after_ms: None,
                last_error: None,
                observed_at_ms: 1_780_185_600_000,
                coverage: Some(MarketDataCoverage::new(1, 1)),
                problem: None,
            },
        }),
        funding: None,
    }
}

fn scan_meta() -> OpportunityScanMeta {
    OpportunityScanMeta {
        candidate_count: 250,
        emitted_count: 240,
        scan_ms: 12,
        publish_ms: 1,
        ..OpportunityScanMeta::default()
    }
}

fn scope_meta(rows: usize) -> OpportunityQueryScopeMeta {
    OpportunityQueryScopeMeta {
        global_total_count: 240,
        strategy_scope_count: 240,
        symbol_scope_count: 240,
        filtered_count: 240,
        page_count: 2,
        candidate_count: 250,
        emitted_count: rows,
    }
}

fn counts(total: usize) -> OpportunityCountBreakdown {
    let mut strategy_counts = HashMap::new();
    strategy_counts.insert(StrategyKind::PerpCross, total);
    OpportunityCountBreakdown {
        total_count: total,
        executable_count: total,
        strategy_counts: strategy_counts.clone(),
        executable_strategy_counts: strategy_counts,
    }
}
