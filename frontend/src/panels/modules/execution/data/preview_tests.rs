use super::*;
use crate::panels::modules::opportunity_format::missing_quote_label;
use shared_types::{MarketDataSourceKind, RiskBlockEvidence, RiskBlockReason, TradeFeeEvidence};

#[path = "preview_tests/backoff.rs"]
mod backoff;
#[path = "preview_tests/fee.rs"]
mod fee;
#[path = "preview_tests/fixtures.rs"]
mod fixtures;
#[path = "preview_tests/parse.rs"]
mod parse;
#[path = "preview_tests/problem.rs"]
mod problem;
#[path = "preview_tests/state.rs"]
mod state;
#[path = "preview_tests/ticket.rs"]
mod ticket;
use fixtures::{
    execution_cost, hyperliquid_ticket_order_plans, leg_evidence, preview_input,
    preview_query_fixture, preview_response, preview_seed_fixture,
};

#[test]
fn preview_request_requires_the_current_non_empty_selection() {
    let query = preview_query_fixture();

    assert!(active_preview_query(Some(query.clone()), "").is_none());
    assert!(active_preview_query(Some(query.clone()), "opp-2").is_none());
    assert_eq!(
        active_preview_query(Some(query), "opp-1").map(|query| query.seed.opportunity_id),
        Some("opp-1".to_owned())
    );
}

#[test]
fn pending_preview_is_not_ready_and_keeps_draft_notional() {
    let preview = pending_preview(&preview_seed_fixture(), &preview_input());

    assert!(!preview.can_submit());
    assert_eq!(preview.readiness, PreviewReadiness::Pending);
    assert_eq!(preview.execution_mode_label, "等待");
    assert_eq!(preview.long_notional_usd, 200.0);
    assert_eq!(preview.short_notional_usd, 200.0);
}

#[test]
fn preview_request_keeps_the_selected_opportunity_snapshot() {
    let seed = preview_seed_fixture();

    let request = preview_request(&seed, &preview_input());

    assert_eq!(request.opportunity_id, "opp-1");
    assert_eq!(
        request.opportunity_snapshot_id.as_deref(),
        Some(seed.opportunity_snapshot_id.as_str())
    );
}

#[test]
fn default_capital_text_rounds_down_inside_verified_depth() {
    let mut selection = ExecutionSelection::empty();
    selection.default_capital_usd = 331.5;
    selection.default_leverage = 2.0;

    assert_eq!(default_capital_text(&selection), "331");
}

#[test]
fn from_api_preview_ignores_legacy_order_plans_without_ticket_evidence() {
    let mut response = preview_response();
    let ticket_order_plans = hyperliquid_ticket_order_plans();
    response.ticket_order_plans = None;
    response.long_order_plan = Some(ticket_order_plans.long.compile_plan);
    response.short_order_plan = Some(ticket_order_plans.short.compile_plan);

    let preview = from_api_preview(
        response,
        &PreviewSeed::from_selection(&ExecutionSelection::empty()),
        &preview_input(),
    );

    assert!(preview.order_plans.is_empty());
    assert!(preview
        .risk
        .blockers
        .contains(&"HEDGE_TICKET_ORDER_PLAN_EVIDENCE_MISSING".to_owned()));
    assert!(!preview.can_submit());
}

#[test]
fn from_api_preview_uses_ticket_hyperliquid_builder_order_plan_evidence() {
    let mut response = preview_response();
    response.ticket_order_plans = Some(hyperliquid_ticket_order_plans());

    let preview = from_api_preview(
        response,
        &PreviewSeed::from_selection(&ExecutionSelection::empty()),
        &preview_input(),
    );

    assert_eq!(preview.order_plans.len(), 2);
    assert_eq!(
        preview.order_plans[0].role,
        shared_types::HedgeLegRole::Long
    );
    assert_eq!(
        preview.order_plans[0].venue_order_kind,
        shared_types::VenueOrderKind::ProtectedIoc
    );
    assert_eq!(
        preview.order_plans[0]
            .client_order_id_policy
            .venue_client_order_id
            .as_deref(),
        Some("0x00000000000000000000000000000001")
    );
    assert!(preview.can_submit());
}

#[test]
fn live_preview_fails_closed_without_ticket_instrument_sizing_contracts() {
    let mut response = preview_response();
    response.long_leg.mode = shared_types::ExecutionMode::Live;
    response.short_leg.mode = shared_types::ExecutionMode::Live;
    if let Some(plans) = response.ticket_order_plans.as_mut() {
        plans.long.compile_plan.instrument_spec = None;
        plans.long.compile_plan.sizing_plan = None;
        plans.short.compile_plan.instrument_spec = None;
        plans.short.compile_plan.sizing_plan = None;
    }

    let preview = from_api_preview(
        response,
        &PreviewSeed::from_selection(&ExecutionSelection::empty()),
        &preview_input(),
    );

    assert!(!preview.can_submit());
    assert_eq!(
        preview
            .risk
            .blockers
            .iter()
            .filter(|blocker| blocker.contains("INSTRUMENT_SPEC_MISSING"))
            .count(),
        2
    );
}

#[test]
fn from_api_preview_keeps_ticket_leg_price_provenance() {
    let mut selection = ExecutionSelection::empty();
    selection.long_market_evidence =
        Some(leg_evidence("seed", "MU", MarketDataSourceKind::LocalCache));
    let mut response = preview_response();
    response.ticket.long_leg.market_evidence = Some(leg_evidence(
        "hyperliquid:xyz",
        "MU",
        MarketDataSourceKind::WsPush,
    ));
    response.ticket.short_leg.market_evidence = Some(leg_evidence(
        "gate",
        "MU",
        MarketDataSourceKind::RestBaseline,
    ));

    let seed = PreviewSeed::from_selection(&selection);
    let preview = from_api_preview(response, &seed, &preview_input());

    assert!(preview
        .long_market_evidence
        .as_ref()
        .is_some_and(|evidence| evidence.venue == "hyperliquid:xyz"
            && evidence.health.source == MarketDataSourceKind::WsPush));
    assert!(preview
        .short_market_evidence
        .as_ref()
        .is_some_and(|evidence| evidence.venue == "gate"));
}

#[test]
fn from_api_preview_derives_net_edge_from_gross_without_double_charging_costs() {
    let response = preview_response();
    let seed = PreviewSeed::from_selection(&ExecutionSelection::empty());
    let preview = from_api_preview(response, &seed, &preview_input());

    assert_eq!(preview.estimated_funding_usd, 1.3);
    assert!((preview.net_edge_usd() - 1.0).abs() < f64::EPSILON);
}

#[test]
fn missing_gross_edge_never_falls_back_to_funding_projection() {
    let mut response = preview_response();
    response.estimated_gross_edge_usd = 0.0;
    response.estimated_funding_next_settlement_usd = Some(2.0);
    let seed = PreviewSeed::from_selection(&ExecutionSelection::empty());
    let preview = from_api_preview(response, &seed, &preview_input());

    assert!((preview.net_edge_usd() + 0.3).abs() < 1e-9);
}

#[test]
fn missing_legacy_gross_edge_fails_closed() {
    let mut response = preview_response();
    response.estimated_gross_edge_usd = 0.0;
    response.estimated_funding_next_settlement_usd = None;
    let seed = PreviewSeed::from_selection(&ExecutionSelection::empty());
    let preview = from_api_preview(response, &seed, &preview_input());

    assert!((preview.net_edge_usd() + 0.3).abs() < 1e-9);
}

#[test]
fn from_api_preview_keeps_one_cycle_cost_evidence() {
    let mut response = preview_response();
    response.ticket.cost = Some(execution_cost());

    let seed = PreviewSeed::from_selection(&ExecutionSelection::empty());
    let preview = from_api_preview(response, &seed, &preview_input());
    assert!(preview.one_cycle_cost.is_some());
    let Some(cost) = preview.one_cycle_cost else {
        return;
    };
    assert_eq!(cost.gross_edge_bps, 4.0);
    assert_eq!(cost.total_cost_bps, 15.0);
    assert_eq!(cost.net_bps, -11.0);
    assert!(!cost.covers_round_trip_cost);
    assert!(cost.funding_window_mismatch_evidence.is_some());
    let Some(evidence) = cost.funding_window_mismatch_evidence else {
        return;
    };
    assert_eq!(
        evidence.yield_basis,
        shared_types::YieldBasis::NativeSettlement
    );
    assert_eq!(evidence.buffer_bps, 0.0);
    assert_eq!(evidence.long_next_settlement_ms, 1_000);
    assert_eq!(evidence.short_next_settlement_ms, 2_000);
}

#[test]
fn from_api_preview_keeps_seed_profit_evidence() {
    let mut selection = ExecutionSelection::empty();
    selection.one_cycle_net_bps = -11.0;
    selection.fee_evidence_ids = vec![
        "fee:hyperliquid:perp:vip0".into(),
        "fee:gate:perp:vip0".into(),
    ];
    selection.fee_evidence_complete = true;

    let seed = PreviewSeed::from_selection(&selection);
    let preview = from_api_preview(preview_response(), &seed, &preview_input());

    assert_eq!(preview.profit_evidence.one_cycle_net_bps, -11.0);
    assert_eq!(
        preview.profit_evidence.fee_evidence_ids,
        ["fee:hyperliquid:perp:vip0", "fee:gate:perp:vip0"]
    );
    assert!(preview.profit_evidence.fee_evidence_complete);
}

#[test]
fn from_api_preview_prefers_risk_block_evidence_note() {
    let mut response = preview_response();
    response.long_risk = shared_types::RiskDecision::block_with_evidence(
        vec![RiskBlockReason::MaxOrderNotionalExceeded],
        250.0,
        vec![RiskBlockEvidence {
            code: RiskBlockReason::MaxOrderNotionalExceeded,
            field: "max_order_notional_usd".into(),
            actual: Some(serde_json::json!(250.0)),
            limit: Some(serde_json::json!(200.0)),
            venue: Some("venue".into()),
            symbol: Some("MU".into()),
            source: "risk_config.max_order_notional".into(),
            checked_at_ms: 42,
        }],
    );

    let seed = PreviewSeed::from_selection(&ExecutionSelection::empty());
    let preview = from_api_preview(response, &seed, &preview_input());

    assert!(preview.risk.note.contains("max_order_notional_usd"));
    assert!(preview.risk.note.contains("250"));
    assert!(preview.risk.note.contains("200"));
    assert!(preview.risk.note.contains("risk_config.max_order_notional"));
    assert!(!preview.risk.note.contains("后端阻断：多腿 1 条，空腿 0 条"));
}
