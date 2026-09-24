use super::*;

pub(super) fn preview_response() -> shared_types::HedgePreviewResponse {
    shared_types::HedgePreviewResponse {
        opportunity_id: "opp-1".into(),
        opportunity_snapshot_id: "snapshot-1".into(),
        requested_opportunity_snapshot_id: None,
        ticket: ticket(),
        workflow_view: shared_types::HedgeTicketView {
            ticket_id: Some("ticket-1".into()),
            opportunity_id: Some("opp-1".into()),
            ..shared_types::HedgeTicketView::default()
        },
        ticket_order_plans: Some(hyperliquid_ticket_order_plans()),
        long_leg: order_intent("long", shared_types::OrderSide::Buy),
        long_risk: shared_types::RiskDecision::allow(200.0),
        short_leg: order_intent("short", shared_types::OrderSide::Sell),
        short_risk: shared_types::RiskDecision::allow(200.0),
        long_order_plan: None,
        short_order_plan: None,
        estimated_funding_next_settlement_usd: Some(1.0),
        estimated_funding_per_8h_usd: 1.0,
        estimated_gross_edge_usd: 1.3,
        estimated_open_cost_usd: 0.1,
        estimated_close_cost_usd: 0.1,
        estimated_slippage_usd: 0.1,
        current_account_liq_distance_pct: None,
        after_hedge_liq_distance_pct: None,
        positions_evidence: None,
        used_capital_usd: 100.0,
        max_loss_usd: 5.0,
        idempotency_key: "idem-1".into(),
    }
}
