use super::format::{execution_mode_label, fee_evidence_lines};
use super::model::*;

pub(super) fn preview_request(
    seed: &PreviewSeed,
    input: &PreviewInput,
) -> shared_types::HedgePreviewRequest {
    shared_types::HedgePreviewRequest {
        opportunity_id: seed.opportunity_id.clone(),
        opportunity_snapshot_id: (!seed.opportunity_snapshot_id.trim().is_empty())
            .then(|| seed.opportunity_snapshot_id.clone()),
        capital_usd: input.capital_usd,
        leverage: input.leverage,
        long_price: input.long_price,
        short_price: input.short_price,
        long_notional_usd: Some(input.long_notional_usd),
        short_notional_usd: Some(input.short_notional_usd),
        execution_params: Some(input.execution_params.clone()),
    }
}

pub(super) fn from_api_preview(
    resp: shared_types::HedgePreviewResponse,
    seed: &PreviewSeed,
    _input: &PreviewInput,
) -> ExecutionPreview {
    let long_reference_price = resp.ticket.long_leg.reference_price;
    let short_reference_price = resp.ticket.short_leg.reference_price;
    let long_market_evidence = resp
        .ticket
        .long_leg
        .market_evidence
        .clone()
        .or_else(|| seed.long_market_evidence.clone());
    let short_market_evidence = resp
        .ticket
        .short_leg
        .market_evidence
        .clone()
        .or_else(|| seed.short_market_evidence.clone());
    let long = resp.long_risk.computed_notional.max(0.0);
    let short = resp.short_risk.computed_notional.max(0.0);
    let modes_match = resp.long_leg.mode == resp.short_leg.mode;
    let guards = resp.ticket.guards.clone();
    let TicketPlanEvidence {
        order_plans,
        identity_evidence_required,
        blockers: ticket_plan_blockers,
    } = ticket_plan_evidence(&resp);
    let mut blockers = resp.ticket.blockers.clone();
    blockers.extend(ticket_plan_blockers);
    let fee_evidence = fee_evidence_lines(&resp.ticket.fee_snapshots);
    let one_cycle_cost = resp
        .ticket
        .cost
        .as_ref()
        .map(PreviewOneCycleCost::from_cost);
    // Ticket-bound gross edge is signed. Missing, zero, or negative evidence must never
    // fall back to a stale positive list estimate on an execution surface.
    let estimated_gross_edge_usd = if resp.estimated_gross_edge_usd.is_finite() {
        resp.estimated_gross_edge_usd
    } else {
        0.0
    };
    let executable_depth = resp.ticket.sizing.max_executable_notional.clone();
    let risk_note = if !modes_match {
        "后端返回双腿执行模式不一致，提交保持禁用。".into()
    } else if !blockers.is_empty() {
        format!("HedgeTicket 阻断 {} 条", blockers.len())
    } else if resp.long_risk.allowed && resp.short_risk.allowed {
        "后端 RiskDecision 通过".into()
    } else if let Some(note) = risk_evidence_note(&resp.long_risk, &resp.short_risk) {
        note
    } else {
        format!(
            "后端阻断：多腿 {} 条，空腿 {} 条",
            resp.long_risk.reasons.len(),
            resp.short_risk.reasons.len()
        )
    };
    ExecutionPreview {
        opportunity_id: resp.opportunity_id,
        opportunity_snapshot_id: resp.opportunity_snapshot_id,
        ticket_id: Some(resp.ticket.ticket_id),
        expires_at_ms: Some(resp.ticket.expires_at_ms),
        idempotency_key: Some(resp.idempotency_key),
        readiness: PreviewReadiness::Ready,
        source: "后端预检",
        estimated_funding_usd: estimated_gross_edge_usd,
        open_cost_usd: resp.estimated_open_cost_usd,
        close_cost_usd: resp.estimated_close_cost_usd,
        slippage_cost_usd: resp.estimated_slippage_usd,
        one_cycle_cost,
        max_loss_usd: resp.max_loss_usd,
        used_capital_usd: resp.used_capital_usd,
        liquidation: PreviewLiquidation {
            current_account_pct: resp.current_account_liq_distance_pct,
            after_hedge_pct: resp.after_hedge_liq_distance_pct,
            positions_evidence: resp.positions_evidence,
        },
        execution_mode_label: execution_mode_label(resp.long_leg.mode),
        long_allowed: modes_match && resp.long_risk.allowed,
        short_allowed: modes_match && resp.short_risk.allowed,
        long_notional_usd: long,
        short_notional_usd: short,
        long_reference_price,
        short_reference_price,
        long_market_evidence,
        short_market_evidence,
        depth: PreviewDepth {
            long_5bps: resp.ticket.long_leg.depth_usd_5bps,
            long_10bps: resp.ticket.long_leg.depth_usd_10bps,
            long_20bps: resp.ticket.long_leg.depth_usd_20bps,
            short_5bps: resp.ticket.short_leg.depth_usd_5bps,
            short_10bps: resp.ticket.short_leg.depth_usd_10bps,
            short_20bps: resp.ticket.short_leg.depth_usd_20bps,
            executable_status: executable_depth.status,
            executable_amount_usd: executable_depth.amount_usd,
            executable_reason: executable_depth.reason,
            long_reason: resp.ticket.long_leg.depth_reason,
            short_reason: resp.ticket.short_leg.depth_reason,
            long_depth_health: resp.ticket.long_leg.depth_health,
            short_depth_health: resp.ticket.short_leg.depth_health,
        },
        fee_evidence,
        profit_evidence: seed.profit_evidence.clone(),
        order_plans,
        identity_evidence_required,
        risk: PreviewRisk {
            note: risk_note,
            guards,
            blockers,
        },
    }
}

struct TicketPlanEvidence {
    order_plans: Vec<shared_types::OrderCompilePlan>,
    identity_evidence_required: bool,
    blockers: Vec<String>,
}

fn ticket_plan_evidence(resp: &shared_types::HedgePreviewResponse) -> TicketPlanEvidence {
    let Some(ticket_order_plans) = resp.ticket_order_plans.as_ref() else {
        return TicketPlanEvidence {
            order_plans: Vec::new(),
            identity_evidence_required: false,
            blockers: vec!["HEDGE_TICKET_ORDER_PLAN_EVIDENCE_MISSING".to_owned()],
        };
    };
    let plans = match ticket_order_plans.plans_for_ticket(&resp.ticket.ticket_id) {
        Ok(plans) => plans,
        Err(error) => {
            return TicketPlanEvidence {
                order_plans: Vec::new(),
                identity_evidence_required: false,
                blockers: vec![format!("HEDGE_TICKET_ORDER_PLAN_EVIDENCE_INVALID: {error}")],
            };
        }
    };
    let identity_evidence_required = plans
        .iter()
        .any(|plan| plan.identity_plan.evidence_required);
    let mut blockers = order_identity_blockers(&plans, identity_evidence_required);
    if resp.long_leg.mode == shared_types::ExecutionMode::Live
        || resp.short_leg.mode == shared_types::ExecutionMode::Live
    {
        blockers.extend(order_sizing_blockers(&plans));
    }
    let order_plans = plans
        .into_iter()
        .map(|plan| plan.compile_plan.clone())
        .collect();
    TicketPlanEvidence {
        order_plans,
        identity_evidence_required,
        blockers,
    }
}

fn order_sizing_blockers(
    plans: &[&shared_types::hedge::HedgeTicketOrderPlanEvidence],
) -> Vec<String> {
    plans
        .iter()
        .filter_map(|evidence| {
            let plan = &evidence.compile_plan;
            plan.validate_sizing_contract()
                .err()
                .map(|error| format!("{} {} {}", plan.exchange, plan.symbol, error.code()))
        })
        .collect()
}

fn order_identity_blockers(
    plans: &[&shared_types::hedge::HedgeTicketOrderPlanEvidence],
    identity_evidence_required: bool,
) -> Vec<String> {
    if identity_evidence_required && plans.len() != 2 {
        return vec!["ORDER_IDENTITY_PLAN_MISSING".to_owned()];
    }
    plans
        .iter()
        .flat_map(|plan| {
            plan.identity_plan.blockers.iter().map(move |blocker| {
                format!(
                    "{} {} {blocker}",
                    plan.compile_plan.exchange, plan.compile_plan.symbol
                )
            })
        })
        .collect()
}

fn risk_evidence_note(
    long: &shared_types::RiskDecision,
    short: &shared_types::RiskDecision,
) -> Option<String> {
    let long_note = decision_evidence_note("多腿", long);
    let short_note = decision_evidence_note("空腿", short);
    match (long_note, short_note) {
        (None, None) => None,
        (long_note, short_note) => Some(
            [long_note, short_note]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("; "),
        ),
    }
}

fn decision_evidence_note(label: &str, decision: &shared_types::RiskDecision) -> Option<String> {
    (!decision.evidence.is_empty()).then(|| {
        format!(
            "{label}: {}",
            decision
                .evidence
                .iter()
                .map(shared_types::RiskBlockEvidence::compact_summary)
                .collect::<Vec<_>>()
                .join(", ")
        )
    })
}
