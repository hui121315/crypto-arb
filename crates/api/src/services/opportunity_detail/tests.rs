use super::*;
use common::config::AppConfig;
use shared_types::{
    ArbitrageType, HistoryResponse, MarketDataCoverage, MarketDataHealth, MarketDataQuality,
    MarketDataSourceKind, Recommendation, RiskLevel, StrategyKind,
};

mod orderbook;

#[tokio::test]
async fn missing_opportunity_returns_expired_problem() -> anyhow::Result<()> {
    let state = AppState::new(test_config()).await?;

    let error = detail(
        &state,
        "missing".to_owned(),
        OpportunityDetailRequest::default(),
    )
    .await
    .err()
    .ok_or_else(|| anyhow::anyhow!("missing id should fail"))?;

    assert_eq!(error.code(), codes::OPPORTUNITY_EXPIRED);
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn disabled_history_enters_partial_failures_without_failing_detail() -> anyhow::Result<()> {
    let state = AppState::new(test_config()).await?;
    state.cache_arbitrage_report(shared_types::OpportunityScanReport {
        opportunities: vec![test_opp()],
        meta: shared_types::OpportunityScanMeta::default(),
    });

    let envelope = detail(
        &state,
        "opp-1".to_owned(),
        OpportunityDetailRequest::default(),
    )
    .await?;

    assert_eq!(envelope.opportunity.id, "opp-1");
    assert_eq!(envelope.status, OpportunityEnvelopeStatus::Degraded);
    assert!(envelope
        .partial_failures
        .iter()
        .any(|problem| problem.code == codes::HISTORY_STORE_UNAVAILABLE));
    assert_eq!(
        envelope.long_orderbook.health.quality,
        MarketDataQuality::Unverified
    );
    assert_eq!(
        envelope
            .long_orderbook
            .health
            .coverage
            .as_ref()
            .map(|coverage| coverage.requested),
        Some(0)
    );
    assert!(envelope.long_orderbook.health.problem.is_none());
    Ok(())
}

#[tokio::test]
async fn current_atomic_snapshot_resolves_detail() -> anyhow::Result<()> {
    let state = AppState::new(test_config()).await?;
    state.cache_arbitrage_report(shared_types::OpportunityScanReport {
        opportunities: vec![test_opp()],
        meta: shared_types::OpportunityScanMeta::default(),
    });
    let opportunity = opportunity_by_id(&state, "opp-1")?;

    assert_eq!(opportunity.id, "opp-1");
    Ok(())
}

#[tokio::test]
async fn non_p0_opportunity_is_blocked_from_main_detail() -> anyhow::Result<()> {
    let state = AppState::new(test_config()).await?;
    let mut opportunity = test_opp();
    opportunity.strategy_kind = Some(StrategyKind::Triangular);
    state.cache_arbitrage_report(shared_types::OpportunityScanReport {
        opportunities: vec![opportunity],
        meta: shared_types::OpportunityScanMeta::default(),
    });

    let error = opportunity_by_id(&state, "opp-1")
        .err()
        .ok_or_else(|| anyhow::anyhow!("diagnostic strategy should be blocked"))?;

    assert_eq!(error.code(), codes::OPPORTUNITY_NOT_EXECUTABLE);
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[test]
fn detail_envelope_uses_max_retry_after_from_partial_failures() {
    let envelope = envelope(
        test_opp(),
        DetailSegments {
            long_orderbook: empty_market_envelope::<OrderBookInfo>(),
            short_orderbook: empty_market_envelope::<OrderBookInfo>(),
            history: empty_history_response(),
            long_index_composition: degraded_market_envelope::<IndexCompositionSnapshot>(
                "LONG_INDEX_RATE_LIMITED",
                2_000,
            ),
            short_index_composition: degraded_market_envelope::<IndexCompositionSnapshot>(
                "SHORT_INDEX_RATE_LIMITED",
                5_000,
            ),
        },
        detail_request_meta(OpportunityDetailRequest::default()),
        Vec::new(),
        common::time::now_ms(),
    );

    assert_eq!(envelope.status, OpportunityEnvelopeStatus::Degraded);
    assert_eq!(envelope.retry_after_ms, Some(5_000));
    assert_eq!(
        envelope.error.as_ref().map(|problem| problem.code.as_str()),
        Some("LONG_INDEX_RATE_LIMITED")
    );
    assert_eq!(envelope.partial_failures.len(), 2);
}

#[test]
fn opportunity_detail_request_meta_reports_clamps_as_typed_problems() {
    let query = OpportunityDetailRequest {
        depth: Some(500),
        history_limit: Some(0),
    };
    let meta = detail_request_meta(query);
    let problems = detail_query_problems(query, meta);

    assert_eq!(meta.orderbook_depth.requested, Some(500));
    assert_eq!(meta.orderbook_depth.applied, MAX_ORDERBOOK_DEPTH as usize);
    assert_eq!(meta.history_limit.requested, Some(0));
    assert_eq!(meta.history_limit.applied, 1);
    assert_eq!(problems.len(), 2);
    assert!(problems
        .iter()
        .all(|problem| problem.code == codes::LIST_LIMIT_CLAMPED));
}

fn test_config() -> AppConfig {
    let mut config = AppConfig::default();
    config.history.enabled = false;
    config
}

fn degraded_market_envelope<T>(
    code: &'static str,
    retry_after_ms: u64,
) -> MarketDataEnvelope<Option<T>> {
    let problem = ApiProblem::new(code, "rate limited")
        .with_retry_after_ms(Some(retry_after_ms))
        .with_source("market_data");
    MarketDataEnvelope {
        data: None,
        health: MarketDataHealth {
            quality: MarketDataQuality::RateLimited,
            source: MarketDataSourceKind::RestBaseline,
            freshness_ms: None,
            retry_after_ms: Some(retry_after_ms),
            last_error: Some("rate limited".into()),
            observed_at_ms: 1,
            coverage: Some(MarketDataCoverage::new(1, 0)),
            problem: Some(problem),
        },
        retry_after_ms: Some(retry_after_ms),
        row_cap: None,
        row_evidence: Vec::new(),
        fanout: Vec::new(),
    }
}

fn empty_market_envelope<T>() -> MarketDataEnvelope<Option<T>> {
    MarketDataEnvelope {
        data: None,
        health: MarketDataHealth {
            quality: MarketDataQuality::Fresh,
            source: MarketDataSourceKind::LocalCache,
            freshness_ms: None,
            retry_after_ms: None,
            last_error: None,
            observed_at_ms: 1,
            coverage: Some(MarketDataCoverage::new(0, 0)),
            problem: None,
        },
        retry_after_ms: None,
        row_cap: None,
        row_evidence: Vec::new(),
        fanout: Vec::new(),
    }
}

fn empty_history_response() -> HistoryResponse<OpportunityHistoryRow> {
    HistoryResponse {
        count: 0,
        rows: Vec::new(),
        page: None,
        row_cap: None,
        backend_status: Default::default(),
        storage_health: None,
        source: "memory".into(),
        observed_at_ms: 1,
        latest_at_ms: None,
        freshness_ms: None,
        problem: None,
        retry_after_ms: None,
        problems: Vec::new(),
    }
}

fn test_opp() -> ArbitrageOpportunityDto {
    ArbitrageOpportunityDto {
        id: "opp-1".into(),
        symbol: "BTC".into(),
        arb_type: ArbitrageType::CrossExchange,
        type_label: "永续跨所".into(),
        long_exchange: "binance".into(),
        short_exchange: "okx".into(),
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
        risk_level: RiskLevel::Low,
        volatility: 0.0,
        sharpe_ratio: 0.0,
        score: 80.0,
        score_breakdown: None,
        ranking_key: None,
        recommendation: Recommendation::Hold,
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
        execution_eligible: true,
        execution_blockers: Vec::new(),
        execution_cost: None,
        index_composition: None,
        strategy_kind: Some(StrategyKind::PerpCross),
        strategy_category: Some(StrategyKind::PerpCross.category()),
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
    }
}
