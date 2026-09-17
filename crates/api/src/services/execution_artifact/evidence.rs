mod rows;

use rows::{
    cost_evidence, depth_observed_at, evidence_row, market_evidence_row, money_or_unknown,
    order_plan_evidence, profit_lock_evidence, transfer_route_evidence,
};
use shared_types::{
    ExecutionArtifactBuildRequest, ExecutionArtifactEvidence, ExecutionArtifactLeg, HedgeLegQuote,
    HedgeLegRole, HedgePreviewResponse, HEDGE_PREVIEW_MARKET_MAX_AGE_MS,
};

pub(super) fn evidence(
    preview: &HedgePreviewResponse,
    request: &ExecutionArtifactBuildRequest,
    now_ms: i64,
) -> Vec<ExecutionArtifactEvidence> {
    let checked_at_ms = crate::services::hedge_ticket::ticket_market_checked_at_ms(&preview.ticket);
    let ticket_ready = crate::services::hedge_ticket::ticket_ready(&preview.ticket);
    let plans_detail = order_plan_evidence(preview);
    let long_target = leg_required_notional(preview, HedgeLegRole::Long);
    let short_target = leg_required_notional(preview, HedgeLegRole::Short);
    let long_depth = preview.ticket.long_leg.depth_usd_5bps;
    let short_depth = preview.ticket.short_leg.depth_usd_5bps;
    let depth_passed = long_target
        .is_some_and(|target| long_depth.is_some_and(|value| value.is_finite() && value >= target))
        && short_target.is_some_and(|target| {
            short_depth.is_some_and(|value| value.is_finite() && value >= target)
        });
    let environment_passed =
        preview.long_leg.mode.environment() == preview.short_leg.mode.environment();
    vec![
        evidence_row(
            "snapshot_binding",
            "机会快照",
            !request.opportunity_snapshot_id.trim().is_empty(),
            format!("snapshot={}", request.opportunity_snapshot_id),
            Some(checked_at_ms),
        ),
        evidence_row(
            "ticket_guards",
            "票据守卫",
            ticket_ready,
            if ticket_ready {
                format!("{} guards passed", preview.ticket.guards.len())
            } else {
                "ticket blockers or guards remain".to_owned()
            },
            Some(checked_at_ms),
        ),
        transfer_route_evidence(preview),
        profit_lock_evidence(
            &crate::services::hedge_ticket::ticket_profit_proof(&preview.ticket),
            checked_at_ms,
        ),
        evidence_row(
            "risk",
            "双腿风险",
            preview.long_risk.allowed && preview.short_risk.allowed,
            format!(
                "long={}; short={}",
                preview.long_risk.allowed, preview.short_risk.allowed
            ),
            Some(checked_at_ms),
        ),
        evidence_row(
            "order_plans",
            "执行规格",
            plans_detail.0,
            plans_detail.1,
            Some(checked_at_ms),
        ),
        market_evidence_row("long_market", "多腿行情", &preview.ticket.long_leg, now_ms),
        market_evidence_row(
            "short_market",
            "空腿行情",
            &preview.ticket.short_leg,
            now_ms,
        ),
        evidence_row(
            "depth",
            "双腿深度",
            depth_passed,
            format!(
                "required long={}; short={}; available long={}; short={}",
                money_or_unknown(long_target),
                money_or_unknown(short_target),
                money_or_unknown(long_depth),
                money_or_unknown(short_depth)
            ),
            depth_observed_at(preview),
        ),
        cost_evidence(preview),
        evidence_row(
            "environment",
            "执行环境",
            environment_passed,
            format!(
                "long={:?}; short={:?}",
                preview.long_leg.mode.environment(),
                preview.short_leg.mode.environment()
            ),
            Some(checked_at_ms),
        ),
    ]
}

fn leg_required_notional(preview: &HedgePreviewResponse, role: HedgeLegRole) -> Option<f64> {
    let (intent, quote) = match role {
        HedgeLegRole::Long => (&preview.long_leg, &preview.ticket.long_leg),
        HedgeLegRole::Short => (&preview.short_leg, &preview.ticket.short_leg),
    };
    quote
        .open_vwap_price
        .or(quote.reference_price)
        .map(|price| intent.quantity * price)
        .filter(|notional| notional.is_finite() && *notional > 0.0)
}

pub(super) fn market_evidence_expired(leg: &HedgeLegQuote, now_ms: i64) -> bool {
    leg.market_evidence.as_ref().is_none_or(|item| {
        item.health.observed_at_ms <= 0
            || item.health.observed_at_ms > now_ms
            || now_ms.saturating_sub(item.health.observed_at_ms) > HEDGE_PREVIEW_MARKET_MAX_AGE_MS
    })
}

pub(super) fn artifact_leg(
    preview: &HedgePreviewResponse,
    role: HedgeLegRole,
) -> ExecutionArtifactLeg {
    let (quote, intent, risk) = match role {
        HedgeLegRole::Long => (
            &preview.ticket.long_leg,
            &preview.long_leg,
            &preview.long_risk,
        ),
        HedgeLegRole::Short => (
            &preview.ticket.short_leg,
            &preview.short_leg,
            &preview.short_risk,
        ),
    };
    ExecutionArtifactLeg {
        role,
        venue: quote.exchange.clone(),
        symbol: quote.symbol.clone(),
        side: intent.side,
        reference_price: quote.reference_price,
        target_notional_usd: risk.computed_notional,
        depth_usd_5bps: quote.depth_usd_5bps,
        market_quality: quote
            .market_evidence
            .as_ref()
            .map(|item| item.health.quality),
        market_source: quote
            .market_evidence
            .as_ref()
            .map(|item| item.health.source),
        market_observed_at_ms: quote
            .market_evidence
            .as_ref()
            .map(|item| item.health.observed_at_ms),
    }
}

#[cfg(test)]
mod tests;
