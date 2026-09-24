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
        "#execution?symbol=MU&strategy=perp_cross&origin=position&page=2&opp=opp-1&run=run-1&ticket=ticket-1",
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
    assert_eq!(route.ticket_id.as_deref(), Some("ticket-1"));
}

#[test]
fn position_handoff_requires_the_actual_pair_identity_not_same_symbol() {
    let route = parse_workspace_route_parts(
        "#positions?run=run-1&ticket=ticket-1&opp=opp-1",
        "",
        ModuleId::Positions,
    );
    let scope = RunRouteContext::from_route(&route).unwrap();
    let mut row: shared_types::PositionRow = serde_json::from_value(serde_json::json!({
        "venue":"test", "symbol":"BTCUSDT", "side":"long", "quantity":1.0,
        "entryPrice":1.0,"markPrice":1.0,"leverage":1.0,"unrealizedPnlUsd":0.0,"marginUsd":1.0,
        "pairEvidence":{"source":"execution_run","runId":"run-1","ticketId":"ticket-1","opportunityId":"opp-1",
            "venue":"test","symbol":"BTC","side":"long","partnerVenue":"peer","partnerSymbol":"BTC","partnerSide":"short",
            "legFilledQuantity":1.0,"partnerFilledQuantity":1.0,"matchedNotionalUsd":1.0,"updatedAtMs":1}
    })).unwrap();
    assert!(scope.matches_position(&row));
    row.pair_evidence.as_mut().unwrap().ticket_id = "another-ticket".into();
    assert!(!scope.matches_position(&row));
    row.pair_evidence.as_mut().unwrap().ticket_id = "ticket-1".into();
    row.pair_evidence.as_mut().unwrap().run_id = "run-2".into();
    assert!(!scope.matches_position(&row));
    row.pair_evidence = None;
    assert!(!scope.matches_position(&row));
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
