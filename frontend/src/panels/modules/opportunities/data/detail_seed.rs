use crate::panels::modules::funding_stats::FundingCycleStatsView;
use crate::panels::modules::index_composition::IndexCompositionView;

use super::OpportunityRow;

#[derive(Clone, PartialEq)]
pub(in crate::panels::modules::opportunities) struct OpportunityDetailSeed {
    pub(in crate::panels::modules::opportunities) id: String,
    pub(in crate::panels::modules::opportunities) pair: String,
    pub(in crate::panels::modules::opportunities) domain: String,
    pub(in crate::panels::modules::opportunities) market_scope: String,
    pub(in crate::panels::modules::opportunities) risk: String,
    pub(in crate::panels::modules::opportunities) net_edge: String,
    pub(in crate::panels::modules::opportunities) gross_one_cycle: String,
    pub(in crate::panels::modules::opportunities) round_trip_cost: String,
    pub(in crate::panels::modules::opportunities) cost_verified: bool,
    pub(in crate::panels::modules::opportunities) execution_eligible: bool,
    pub(in crate::panels::modules::opportunities) one_cycle_net: String,
    pub(in crate::panels::modules::opportunities) one_cycle_net_bps: f64,
    pub(in crate::panels::modules::opportunities) funding_stats: FundingCycleStatsView,
    pub(in crate::panels::modules::opportunities) index_composition: IndexCompositionView,
    pub(in crate::panels::modules::opportunities) reason: String,
    pub(in crate::panels::modules::opportunities) long_venue: String,
    pub(in crate::panels::modules::opportunities) short_venue: String,
}

impl OpportunityDetailSeed {
    pub(crate) fn empty() -> Self {
        Self {
            id: String::new(),
            pair: "-".into(),
            domain: "等待机会".into(),
            market_scope: "全部".into(),
            risk: "中".into(),
            net_edge: "0.00%".into(),
            gross_one_cycle: "+0.000%".into(),
            round_trip_cost: "0.000%".into(),
            cost_verified: false,
            execution_eligible: false,
            one_cycle_net: "+0.000%".into(),
            one_cycle_net_bps: 0.0,
            funding_stats: FundingCycleStatsView::default(),
            index_composition: IndexCompositionView::default(),
            reason: "等待扫描结果。".into(),
            long_venue: "-".into(),
            short_venue: "-".into(),
        }
    }
}

pub(crate) fn detail_seed_at(index: usize, rows: &[OpportunityRow]) -> OpportunityDetailSeed {
    rows.get(index)
        .map(detail_seed_from_row)
        .unwrap_or_else(OpportunityDetailSeed::empty)
}

pub(crate) fn detail_seed_from_row(row: &OpportunityRow) -> OpportunityDetailSeed {
    let row = row.as_ref();
    OpportunityDetailSeed {
        id: row.id.clone(),
        pair: row.pair.clone(),
        domain: row.strategy_label.clone(),
        market_scope: row.strategy_label.clone(),
        risk: row.risk.clone(),
        net_edge: row.net_edge.clone(),
        gross_one_cycle: row.gross_one_cycle.clone(),
        round_trip_cost: row.round_trip_cost.clone(),
        cost_verified: row.cost_verified,
        execution_eligible: row.execution_eligible,
        one_cycle_net: row.one_cycle_net.clone(),
        one_cycle_net_bps: row.one_cycle_net_bps,
        funding_stats: FundingCycleStatsView::default(),
        index_composition: IndexCompositionView::default(),
        reason: row.opportunity_reason(),
        long_venue: row.long_venue.clone(),
        short_venue: row.short_venue.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::modules::opportunity_view_model::view_models_from_rows;
    use shared_types::{
        MarketDataHealth, MarketDataQuality, MarketDataSourceKind, OpportunityLegMarketEvidence,
        OpportunityListCost, OpportunityListExecution, OpportunityListLeg, OpportunityListMetrics,
        OpportunityListRow, RiskLevel, StrategyCategory, StrategyKind,
    };

    #[test]
    fn detail_seed_from_row_uses_minimal_backend_list_fields() {
        let row = view_models_from_rows("detail-seed", vec![row()]).remove(0);

        let seed = detail_seed_from_row(&row);

        assert_eq!(seed.id, "1");
        assert_eq!(seed.pair, "MU");
        assert_eq!(seed.long_venue, "hyperliquid:xyz");
        assert_eq!(seed.short_venue, "kucoin");
        assert_eq!(seed.gross_one_cycle, "+0.020%");
        assert_eq!(seed.round_trip_cost, "0.010%");
        assert!(seed.cost_verified);
        assert!(seed.execution_eligible);
        assert_eq!(seed.one_cycle_net_bps, 1.0);
        assert!(seed.reason.contains("hyperliquid:xyz"));
    }

    fn row() -> OpportunityListRow {
        OpportunityListRow {
            id: "1".into(),
            symbol: "MU".into(),
            strategy_kind: Some(StrategyKind::PerpCross),
            strategy_category: Some(StrategyCategory::Futures),
            type_label: "永续跨所".into(),
            spot_leg_mode: None,
            long_leg: leg("hyperliquid:xyz", "做多永续"),
            short_leg: leg("kucoin", "做空永续"),
            metrics: OpportunityListMetrics {
                score: 80.0,
                risk_level: RiskLevel::Low,
                net_single_yield: 0.001,
                annualized_funding_bps: Some(1095.0),
                one_cycle_net_bps: Some(1.0),
                time_to_settlement_ms: 60_000,
                settlement_countdown_seconds: Some(60),
                liquidity_score: 80.0,
            },
            cost: OpportunityListCost {
                verified: true,
                gross_edge_bps: 2.0,
                total_cost_bps: 1.0,
                wear_bps: 0.2,
                one_cycle_net_bps: Some(1.0),
                one_cycle_covers_cost: true,
                breakeven_periods: 1,
                breakeven_hours: 8.0,
                recommended_hold_hours: 8.0,
                net_bps_at_recommended_hold: 1.0,
                fee_evidence_count: 2,
                fee_evidence_complete: true,
                fee_evidence_ids: vec!["fee:binance:perp:vip0".into(), "fee:okx:perp:vip0".into()],
                one_cycle_penalty: 0.0,
            },
            execution: OpportunityListExecution {
                eligible: true,
                blockers: Vec::new(),
                optimal_position: 10_000.0,
                max_position: 20_000.0,
            },
            data_source: "test".into(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn leg(venue: &str, action: &str) -> OpportunityListLeg {
        OpportunityListLeg {
            venue: venue.into(),
            action: format!("{venue} {action}"),
            price: Some(100.0),
            market_evidence: Some(OpportunityLegMarketEvidence {
                venue: venue.into(),
                symbol: "MU".into(),
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
}
