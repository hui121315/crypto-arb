use crate::panels::modules::opportunity_format::missing_quote_label;
use crate::panels::modules::opportunity_view_model::OpportunityListViewModel;
use shared_types::{ExecutionRun, OpportunityLegMarketEvidence};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::panels) struct ExecutionVenuePair {
    pub opportunity_id: String,
    pub pair: String,
    pub long_venue: String,
    pub short_venue: String,
}

#[derive(Clone, PartialEq)]
pub(in crate::panels) struct ExecutionSelection {
    pub source_module: &'static str,
    pub opportunity_id: String,
    pub opportunity_snapshot_id: String,
    pub pair: String,
    pub strategy_label: String,
    pub summary: String,
    pub freshness_label: String,
    pub edge_label: String,
    pub long_leg_label: String,
    pub short_leg_label: String,
    pub long_price_label: String,
    pub short_price_label: String,
    pub long_market_evidence: Option<OpportunityLegMarketEvidence>,
    pub short_market_evidence: Option<OpportunityLegMarketEvidence>,
    pub fee_evidence_ids: Vec<String>,
    pub fee_evidence_complete: bool,
    pub one_cycle_net_bps: f64,
    pub default_capital_usd: f64,
    pub default_leverage: f64,
    pub default_limit_offset_bps: f64,
    pub execution_blockers: Vec<String>,
}

impl ExecutionSelection {
    pub(in crate::panels) fn empty() -> Self {
        Self {
            source_module: "未选择",
            opportunity_id: String::new(),
            opportunity_snapshot_id: String::new(),
            pair: "-".into(),
            strategy_label: "等待机会".into(),
            summary: "请先从机会扫描或期货套利选择一条机会。".into(),
            freshness_label: "-".into(),
            edge_label: "未知".into(),
            long_leg_label: "-".into(),
            short_leg_label: "-".into(),
            long_price_label: missing_quote_label().into(),
            short_price_label: missing_quote_label().into(),
            long_market_evidence: None,
            short_market_evidence: None,
            fee_evidence_ids: Vec::new(),
            fee_evidence_complete: false,
            one_cycle_net_bps: 0.0,
            default_capital_usd: 100_000.0,
            default_leverage: 1.0,
            default_limit_offset_bps: 0.0,
            execution_blockers: vec!["等待可执行机会".into()],
        }
    }

    fn from_list_view(row: &OpportunityListViewModel, source_module: &'static str) -> Self {
        if row.id.trim().is_empty() {
            return Self::empty();
        }
        Self {
            source_module,
            opportunity_id: row.id.clone(),
            opportunity_snapshot_id: row.snapshot_id.clone(),
            pair: row.pair.clone(),
            strategy_label: row.strategy_label.clone(),
            summary: row.opportunity_reason(),
            freshness_label: row.tte(),
            edge_label: row.net_edge.clone(),
            long_leg_label: row.long_leg.clone(),
            short_leg_label: row.short_leg.clone(),
            long_price_label: row.long_price.clone(),
            short_price_label: row.short_price.clone(),
            long_market_evidence: row.long_market_evidence_raw.clone(),
            short_market_evidence: row.short_market_evidence_raw.clone(),
            fee_evidence_ids: row.fee_evidence_ids.clone(),
            fee_evidence_complete: row.fee_evidence_complete,
            one_cycle_net_bps: row.one_cycle_net_bps,
            default_capital_usd: row.default_capital_usd(),
            default_leverage: row.default_leverage(),
            default_limit_offset_bps: 0.0,
            execution_blockers: row.execution_blockers.clone(),
        }
    }

    pub(in crate::panels) fn venue_pair(&self) -> Option<ExecutionVenuePair> {
        if self.opportunity_id.trim().is_empty() {
            return None;
        }
        let long_venue = self.long_market_evidence.as_ref()?.venue.trim();
        let short_venue = self.short_market_evidence.as_ref()?.venue.trim();
        if long_venue.is_empty() || short_venue.is_empty() {
            return None;
        }
        Some(ExecutionVenuePair {
            opportunity_id: self.opportunity_id.clone(),
            pair: self.pair.clone(),
            long_venue: long_venue.to_owned(),
            short_venue: short_venue.to_owned(),
        })
    }

    pub(in crate::panels) fn matches_run(
        &self,
        ticket_id: Option<&str>,
        run: &ExecutionRun,
    ) -> bool {
        if self.opportunity_id.trim().is_empty() {
            return false;
        }
        ticket_id.map_or_else(
            || run.opportunity_id == self.opportunity_id,
            |ticket_id| run.ticket_id == ticket_id,
        )
    }
}

#[derive(Clone, PartialEq)]
pub(in crate::panels) struct ExecutionSelectionSeed(ExecutionSelection);

impl ExecutionSelectionSeed {
    pub(in crate::panels) fn from_opportunities(row: &OpportunityListViewModel) -> Self {
        Self(ExecutionSelection::from_list_view(row, "机会扫描"))
    }

    pub(in crate::panels) fn from_futures(row: &OpportunityListViewModel) -> Self {
        Self(ExecutionSelection::from_list_view(row, "期货套利"))
    }

    pub(in crate::panels) fn into_selection(self) -> ExecutionSelection {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::modules::opportunity_view_model::view_models_from_rows;
    use shared_types::{
        MarketDataHealth, MarketDataQuality, MarketDataSourceKind, OpportunityListCost,
        OpportunityListExecution, OpportunityListLeg, OpportunityListMetrics, OpportunityListRow,
        RiskLevel, StrategyCategory, StrategyKind,
    };

    #[test]
    fn empty_selection_blocks_execution() {
        let selection = ExecutionSelection::empty();

        assert!(selection.opportunity_id.trim().is_empty());
        assert_eq!(selection.long_price_label, missing_quote_label());
        assert_eq!(selection.edge_label, "未知");
        assert_eq!(selection.execution_blockers, ["等待可执行机会"]);
        assert_eq!(selection.venue_pair(), None);
    }

    #[test]
    fn selection_keeps_execution_fields_only() {
        let row = view_models_from_rows("snap-execution-selection", vec![row("opp-1")]).remove(0);

        let selection = ExecutionSelectionSeed::from_opportunities(row.as_ref()).into_selection();

        assert!(!selection.opportunity_id.trim().is_empty());
        assert_eq!(selection.opportunity_id, "opp-1");
        assert_eq!(
            selection.opportunity_snapshot_id,
            "snap-execution-selection"
        );
        assert_eq!(selection.source_module, "机会扫描");
        assert_eq!(selection.strategy_label, "永续跨所");
        assert_eq!(selection.long_leg_label, "hyperliquid 做多永续");
        assert!(selection
            .long_market_evidence
            .as_ref()
            .is_some_and(|evidence| evidence.venue == "hyperliquid"));
        assert_eq!(
            selection.fee_evidence_ids,
            ["fee:hyperliquid:perp:vip0", "fee:gate:perp:vip0"]
        );
        assert_eq!(selection.one_cycle_net_bps, 8.0);
        assert_eq!(selection.default_limit_offset_bps, 0.0);
        assert!(selection.summary.contains("hyperliquid"));
        assert_eq!(
            selection.venue_pair(),
            Some(ExecutionVenuePair {
                opportunity_id: "opp-1".into(),
                pair: "MU".into(),
                long_venue: "hyperliquid".into(),
                short_venue: "gate".into(),
            })
        );
    }

    fn row(id: &str) -> OpportunityListRow {
        OpportunityListRow {
            id: id.into(),
            symbol: "MU".into(),
            strategy_kind: Some(StrategyKind::PerpCross),
            strategy_category: Some(StrategyCategory::Futures),
            type_label: "永续跨所".into(),
            spot_leg_mode: None,
            long_leg: leg("hyperliquid", "做多永续"),
            short_leg: leg("gate", "做空永续"),
            metrics: OpportunityListMetrics {
                score: 88.0,
                risk_level: RiskLevel::Low,
                net_single_yield: 0.001,
                annualized_funding_bps: Some(1095.0),
                one_cycle_net_bps: Some(8.0),
                time_to_settlement_ms: 60_000,
                settlement_countdown_seconds: Some(60),
                liquidity_score: 80.0,
            },
            cost: OpportunityListCost {
                verified: true,
                gross_edge_bps: 10.0,
                total_cost_bps: 2.0,
                wear_bps: 1.0,
                one_cycle_net_bps: Some(8.0),
                one_cycle_covers_cost: true,
                breakeven_periods: 1,
                breakeven_hours: 8.0,
                recommended_hold_hours: 8.0,
                net_bps_at_recommended_hold: 8.0,
                fee_evidence_count: 2,
                fee_evidence_complete: true,
                fee_evidence_ids: vec![
                    "fee:hyperliquid:perp:vip0".into(),
                    "fee:gate:perp:vip0".into(),
                ],
                one_cycle_penalty: 3.5,
            },
            execution: OpportunityListExecution {
                eligible: true,
                blockers: Vec::new(),
                optimal_position: 10_000.0,
                max_position: 20_000.0,
            },
            data_source: "market-data-cache".into(),
            updated_at: chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
        }
    }

    fn leg(venue: &str, action: &str) -> OpportunityListLeg {
        OpportunityListLeg {
            venue: venue.into(),
            action: format!("{venue} {action}"),
            price: Some(100.0),
            market_evidence: Some(OpportunityLegMarketEvidence {
                venue: venue.into(),
                symbol: "MUUSDT".into(),
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
