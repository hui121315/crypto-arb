use super::super::*;
use super::*;

#[test]
fn mock_adapter_hedge_preview_uses_dry_run() {
    assert_eq!(execution_mode_for_adapter("mock"), ExecutionMode::DryRun);
    assert_eq!(
        execution_mode_for_adapter("unknown_adapter"),
        ExecutionMode::DryRun
    );
}

#[test]
fn testnet_adapters_hedge_preview_use_paper() {
    assert_eq!(
        execution_mode_for_adapter("binance_testnet"),
        ExecutionMode::DryRun
    );
    assert_eq!(
        execution_mode_for_adapter("okx_testnet"),
        ExecutionMode::DryRun
    );
}

#[test]
fn live_router_hedge_preview_uses_live() {
    assert_eq!(execution_mode_for_adapter("live"), ExecutionMode::Live);
}

#[test]
fn blocked_detail_reports_current_risk_reason() {
    let long = RiskDecision::allow(100.0);
    let short = RiskDecision::block(vec![shared_types::RiskBlockReason::KillSwitchActive], 100.0);
    let detail = blocked_detail(&long, &short);
    assert!(!detail.is_empty());
    assert!(detail.contains("KillSwitchActive"));
}

#[test]
fn estimated_perp_cross_funding_keeps_the_native_joint_event() {
    let mut ticket = ticket_with(Vec::new(), true);
    ticket.cost = Some(shared_types::ExecutionCostProfile {
        gross_edge_bps: 1.0,
        fee_bps: 0.2,
        wear_bps: 0.2,
        total_cost_bps: 0.4,
        one_cycle: shared_types::OneCycleCostProfile::default(),
        breakeven_periods: 1,
        breakeven_hours: 1.0,
        recommended_hold_periods: 1,
        recommended_hold_hours: 1.0,
        net_bps_at_recommended_hold: 0.6,
        round_trip: None,
    });

    assert_eq!(preview_funding_yield(&ticket), 0.0001);
}

#[test]
fn spot_perp_previews_use_only_the_native_short_funding_event() {
    let mut ticket = ticket_with(Vec::new(), true);
    ticket.strategy = Some(StrategyKind::SpotPerp);
    ticket.spot_leg_mode = Some(shared_types::SpotLegMode::BuySpot);
    ticket.short_leg.funding_bps = Some(2.0);

    assert_eq!(preview_funding_yield(&ticket), 0.000_2);

    ticket.strategy = Some(StrategyKind::CrossSpotPerp);
    assert_eq!(preview_funding_yield(&ticket), 0.000_2);
}

#[test]
fn p0_strategy_falls_back_from_arb_type_without_inventing_kind() {
    assert_eq!(
        p0_strategy_from_arb_type(ArbitrageType::CrossExchange),
        Some(StrategyKind::PerpCross)
    );
    assert_eq!(
        p0_strategy_from_arb_type(ArbitrageType::SpotCross),
        Some(StrategyKind::SpotCross)
    );
}

#[test]
fn ticket_ready_requires_no_blockers_and_passed_guards() {
    assert!(ticket_ready(&ticket_with(Vec::new(), true)));
    assert!(!ticket_ready(&ticket_with(vec!["深度不足".into()], true)));
    assert!(!ticket_ready(&ticket_with(Vec::new(), false)));
}

#[tokio::test]
async fn dry_run_preview_skips_live_adapter_capability_guard() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    let long = order_plan_for_exchange("mock");
    let short = order_plan_for_exchange("mock");
    let mut dry_run_ticket = ticket_with(Vec::new(), true);

    append_order_capability_guard(
        &state,
        &mut dry_run_ticket,
        ExecutionMode::DryRun,
        &long,
        &short,
    );

    assert!(dry_run_ticket
        .guards
        .iter()
        .all(|guard| guard.key != "order_capability"));
    assert!(dry_run_ticket.blockers.is_empty());

    let mut live_ticket = ticket_with(Vec::new(), true);
    append_order_capability_guard(&state, &mut live_ticket, ExecutionMode::Live, &long, &short);

    let capability_guard = live_ticket
        .guards
        .iter()
        .find(|guard| guard.key == "order_capability");
    assert!(capability_guard.is_some(), "live capability guard");
    if let Some(capability_guard) = capability_guard {
        assert!(!capability_guard.passed);
        assert!(capability_guard.detail.contains("不支持"));
    }
    Ok(())
}

#[test]
fn estimated_costs_split_fee_and_vwap_wear() {
    let mut ticket = ticket_with(Vec::new(), true);
    ticket.cost = Some(ExecutionCostProfile {
        gross_edge_bps: 30.0,
        fee_bps: 12.0,
        wear_bps: 10.0,
        total_cost_bps: 22.0,
        one_cycle: OneCycleCostProfile {
            gross_edge_bps: 30.0,
            open_fee_bps: 5.0,
            close_fee_bps: 7.0,
            open_slippage_bps: 3.0,
            close_slippage_bps: 7.0,
            funding_window_mismatch_buffer_bps: 0.0,
            yield_basis: None,
            long_next_settlement_ms: None,
            short_next_settlement_ms: None,
            target_buffer_bps: 0.0,
            net_bps: 8.0,
            covers_round_trip_cost: true,
        },
        breakeven_periods: 1,
        breakeven_hours: 8.0,
        recommended_hold_periods: 2,
        recommended_hold_hours: 16.0,
        net_bps_at_recommended_hold: 38.0,
        round_trip: None,
    });

    let costs = estimated_costs_usd(&ticket, 10_000.0);

    assert_eq!(costs.open_usd, 5.0);
    assert_eq!(costs.close_usd, 7.0);
    assert_eq!(costs.slippage_usd, 10.0);
    assert_eq!(costs.total_usd(), 22.0);
}
