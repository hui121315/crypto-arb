//! 共享机会列表投影的缓存、证据与按需构建规则单测。

#[cfg(test)]
mod tests {
    use super::super::{rejected_main_p0_ids, view_models_from_rows, OpportunityListViewModel};
    use shared_types::{
        MarketDataHealth, MarketDataQuality, MarketDataSourceKind, OpportunityLegMarketEvidence,
        OpportunityListCost, OpportunityListExecution, OpportunityListLeg,
        OpportunityListLegFunding, OpportunityListMetrics, OpportunityListRow, RiskLevel,
        StrategyCategory, StrategyKind,
    };
    use std::sync::Arc;

    #[test]
    fn view_models_reuse_unchanged_rows_inside_same_snapshot() {
        let first = view_models_from_rows("snap-a", vec![row("opp-1", 10.0)]);
        let second = view_models_from_rows("snap-a", vec![row("opp-1", 10.0)]);

        assert!(Arc::ptr_eq(&first[0], &second[0]));
        assert_eq!(second[0].long_price, "10.0000");
    }

    #[test]
    fn view_models_replace_changed_rows_inside_same_snapshot() {
        let first = view_models_from_rows("snap-a2", vec![row("opp-1", 10.0)]);
        let second = view_models_from_rows("snap-a2", vec![row("opp-1", 20.0)]);

        assert!(!Arc::ptr_eq(&first[0], &second[0]));
        assert_eq!(second[0].long_price, "20.0000");
    }

    #[test]
    fn view_models_reset_when_snapshot_changes() {
        let first = view_models_from_rows("snap-b", vec![row("opp-2", 10.0)]);
        let second = view_models_from_rows("snap-c", vec![row("opp-2", 20.0)]);

        assert!(!Arc::ptr_eq(&first[0], &second[0]));
        assert_eq!(second[0].long_price, "20.0000");
    }

    #[test]
    fn main_ui_rejects_non_p0_rows_and_reports_patch_removals() {
        let mut diagnostic = row("diagnostic-1", 10.0);
        diagnostic.strategy_kind = Some(StrategyKind::Triangular);
        diagnostic.strategy_category = Some(StrategyCategory::Futures);

        let rejected = rejected_main_p0_ids(std::slice::from_ref(&diagnostic));
        let projected = view_models_from_rows("snap-main-p0", vec![diagnostic]);

        assert_eq!(rejected, ["diagnostic-1"]);
        assert!(projected.is_empty());
    }

    #[test]
    fn view_model_surfaces_cost_and_deferred_depth_evidence() {
        let view = OpportunityListViewModel::from_row(row("opp-evidence", 10.0), "snap-test");

        assert_eq!(view.cost_evidence_label(), "费率数据依据 2/2");
        assert_eq!(
            view.fee_evidence_ids,
            ["fee:binance:perp:vip0", "fee:okx:perp:vip0"]
        );
        assert!(view.cost_detail().contains("回合成本 0.050%"));
        assert!(view.cost_detail().contains("费率数据依据 2/2"));
        assert_eq!(view.depth_evidence_label(), "点击构建后核对");
        assert!(view.depth_detail().contains("读取双腿实时 0.05% 盘口"));
    }

    #[test]
    fn default_size_uses_strategy_recommendation_before_depth_check() {
        let mut row = row("opp-depth-cap", 10.0);
        row.execution.optimal_position = 10_000.0;
        row.execution.max_position = 20_000.0;

        let view = OpportunityListViewModel::from_row(row, "snap-test");

        assert_eq!(view.default_size_usd(), 10_000.0);
        assert_eq!(view.default_capital_usd(), 5_000.0);
    }

    #[test]
    fn list_without_depth_remains_buildable_and_explains_deferred_validation() {
        let view = OpportunityListViewModel::from_row(row("opp-depth-deferred", 10.0), "snap-test");

        assert!(view.execution_eligible);
        assert!(view.execution_blockers.is_empty());
        assert_eq!(view.opportunity_size_label(), "构建时核对");
        assert_eq!(view.depth_evidence_label(), "点击构建后核对");
        assert!(view.depth_detail().contains("点击构建对冲后读取"));
    }

    #[test]
    fn ticket_bound_spot_perp_proof_does_not_disable_build_preflight() {
        let mut deferred = row("spot-perp-deferred", 10.0);
        deferred.strategy_kind = Some(StrategyKind::SpotPerp);
        deferred.spot_leg_mode = Some(shared_types::SpotLegMode::BuySpot);
        deferred.execution.blockers =
            vec![shared_types::DEFERRED_SPOT_PERP_TICKET_BLOCKER.to_owned()];

        let view = OpportunityListViewModel::from_row(deferred.clone(), "snap-test");

        assert!(view.execution_eligible);
        assert_eq!(
            view.execution_blockers,
            [shared_types::DEFERRED_SPOT_PERP_TICKET_BLOCKER]
        );

        deferred
            .execution
            .blockers
            .push("交易所标的身份未通过".to_owned());
        let blocked = OpportunityListViewModel::from_row(deferred, "snap-test");
        assert!(!blocked.execution_eligible);
    }

    #[test]
    fn fresh_rest_price_evidence_remains_observation_only() {
        let mut row = row("opp-rest-baseline", 10.0);
        if let Some(evidence) = row.long_leg.market_evidence.as_mut() {
            evidence.health.source = MarketDataSourceKind::RestBaseline;
        }

        let view = OpportunityListViewModel::from_row(row, "snap-test");

        assert!(!view.execution_eligible);
        assert!(view
            .execution_blockers
            .iter()
            .any(|blocker| blocker.contains("实时行情")));
    }

    #[test]
    fn view_model_marks_partial_cost_evidence() {
        let mut row = row("opp-partial-cost", 10.0);
        row.cost.verified = false;
        row.cost.fee_evidence_count = 1;
        row.cost.fee_evidence_complete = false;

        let view = OpportunityListViewModel::from_row(row, "snap-test");

        assert_eq!(view.round_trip_cost, "成本未验证");
        assert_eq!(view.cost_evidence_label(), "费率数据依据 1/2 未完整");
        assert!(view.cost_detail().contains("成本未验证"));
    }

    #[test]
    fn missing_annualized_funding_stays_missing_in_view_model() {
        let mut row = row("opp-missing-apr", 10.0);
        row.metrics.annualized_funding_bps = None;

        let view = OpportunityListViewModel::from_row(row, "snap-test");

        assert_eq!(view.est_apr_pct, None);
    }

    #[test]
    fn native_funding_column_never_reuses_total_strategy_profit() {
        let mut row = row("opp-native-funding", 10.0);
        row.metrics.net_single_yield = 0.50;
        row.long_leg.funding = Some(funding(0.000_1));
        row.short_leg.funding = Some(funding(0.000_3));

        let view = OpportunityListViewModel::from_row(row, "snap-test");

        assert!(view
            .predicted_funding_bps
            .is_some_and(|bps| (bps - 2.0).abs() < 1e-9));
    }

    #[test]
    fn missing_settlement_countdown_stays_missing_in_view_model() {
        let mut row = row("opp-missing-settlement", 10.0);
        row.metrics.settlement_countdown_seconds = None;
        row.metrics.time_to_settlement_ms = 0;

        let view = OpportunityListViewModel::from_row(row, "snap-test");

        assert_eq!(view.settlement_countdown_seconds, None);
        assert_eq!(view.cycle_label(), "结算时间数据待确认");
        assert!(view.tte().contains("结算时间数据待确认"));
    }

    fn row(id: &str, price: f64) -> OpportunityListRow {
        OpportunityListRow {
            id: id.into(),
            symbol: "MU".into(),
            strategy_kind: Some(StrategyKind::PerpCross),
            strategy_category: Some(StrategyCategory::Futures),
            type_label: "永续跨所".into(),
            spot_leg_mode: None,
            long_leg: leg("binance", price),
            short_leg: leg("okx", price),
            metrics: OpportunityListMetrics {
                score: 80.0,
                risk_level: RiskLevel::Low,
                net_single_yield: 0.001,
                annualized_funding_bps: Some(1095.0),
                one_cycle_net_bps: Some(5.0),
                time_to_settlement_ms: 0,
                settlement_countdown_seconds: Some(0),
                liquidity_score: 80.0,
            },
            cost: OpportunityListCost {
                verified: true,
                gross_edge_bps: 10.0,
                total_cost_bps: 5.0,
                wear_bps: 1.0,
                one_cycle_net_bps: Some(5.0),
                one_cycle_covers_cost: true,
                breakeven_periods: 1,
                breakeven_hours: 8.0,
                recommended_hold_hours: 8.0,
                net_bps_at_recommended_hold: 5.0,
                fee_evidence_count: 2,
                fee_evidence_complete: true,
                fee_evidence_ids: vec!["fee:binance:perp:vip0".into(), "fee:okx:perp:vip0".into()],
                one_cycle_penalty: 0.0,
            },
            execution: OpportunityListExecution {
                eligible: true,
                blockers: Vec::new(),
                optimal_position: 0.0,
                max_position: 0.0,
            },
            data_source: "test".into(),
            updated_at: chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
        }
    }

    fn leg(venue: &str, price: f64) -> OpportunityListLeg {
        OpportunityListLeg {
            venue: venue.into(),
            action: format!("{venue} 做多"),
            price: Some(price),
            market_evidence: Some(OpportunityLegMarketEvidence {
                venue: venue.into(),
                symbol: "MUUSDT".into(),
                price: Some(price),
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

    fn funding(rate: f64) -> OpportunityListLegFunding {
        OpportunityListLegFunding {
            rate,
            interval_hours: Some(8),
            next_funding_time_ms: Some(1),
        }
    }
}
