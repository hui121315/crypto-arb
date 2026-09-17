use super::*;
use crate::panels::modules::funding_stats::FundingCycleStatsView;
use crate::panels::modules::index_composition::IndexCompositionView;
use crate::panels::modules::opportunity_format::missing_quote_text;
use crate::panels::modules::opportunity_view_model::OpportunityListViewModel;
use shared_types::{SpotLegMode, StrategyCategory, StrategyKind};
use std::sync::Arc;

#[test]
fn leg_price_line_keeps_real_price() {
    assert_eq!(leg_price_line("0.05402000", None), "价格 0.05402000");
}

#[test]
fn leg_price_line_explains_missing_price() {
    assert_eq!(
        leg_price_line("-", Some("证据 缺数据 · 本地缓存")),
        format!(
            "价格 {}",
            missing_quote_text(Some("证据 缺数据 · 本地缓存"))
        )
    );
    assert_eq!(
        leg_price_line("", None),
        format!("价格 {}", missing_quote_text(None))
    );
}

#[test]
fn missing_metrics_render_as_pending_text() {
    assert_eq!(signed_bps_text(None, "未接入"), "未接入");
    assert_eq!(signed_bps_text(None, "未知"), "未知");
    assert_eq!(alignment_text(None), "待窗口");
    assert_eq!(hours_text(None), "未知");
    let mut settlement = row();
    Arc::make_mut(&mut settlement.view).settlement_countdown_seconds = Some(0);
    assert_eq!(settlement_countdown_text(&settlement), "结算同步中");
    Arc::make_mut(&mut settlement.view).settlement_countdown_seconds = Some(30);
    assert_eq!(settlement_countdown_text(&settlement), "<1m");
    Arc::make_mut(&mut settlement.view).settlement_countdown_seconds = None;
    assert_eq!(settlement_countdown_text(&settlement), "结算时间缺证据");
}

#[test]
fn cost_cells_include_fee_evidence_state() {
    let mut verified = row();
    {
        let view = Arc::make_mut(&mut verified.view);
        view.cost_verified = true;
        view.cost_total_bps = 47.0;
        view.cost_wear_bps = 20.0;
        view.fee_evidence_count = 2;
        view.fee_evidence_complete = true;
        view.round_trip_cost = "0.470%".into();
    }

    assert!(cost_text(&verified).contains("费率证据 2/2"));
    assert!(cost_breakeven_detail_text(&verified).contains("费率证据 2/2"));
    assert!(detail_value(&verified, ColumnId::RoundTripCostBps).contains("费率证据 2/2"));

    let mut partial = row();
    {
        let view = Arc::make_mut(&mut partial.view);
        view.cost_verified = false;
        view.fee_evidence_count = 1;
        view.fee_evidence_complete = false;
        view.round_trip_cost = "成本未验证".into();
    }

    assert!(cost_text(&partial).contains("费率证据 1/2 未完整"));
    assert!(cost_breakeven_detail_text(&partial).contains("费率证据 1/2 未完整"));
    assert!(detail_value(&partial, ColumnId::RoundTripCostBps).contains("费率证据 1/2 未完整"));
}

#[test]
fn spot_cross_uses_immediate_finality_without_fake_holding_hours() {
    let mut opportunity = row();
    let view = Arc::make_mut(&mut opportunity.view);
    view.strategy_kind = Some(StrategyKind::SpotCross);
    view.cost_verified = true;
    view.one_cycle_net = "+0.120%".into();
    view.one_cycle_net_bps = 12.0;
    view.breakeven_periods = 1;

    assert_eq!(breakeven_text(&opportunity), "即时费后 +0.120%");
    let detail = cost_breakeven_detail_text(&opportunity);
    assert!(detail.contains("即时费后 +0.120%"));
    assert!(!detail.contains("0.0h"));
}

#[test]
fn projected_strategies_name_their_unlocked_exit() {
    let mut convergence = row();
    {
        let view = Arc::make_mut(&mut convergence.view);
        view.strategy_kind = Some(StrategyKind::PerpPriceSpread);
        view.cost_verified = true;
        view.one_cycle_net_bps = 12.0;
        view.recommended_hold_hours = 0.5;
        view.net_bps_at_recommended_hold = 12.0;
    }
    assert_eq!(breakeven_text(&convergence), "等待价差收敛");
    assert!(one_cycle_detail_text(&convergence).starts_with("预测费后边际"));

    let mut basis = convergence;
    {
        let view = Arc::make_mut(&mut basis.view);
        view.strategy_kind = Some(StrategyKind::SpotPerp);
        view.settlement_countdown_seconds = Some(1_800);
    }
    assert_eq!(breakeven_text(&basis), "退出基差待绑定");
    assert!(breakeven_context_text(&basis).contains("下一 Funding 30m"));
}

#[test]
fn unprofitable_native_event_does_not_render_a_fake_breakeven() {
    let mut opportunity = row();
    let view = Arc::make_mut(&mut opportunity.view);
    view.cost_verified = true;
    view.breakeven_periods = 0;
    view.breakeven_hours = 0.0;

    assert_eq!(breakeven_text(&opportunity), "单次未覆盖成本");
}

fn row() -> FuturesOpportunity {
    let view = Arc::new(OpportunityListViewModel {
        id: "opp-1".into(),
        snapshot_id: "test-snapshot".into(),
        pair: "MU".into(),
        strategy_label: "永续跨所".into(),
        strategy_kind: Some(StrategyKind::PerpCross),
        strategy_category: Some(StrategyCategory::Futures),
        spot_leg_mode: None::<SpotLegMode>,
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
        net_edge: "+0.000%".into(),
        net_basis_bps: 0.0,
        predicted_funding_bps: Some(0.0),
        est_apr_pct: Some(0.0),
        gross_one_cycle_bps: 0.0,
        gross_one_cycle: "成本未验证".into(),
        round_trip_cost: "成本未验证".into(),
        cost_verified: false,
        cost_total_bps: 0.0,
        cost_wear_bps: 0.0,
        fee_evidence_count: 0,
        fee_evidence_complete: false,
        fee_evidence_ids: Vec::new(),
        one_cycle_penalty: 0.0,
        one_cycle_net: "单次未验证".into(),
        one_cycle_net_bps: 0.0,
        one_cycle_covers_cost: false,
        breakeven_periods: 0,
        breakeven_hours: 0.0,
        recommended_hold_hours: 0.0,
        net_bps_at_recommended_hold: 0.0,
        risk: "低".into(),
        risk_level: shared_types::RiskLevel::Low,
        settlement_countdown_seconds: None,
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
        borrow_cost_bps_per_day: None,
        funding_alignment_minutes: None,
        funding_cap_distance_bps: None,
        min_hold_hours: None,
    }
}
