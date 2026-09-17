use crate::panels::modules::funding_stats::FundingCycleStatsView;
use crate::panels::modules::index_composition::IndexCompositionView;
use crate::panels::modules::opportunity_view_model::OpportunityListViewModel;
use shared_types::{
    OpportunityLegMarketEvidence, OpportunityListPage, StrategyCategory, StrategyKind,
};
use std::sync::Arc;

use super::super::*;

pub(super) fn row(
    idx: usize,
    kind: StrategyKind,
    est_apr_pct: f64,
    depth_usd: f64,
) -> FuturesOpportunityRow {
    Arc::new(row_value(idx, kind, est_apr_pct, depth_usd))
}

pub(super) fn row_value(
    idx: usize,
    kind: StrategyKind,
    est_apr_pct: f64,
    _depth_usd: f64,
) -> FuturesOpportunity {
    let view = Arc::new(OpportunityListViewModel {
        id: format!("opp-{idx}"),
        snapshot_id: "test-snapshot".into(),
        pair: format!("SYM{idx}"),
        strategy_label: kind.label_zh().into(),
        strategy_kind: Some(kind),
        strategy_category: Some(kind.category()),
        spot_leg_mode: None,
        data_source: "test".into(),
        updated_at_ms: 0,
        long_venue: "binance".into(),
        short_venue: "okx".into(),
        long_leg: "binance 做多".into(),
        short_leg: "okx 做空".into(),
        long_price: "-".into(),
        short_price: "-".into(),
        long_market_evidence: None,
        short_market_evidence: None,
        long_market_evidence_raw: None,
        short_market_evidence_raw: None,
        long_funding: None,
        short_funding: None,
        net_edge: "0.000%".into(),
        net_basis_bps: 0.0,
        predicted_funding_bps: Some(0.0),
        est_apr_pct: Some(est_apr_pct),
        gross_one_cycle_bps: 0.0,
        gross_one_cycle: "+0.000%".into(),
        round_trip_cost: "0.000%".into(),
        cost_verified: true,
        cost_total_bps: 0.0,
        cost_wear_bps: 0.0,
        fee_evidence_count: 2,
        fee_evidence_complete: true,
        fee_evidence_ids: vec!["fee:binance:perp:vip0".into(), "fee:okx:perp:vip0".into()],
        one_cycle_penalty: 0.0,
        one_cycle_net: "+0.000%".into(),
        one_cycle_net_bps: 0.0,
        one_cycle_covers_cost: false,
        breakeven_periods: 1,
        breakeven_hours: 8.0,
        recommended_hold_hours: 16.0,
        net_bps_at_recommended_hold: 0.0,
        risk: "低".into(),
        risk_level: shared_types::RiskLevel::Low,
        settlement_countdown_seconds: Some(0),
        time_to_settlement_ms: 0,
        optimal_position: 0.0,
        max_position: 0.0,
        execution_eligible: false,
        execution_blockers: Vec::new(),
    });
    FuturesOpportunity {
        view,
        funding_curve: Vec::new(),
        funding_stats: FundingCycleStatsView::default(),
        index_composition: IndexCompositionView::default(),
        borrow_cost_bps_per_day: Some(0.0),
        funding_alignment_minutes: Some(0),
        funding_cap_distance_bps: Some(0.0),
        min_hold_hours: Some(0.0),
    }
}

pub(super) fn leg_evidence(venue: &str, symbol: &str) -> OpportunityLegMarketEvidence {
    OpportunityLegMarketEvidence {
        venue: venue.into(),
        symbol: symbol.into(),
        price: Some(100.0),
        health: shared_types::MarketDataHealth {
            quality: shared_types::MarketDataQuality::Fresh,
            source: shared_types::MarketDataSourceKind::WsPush,
            freshness_ms: Some(10),
            retry_after_ms: None,
            last_error: None,
            observed_at_ms: 1,
            coverage: None,
            problem: None,
        },
    }
}

pub(super) fn page(snapshot_id: &str) -> OpportunityListPage {
    OpportunityListPage {
        page_size: FUTURES_PAGE_SIZE,
        returned_count: 1,
        total_rows: 1,
        snapshot_id: snapshot_id.into(),
        ..OpportunityListPage::default()
    }
}

pub(super) fn dto(id: &str, kind: StrategyKind) -> shared_types::ArbitrageOpportunityDto {
    let mut dto = dto_template();
    dto.id = id.into();
    dto.strategy_kind = Some(kind);
    dto.strategy_category = Some(kind.category());
    dto
}

pub(super) fn dto_template() -> shared_types::ArbitrageOpportunityDto {
    shared_types::ArbitrageOpportunityDto {
        id: "test".into(),
        symbol: "BTC".into(),
        arb_type: shared_types::ArbitrageType::CrossExchange,
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
        execution_eligible: true,
        execution_blockers: Vec::new(),
        execution_cost: None,
        index_composition: None,
        strategy_kind: Some(StrategyKind::PerpCross),
        strategy_category: Some(StrategyCategory::Futures),
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
