use super::super::*;
use crate::panels::modules::funding_stats::FundingCycleStatsView;
use crate::panels::modules::index_composition::{IndexCompositionDetailView, IndexCompositionView};
use crate::panels::modules::opportunity_view_model::view_models_from_rows;
use shared_types::StrategyCategory;
use shared_types::{MarketDataEnvelope, OrderBookInfo};
use shared_types::{
    MarketDataHealth, MarketDataQuality, MarketDataSourceKind, OpportunityLegMarketEvidence,
    OpportunityListCost, OpportunityListExecution, OpportunityListLeg, OpportunityListMetrics,
    OpportunityListRow as SharedOpportunityListRow, RiskLevel,
};
use shared_types::{OpportunityListPage, StrategyKind};

pub(super) fn orderbook(exchange: &str) -> OrderBookInfo {
    OrderBookInfo {
        symbol: "MU".into(),
        exchange: exchange.into(),
        bids: vec![[100.0, 1.0]],
        asks: vec![[101.0, 1.0]],
        timestamp: 1,
    }
}

pub(super) fn orderbook_envelope(
    data: Option<OrderBookInfo>,
) -> MarketDataEnvelope<Option<OrderBookInfo>> {
    MarketDataEnvelope {
        data,
        health: market_health(MarketDataQuality::Fresh),
        retry_after_ms: None,
        row_cap: None,
        row_evidence: Vec::new(),
        fanout: Vec::new(),
    }
}

pub(super) fn row_ref(
    id: &str,
    pair: &str,
    buy_venue: &str,
    execution_eligible: bool,
) -> OpportunityRow {
    let snapshot_id = format!("{id}:{pair}:{buy_venue}:{execution_eligible}:8");
    row_ref_with_net(&snapshot_id, id, pair, buy_venue, execution_eligible, 8.0)
}

pub(super) fn row_ref_with_spot_leg_mode(
    snapshot_id: &str,
    id: &str,
    pair: &str,
    buy_venue: &str,
    mode: shared_types::SpotLegMode,
) -> OpportunityRow {
    let mut row = row(id, pair, buy_venue, true, 8.0);
    row.spot_leg_mode = Some(mode);
    view_models_from_rows(snapshot_id, vec![row]).remove(0)
}

pub(super) fn row_ref_with_net(
    snapshot_id: &str,
    id: &str,
    pair: &str,
    buy_venue: &str,
    execution_eligible: bool,
    one_cycle_net_bps: f64,
) -> OpportunityRow {
    view_models_from_rows(
        snapshot_id,
        vec![row(
            id,
            pair,
            buy_venue,
            execution_eligible,
            one_cycle_net_bps,
        )],
    )
    .remove(0)
}

pub(super) fn page(snapshot_id: &str) -> OpportunityListPage {
    OpportunityListPage {
        page_size: OPPORTUNITY_PAGE_SIZE,
        returned_count: 1,
        total_rows: 1,
        snapshot_id: snapshot_id.into(),
        ..OpportunityListPage::default()
    }
}

pub(super) fn row(
    id: &str,
    pair: &str,
    buy_venue: &str,
    execution_eligible: bool,
    one_cycle_net_bps: f64,
) -> SharedOpportunityListRow {
    SharedOpportunityListRow {
        id: id.into(),
        symbol: pair.into(),
        strategy_kind: Some(StrategyKind::PerpCross),
        strategy_category: Some(StrategyCategory::Futures),
        type_label: "永续跨所".into(),
        spot_leg_mode: None,
        long_leg: leg(buy_venue, pair, "做多永续"),
        short_leg: leg("kucoin", pair, "做空永续"),
        metrics: OpportunityListMetrics {
            score: 0.0,
            risk_level: RiskLevel::Low,
            net_single_yield: 0.001,
            annualized_funding_bps: Some(1095.0),
            one_cycle_net_bps: Some(one_cycle_net_bps),
            time_to_settlement_ms: 60_000,
            settlement_countdown_seconds: Some(60),
            liquidity_score: 80.0,
        },
        cost: OpportunityListCost {
            verified: true,
            gross_edge_bps: one_cycle_net_bps + 1.0,
            total_cost_bps: 1.0,
            wear_bps: 0.2,
            one_cycle_net_bps: Some(one_cycle_net_bps),
            one_cycle_covers_cost: true,
            breakeven_periods: 1,
            breakeven_hours: 8.0,
            recommended_hold_hours: 8.0,
            net_bps_at_recommended_hold: one_cycle_net_bps,
            fee_evidence_count: 2,
            fee_evidence_complete: true,
            fee_evidence_ids: vec!["fee:binance:perp:vip0".into(), "fee:okx:perp:vip0".into()],
            one_cycle_penalty: 0.0,
        },
        execution: OpportunityListExecution {
            eligible: execution_eligible,
            blockers: if execution_eligible {
                Vec::new()
            } else {
                vec!["blocked".into()]
            },
            optimal_position: 10_000.0,
            max_position: 20_000.0,
        },
        data_source: "test".into(),
        updated_at: chrono::Utc::now(),
    }
}

pub(super) fn leg(venue: &str, pair: &str, action: &str) -> OpportunityListLeg {
    OpportunityListLeg {
        venue: venue.into(),
        action: format!("{venue} {action}"),
        price: Some(100.0),
        market_evidence: Some(OpportunityLegMarketEvidence {
            venue: venue.into(),
            symbol: pair.into(),
            price: Some(100.0),
            health: MarketDataHealth {
                quality: MarketDataQuality::Fresh,
                source: MarketDataSourceKind::WsPush,
                freshness_ms: Some(10),
                retry_after_ms: None,
                last_error: None,
                observed_at_ms: 1,
                coverage: None,
                problem: None,
            },
        }),
        funding: None,
    }
}

pub(super) fn detail_fixture(id: &str, pair: &str) -> OpportunityDetail {
    OpportunityDetail {
        id: id.into(),
        pair: pair.into(),
        domain: "永续跨所".into(),
        market_scope: "套利".into(),
        risk: "低".into(),
        net_edge: "+0.010%".into(),
        gross_one_cycle: "+0.020%".into(),
        round_trip_cost: "0.010%".into(),
        cost_verified: true,
        execution_eligible: true,
        one_cycle_net: "+0.010%".into(),
        one_cycle_net_bps: 1.0,
        funding_stats: FundingCycleStatsView::default(),
        index_composition: IndexCompositionView::default(),
        index_composition_detail: IndexCompositionDetailView::default(),
        reason: "test".into(),
        books: Vec::new(),
        history_health: "历史 memory · 0条".into(),
        history: Vec::new(),
        section_evidence: Vec::new(),
    }
}

pub(super) fn seed(id: &str, pair: &str) -> OpportunityDetailSeed {
    OpportunityDetailSeed {
        id: id.into(),
        pair: pair.into(),
        domain: "永续跨所".into(),
        market_scope: "套利".into(),
        risk: "低".into(),
        net_edge: "+0.010%".into(),
        gross_one_cycle: "+0.020%".into(),
        round_trip_cost: "0.010%".into(),
        cost_verified: true,
        execution_eligible: true,
        one_cycle_net: "+0.010%".into(),
        one_cycle_net_bps: 1.0,
        funding_stats: FundingCycleStatsView::default(),
        index_composition: IndexCompositionView::default(),
        reason: "test".into(),
        long_venue: "binance".into(),
        short_venue: "okx".into(),
    }
}

pub(super) fn market_health(quality: MarketDataQuality) -> MarketDataHealth {
    MarketDataHealth {
        quality,
        source: MarketDataSourceKind::LocalCache,
        freshness_ms: Some(10),
        retry_after_ms: None,
        last_error: None,
        observed_at_ms: 1,
        coverage: None,
        problem: None,
    }
}
