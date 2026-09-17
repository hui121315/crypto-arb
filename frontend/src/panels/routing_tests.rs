use super::*;

#[test]
fn module_slug_round_trips() {
    for module in [
        ModuleId::Positions,
        ModuleId::Futures,
        ModuleId::Opportunities,
        ModuleId::GateCrossEx,
        ModuleId::Onchain,
        ModuleId::Stocks,
        ModuleId::Automation,
        ModuleId::Execution,
        ModuleId::Review,
        ModuleId::Settings,
    ] {
        assert_eq!(ModuleId::from_slug(module.slug()), Some(module));
    }
}

#[test]
fn workspace_route_parses_hash_and_query_context() {
    let route = parse_workspace_route_parts(
        "#execution?symbol=MU&strategy=perp_cross&origin=position&page=2&opp=opp-1&run=run-1",
        "",
        ModuleId::Positions,
    );

    assert_eq!(route.module, ModuleId::Execution);
    assert_eq!(route.symbol.as_deref(), Some("MU"));
    assert_eq!(route.strategy, Some(StrategyKind::PerpCross));
    assert_eq!(route.origin, Some(WorkspaceRouteOrigin::Position));
    assert_eq!(route.page.as_deref(), Some("2"));
    assert_eq!(route.opportunity_id.as_deref(), Some("opp-1"));
    assert_eq!(route.run_id.as_deref(), Some("run-1"));
}

#[test]
fn fragment_context_wins_and_query_can_choose_module() {
    let route = parse_workspace_route_parts(
        "#?symbol=BTC",
        "?module=futures&symbol=ETH&strategy=spot_perp&page=0",
        ModuleId::Positions,
    );

    assert_eq!(route.module, ModuleId::Futures);
    assert_eq!(route.symbol.as_deref(), Some("BTC"));
    assert_eq!(route.strategy, Some(StrategyKind::SpotPerp));
    assert_eq!(route.origin, None);
    assert_eq!(route.page.as_deref(), Some("0"));
}

#[test]
fn route_rejects_unknown_module_strategy_and_oversized_token() {
    let oversized = "x".repeat(MAX_ROUTE_TOKEN_CHARS + 1);
    let route = parse_workspace_route_parts(
        &format!("#unknown?symbol={oversized}&strategy=not_real"),
        "",
        ModuleId::Review,
    );

    assert_eq!(route.module, ModuleId::Review);
    assert_eq!(route.symbol, None);
    assert_eq!(route.strategy, None);
    assert_eq!(route.origin, None);
}
