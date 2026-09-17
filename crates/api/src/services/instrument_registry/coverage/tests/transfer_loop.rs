use super::*;

mod support;
use support::*;

#[test]
fn transfer_refresh_is_started_only_on_demand_and_deduplicated() {
    let registry = listing_ready_registry();
    let mut row = spot_cross_opportunity();
    registry.apply_listing_gate(std::slice::from_mut(&mut row), NOW);

    assert!(registry.should_refresh_transfer_for_candidate(&row, NOW));
    assert!(registry.begin_transfer_refresh("binance", NOW));
    assert!(!registry.begin_transfer_refresh("binance", NOW + 1));
    registry.apply_listing_gate(std::slice::from_mut(&mut row), NOW + 1);
    assert!(row
        .execution_blockers
        .iter()
        .any(|blocker| blocker.starts_with("现货跨所充提闭环未通过：")));

    let unlisted_registry = InstrumentRegistry::default();
    let mut unlisted = spot_cross_opportunity();
    unlisted_registry.apply_listing_gate(std::slice::from_mut(&mut unlisted), NOW);
    assert!(!unlisted_registry.should_refresh_transfer_for_candidate(&unlisted, NOW));
}

#[test]
fn transfer_refresh_waits_for_a_positive_spot_cross_opportunity() {
    let registry = listing_ready_registry();

    let mut no_profit = spot_cross_opportunity();
    no_profit.net_single_yield = 0.0;
    registry.apply_listing_gate(std::slice::from_mut(&mut no_profit), NOW);
    assert!(!registry.should_refresh_transfer_for_candidate(&no_profit, NOW));

    let mut wrong_strategy = spot_cross_opportunity();
    wrong_strategy.strategy_kind = Some(StrategyKind::PerpCross);
    registry.apply_listing_gate(std::slice::from_mut(&mut wrong_strategy), NOW);
    assert!(!registry.should_refresh_transfer_for_candidate(&wrong_strategy, NOW));

    let mut separately_blocked = spot_cross_opportunity();
    registry.apply_listing_gate(std::slice::from_mut(&mut separately_blocked), NOW);
    separately_blocked
        .execution_blockers
        .push("盘口证据尚未确认".into());
    assert!(!registry.should_refresh_transfer_for_candidate(&separately_blocked, NOW));
}

#[test]
fn transfer_gate_ignores_non_ws_discovery_rows() {
    let registry = listing_ready_registry();
    let mut row = spot_cross_opportunity();
    let evidence_changed = row
        .long_leg_market_evidence
        .as_mut()
        .is_some_and(|evidence| {
            evidence.health.source = shared_types::MarketDataSourceKind::RestBaseline;
            true
        });
    assert!(evidence_changed, "missing long-leg market evidence");

    registry.apply_listing_gate(std::slice::from_mut(&mut row), NOW);

    assert!(!registry.should_refresh_transfer_for_candidate(&row, NOW));
    assert!(!row
        .execution_blockers
        .iter()
        .any(|blocker| blocker.starts_with("现货跨所充提闭环未通过：")));
    assert!(!row
        .risk_warnings
        .iter()
        .any(|warning| warning.starts_with("现货跨所充提闭环：")));
}

#[test]
fn transfer_refresh_rechecks_market_evidence_at_the_request_boundary() {
    let registry = listing_ready_registry();
    let mut row = spot_cross_opportunity();
    row.execution_eligible = false;
    row.execution_blockers = vec![format!(
        "{}等待候选触发读取",
        shared_types::SPOT_CROSS_TRANSFER_BLOCKER_PREFIX
    )];
    let evidence_changed = row
        .long_leg_market_evidence
        .as_mut()
        .is_some_and(|evidence| {
            evidence.health.source = shared_types::MarketDataSourceKind::RestBaseline;
            true
        });
    assert!(evidence_changed, "missing long-leg market evidence");

    assert!(!registry.should_refresh_transfer_for_candidate(&row, NOW));
}

#[test]
fn spot_cross_requires_and_prices_the_bilateral_transfer_loop() {
    let registry = ready_registry();
    let mut row = spot_cross_opportunity();

    registry.apply_listing_gate(std::slice::from_mut(&mut row), NOW);

    assert!(row.execution_eligible, "{:?}", row.execution_blockers);
    assert!(row
        .risk_warnings
        .iter()
        .any(|warning| warning.contains("Base 经 bitcoin，Quote 经 tron")));
    assert!((row.net_single_yield - 0.0139).abs() < 1e-9);
    assert!(row.execution_cost.is_some(), "missing cost profile");
    let Some(cost) = row.execution_cost else {
        return;
    };
    assert!((cost.one_cycle.net_bps - 139.0).abs() < 1e-9);
    assert!(cost.one_cycle.covers_round_trip_cost);
}

#[test]
fn spot_cross_blocks_when_a_common_network_is_not_open_both_ways() {
    let registry = ready_registry();
    let mut disabled = transfer("binance", "BTC", "BTC", false, true, (1, 3));
    disabled.withdraw_enabled = false;
    assert_eq!(
        registry.replace_transfer_venue(
            "binance",
            vec![
                disabled,
                transfer("binance", "USDT", "TRC20", true, false, (0, 0))
            ],
        ),
        2
    );
    let mut row = spot_cross_opportunity();

    registry.apply_listing_gate(std::slice::from_mut(&mut row), NOW);

    assert!(!row.execution_eligible);
    assert!(
        row.execution_blockers
            .iter()
            .any(|blocker| blocker.contains("共同网络当前未同时开放提币和充值")),
        "{:?}",
        row.execution_blockers
    );
    assert!(
        shared_types::market_monitor_net_bps_at(&row, NOW).is_some(),
        "充提阻断不应抹掉已由 WS 与费用证明的市场价差"
    );
}

#[test]
fn spot_cross_blocks_when_transfer_cost_removes_the_profit() {
    let registry = ready_registry();
    assert_eq!(
        registry.replace_transfer_venue(
            "okx",
            vec![
                transfer("okx", "BTC", "Bitcoin", true, false, (0, 0)),
                transfer("okx", "USDT", "TRON", false, true, (300, 0)),
            ],
        ),
        2
    );
    let mut row = spot_cross_opportunity();

    registry.apply_listing_gate(std::slice::from_mut(&mut row), NOW);

    assert!(!row.execution_eligible);
    assert!(
        row.execution_blockers
            .iter()
            .any(|blocker| blocker.contains("计入充提成本") && blocker.contains("净收益不再为正")),
        "{:?}",
        row.execution_blockers
    );
}
