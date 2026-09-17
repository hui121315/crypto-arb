//! Compact `HedgeTicket` workflow projection for preview, REST replay and WS deltas.

#[path = "workflow_view/health.rs"]
mod health;
#[cfg(test)]
#[path = "workflow_view/tests.rs"]
mod tests;

use health::{capability_health, fee_health, market_health, preflight_health};
use shared_types::hedge::{HedgeTicketOrderPlanEvidence, HedgeTicketOrderPlans};
use shared_types::{
    problem::codes, venue_names_equal, ExecutionGuard, HedgeLegQuote, HedgeTicket,
    HedgeTicketLegView, HedgeTicketView,
};

pub(super) fn project(
    ticket: &HedgeTicket,
    plans: &HedgeTicketOrderPlans,
    now_ms: i64,
) -> HedgeTicketView {
    let balance_guard = guard(ticket, "margin_balance");
    let capability_guard = guard(ticket, "order_capability");
    HedgeTicketView {
        ticket_id: Some(ticket.ticket_id.clone()),
        opportunity_id: Some(ticket.opportunity_id.clone()),
        strategy: ticket.strategy,
        symbol: Some(ticket.symbol.clone()),
        expires_at_ms: Some(ticket.expires_at_ms),
        blockers: ticket.blockers.clone(),
        long_leg: Some(project_leg(&LegProjection {
            ticket,
            quote: &ticket.long_leg,
            plan: &plans.long,
            balance_guard,
            capability_guard,
            now_ms,
        })),
        short_leg: Some(project_leg(&LegProjection {
            ticket,
            quote: &ticket.short_leg,
            plan: &plans.short,
            balance_guard,
            capability_guard,
            now_ms,
        })),
        execution_run: None,
    }
}

pub(super) fn refresh_margin_evidence(
    view: &mut HedgeTicketView,
    guard: &ExecutionGuard,
    long_venue: &str,
    short_venue: &str,
) -> Result<(), &'static str> {
    let (Some(long), Some(short)) = (view.long_leg.as_mut(), view.short_leg.as_mut()) else {
        return Err("workflow view is missing a hedge leg");
    };
    if !venue_names_equal(&long.venue, long_venue) || !venue_names_equal(&short.venue, short_venue)
    {
        return Err("workflow view venue does not match the confirmed ticket");
    }
    long.balance = preflight_health(
        Some(guard),
        long_venue,
        "margin_balance",
        codes::MARGIN_BALANCE_MISSING,
        "票据腿缺少目标 venue 的保证金余额证据",
    );
    short.balance = preflight_health(
        Some(guard),
        short_venue,
        "margin_balance",
        codes::MARGIN_BALANCE_MISSING,
        "票据腿缺少目标 venue 的保证金余额证据",
    );
    Ok(())
}

struct LegProjection<'a> {
    ticket: &'a HedgeTicket,
    quote: &'a HedgeLegQuote,
    plan: &'a HedgeTicketOrderPlanEvidence,
    balance_guard: Option<&'a ExecutionGuard>,
    capability_guard: Option<&'a ExecutionGuard>,
    now_ms: i64,
}

fn project_leg(input: &LegProjection<'_>) -> HedgeTicketLegView {
    let quote = input.quote;
    HedgeTicketLegView {
        role: quote.role,
        venue: quote.exchange.clone(),
        symbol: quote.symbol.clone(),
        market: market_health(quote),
        fee: fee_health(input.ticket, quote, &input.plan.compile_plan, input.now_ms),
        balance: preflight_health(
            input.balance_guard,
            &quote.exchange,
            "margin_balance",
            codes::MARGIN_BALANCE_MISSING,
            "票据腿缺少目标 venue 的保证金余额证据",
        ),
        capability: capability_health(input.capability_guard, input.plan),
    }
}

fn guard<'a>(ticket: &'a HedgeTicket, key: &str) -> Option<&'a ExecutionGuard> {
    ticket.guards.iter().find(|guard| guard.key == key)
}
