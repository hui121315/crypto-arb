use super::*;

mod envelope;
mod fee_evidence;
mod list;
mod paging_contract;
mod stale;
mod stream;
mod ticket_readiness;

mod counts;
pub(crate) use counts::counts;

#[test]
fn scan_report_collapses_duplicate_ids_and_blocks_the_retained_row() {
    let scan_started_at = chrono::Utc::now();
    let mut first = dto("duplicate", StrategyKind::PerpPriceSpread, true);
    first.long_price = Some(100.0);
    let mut second = first.clone();
    second.long_price = Some(101.0);
    let mut report = OpportunityScanReport {
        opportunities: vec![first, second],
        ..OpportunityScanReport::default()
    };

    let duplicate_count = normalize_scan_report(&mut report, scan_started_at);

    assert_eq!(duplicate_count, 1);
    assert_eq!(report.meta.scan_started_at, Some(scan_started_at));
    assert_eq!(report.opportunities.len(), 1);
    assert_eq!(report.opportunities[0].long_price, Some(100.0));
    assert!(!report.opportunities[0].execution_eligible);
    assert_eq!(
        report.opportunities[0].execution_blockers,
        [DUPLICATE_ID_BLOCKER]
    );
}

fn assert_stale_snapshot_contract(
    status: OpportunityEnvelopeStatus,
    cached_at: DateTime<Utc>,
    observed_at_ms: i64,
    freshness_ms: Option<i64>,
    retry_after_ms: Option<u64>,
    error: Option<&ApiProblem>,
) -> Result<(), &'static str> {
    assert_eq!(status, OpportunityEnvelopeStatus::Stale);
    let freshness_ms = freshness_ms.ok_or("stale snapshots must report freshness")?;
    assert_eq!(freshness_ms, observed_at_ms - cached_at.timestamp_millis());
    assert!(freshness_ms > snapshot_health::SNAPSHOT_STALE_AFTER_MS);
    assert_eq!(retry_after_ms, Some(WARMING_RETRY_AFTER_MS));

    let problem = error.ok_or("stale snapshots must report a typed problem")?;
    assert_eq!(problem.code, codes::OPPORTUNITY_SNAPSHOT_STALE);
    assert_eq!(problem.source.as_deref(), Some("arbitrage-snapshot"));
    assert_eq!(problem.retry_after_ms, Some(WARMING_RETRY_AFTER_MS));
    let details = problem
        .details
        .as_ref()
        .ok_or("stale problem must report details")?;
    assert_eq!(
        details["cachedAtMs"].as_i64(),
        Some(cached_at.timestamp_millis())
    );
    assert_eq!(details["observedAtMs"].as_i64(), Some(observed_at_ms));
    assert_eq!(details["freshnessMs"].as_i64(), Some(freshness_ms));
    assert_eq!(
        details["staleAfterMs"].as_i64(),
        Some(snapshot_health::SNAPSHOT_STALE_AFTER_MS)
    );
    assert_eq!(
        details["refreshIntervalMs"].as_u64(),
        Some(WARMING_RETRY_AFTER_MS)
    );
    assert_eq!(details["lastScanMs"].as_u64(), Some(0));
    Ok(())
}

fn request_meta(window: OpportunityListWindow) -> OpportunityListRequestMeta {
    OpportunityListRequestMeta {
        fast: true,
        fresh: false,
        filter: shared_types::arbitrage::OpportunityListFilterMeta {
            scope: OpportunityEnvelopeScope::MainP0,
            strategy_kinds: vec![StrategyKind::PerpCross],
            symbol: None,
            min_yield: None,
        },
        sort_key: window.sort_key(),
        requested_page_size: window.requested_page_size(),
        applied_page_size: window.page_size(),
        max_page_size: window.max_page_size(),
    }
}

fn dto(id: &str, kind: StrategyKind, execution_eligible: bool) -> ArbitrageOpportunityDto {
    let mut dto = ArbitrageOpportunityDto {
        id: id.into(),
        symbol: "BTC".into(),
        arb_type: shared_types::ArbitrageType::CrossExchange,
        type_label: kind.label_zh().into(),
        long_exchange: "a".into(),
        short_exchange: "b".into(),
        spread_8h: 0.0,
        long_rate_8h: 0.0,
        short_rate_8h: 0.0,
        long_rate: 0.0,
        short_rate: 0.0,
        single_yield: 0.0,
        net_single_yield: 0.0,
        raw_single_yield: 0.0,
        settlement_interval: 8,
        risk_adjusted_yield: 0.0,
        trading_cost_rate: 0.0,
        min_holding_periods: 1,
        risk_level: shared_types::RiskLevel::Low,
        volatility: 0.0,
        sharpe_ratio: 0.0,
        score: 0.0,
        score_breakdown: None,
        ranking_key: None,
        recommendation: shared_types::Recommendation::Hold,
        optimal_position: 0.0,
        max_position: 0.0,
        liquidity_score: 0.0,
        volume_24h: 0.0,
        long_volume_24h: 0.0,
        short_volume_24h: 0.0,
        data_source: "test".into(),
        confidence: 0.0,
        updated_at: chrono::Utc::now(),
        long_funding_interval: 8,
        short_funding_interval: 8,
        settlement_time_diff: false,
        strategy_description: String::new(),
        long_action: String::new(),
        short_action: String::new(),
        long_next_funding_time: 0,
        short_next_funding_time: 0,
        time_to_settlement_ms: 0,
        is_snipe_ready: false,
        long_price: None,
        short_price: None,
        long_leg_market_evidence: None,
        short_leg_market_evidence: None,
        quote_conversions: Vec::new(),
        price_deviation: None,
        basis_spread: None,
        basis_annual_cost: None,
        risk_warnings: Vec::new(),
        execution_eligible,
        execution_blockers: Vec::new(),
        execution_cost: None,
        index_composition: None,
        strategy_kind: Some(kind),
        strategy_category: Some(kind.category()),
        spot_leg_mode: None,
        basis_bps: None,
        annualized_funding_bps: None,
        triangular_path: None,
        onchain_metadata: None,
        predicted_next_funding: None,
        funding_diff_window: None,
        funding_diff_windows: Vec::new(),
        borrow_cost_bps_per_day: None,
        funding_window_alignment_minutes: None,
        funding_cap_distance_bps: None,
        min_hold_hours: None,
        settlement_countdown_seconds: None,
    };
    if execution_eligible {
        make_ready(&mut dto);
    }
    dto
}

fn make_ready(dto: &mut ArbitrageOpportunityDto) {
    dto.long_price = Some(100.0);
    dto.short_price = Some(100.1);
    dto.long_leg_market_evidence = Some(market_evidence("a"));
    dto.short_leg_market_evidence = Some(market_evidence("b"));
    dto.execution_cost = Some(fee_evidence::verified_cost());
}

fn market_evidence(venue: &str) -> shared_types::OpportunityLegMarketEvidence {
    let observed_at_ms = chrono::Utc::now().timestamp_millis();
    shared_types::OpportunityLegMarketEvidence {
        venue: venue.into(),
        symbol: "BTCUSDT".into(),
        price: Some(100.0),
        health: shared_types::MarketDataHealth {
            quality: shared_types::MarketDataQuality::Fresh,
            source: shared_types::MarketDataSourceKind::WsPush,
            freshness_ms: Some(20),
            retry_after_ms: None,
            last_error: None,
            observed_at_ms,
            coverage: Some(shared_types::MarketDataCoverage::new(1, 1)),
            problem: None,
        },
    }
}

fn rate_limited_market_meta(retry_after_ms: u64) -> OpportunityScanMeta {
    OpportunityScanMeta {
        market_data_problem_count: 1,
        market_data_status: Some(shared_types::MarketDataSnapshotStatus {
            observed_at_ms: 1,
            rows: vec![shared_types::MarketDataSnapshotStatusRow {
                venue: crate::services::market_data::cache::MARKET_AGGREGATE_VENUE.into(),
                operation: shared_types::MarketDataSnapshotOperation::PerpTickers,
                health: shared_types::MarketDataHealth {
                    quality: shared_types::MarketDataQuality::RateLimited,
                    source: shared_types::MarketDataSourceKind::RestBaseline,
                    freshness_ms: None,
                    retry_after_ms: Some(retry_after_ms),
                    last_error: Some("rate limited".into()),
                    observed_at_ms: 1,
                    coverage: Some(shared_types::MarketDataCoverage::new(1, 0)),
                    problem: Some(
                        ApiProblem::new("MARKET_DATA_RATE_LIMITED", "rate limited")
                            .with_retry_after_ms(Some(retry_after_ms))
                            .with_source("rest_baseline"),
                    ),
                },
            }],
        }),
        ..OpportunityScanMeta::default()
    }
}

fn unsupported_metadata_meta() -> OpportunityScanMeta {
    OpportunityScanMeta {
        market_data_status: Some(shared_types::MarketDataSnapshotStatus {
            observed_at_ms: 1,
            rows: vec![shared_types::MarketDataSnapshotStatusRow {
                venue: "bybit".into(),
                operation: shared_types::MarketDataSnapshotOperation::Metadata,
                health: shared_types::MarketDataHealth {
                    quality: shared_types::MarketDataQuality::Unsupported,
                    source: shared_types::MarketDataSourceKind::RestBaseline,
                    freshness_ms: None,
                    retry_after_ms: None,
                    last_error: Some("adapter has no separate metadata cache to refresh".into()),
                    observed_at_ms: 1,
                    coverage: Some(shared_types::MarketDataCoverage::new(1, 0)),
                    problem: Some(
                        ApiProblem::new(
                            "MARKET_DATA_UNSUPPORTED",
                            "adapter has no separate metadata cache to refresh",
                        )
                        .with_source("rest_baseline"),
                    ),
                },
            }],
        }),
        ..OpportunityScanMeta::default()
    }
}

fn optional_index_problem_meta() -> OpportunityScanMeta {
    OpportunityScanMeta {
        market_data_status: Some(shared_types::MarketDataSnapshotStatus {
            observed_at_ms: 1,
            rows: vec![shared_types::MarketDataSnapshotStatusRow {
                venue: "binance".into(),
                operation: shared_types::MarketDataSnapshotOperation::IndexCompositions,
                health: shared_types::MarketDataHealth {
                    quality: shared_types::MarketDataQuality::Missing,
                    source: shared_types::MarketDataSourceKind::RestBaseline,
                    freshness_ms: None,
                    retry_after_ms: None,
                    last_error: Some("invalid index symbol".into()),
                    observed_at_ms: 1,
                    coverage: Some(shared_types::MarketDataCoverage::new(1, 0)),
                    problem: Some(
                        ApiProblem::new("MARKET_DATA_MISSING", "invalid index symbol")
                            .with_source("exchange"),
                    ),
                },
            }],
        }),
        ..OpportunityScanMeta::default()
    }
}
