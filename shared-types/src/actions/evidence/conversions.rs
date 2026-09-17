use super::{extend_unique, push_optional, push_unique, ActionEvidence, ActionEvidenceSource};
use crate::{
    ActionRun, ActionRunKind, CloseRun, CloseRunScope, CloseRunStatus, ExecutionRun, OrderRecord,
};

impl ActionEvidence {
    #[must_use]
    pub fn from_action_run(run: &ActionRun) -> Self {
        let mut evidence = Self::default()
            .with_action_kind(run.kind)
            .with_request_id(run.request_id.clone())
            .with_action_run_id(Some(run.id.clone()))
            .with_idempotency_key(run.idempotency_key.clone());
        evidence.observed_at_ms = Some(run.updated_at_ms);
        evidence.push_source(ActionEvidenceSource::ActionRun);
        evidence
    }

    #[must_use]
    pub fn from_close_run(run: &CloseRun) -> Self {
        let mut evidence = Self::default()
            .with_action_kind(close_run_action_kind(run))
            .with_request_id(run.request_id.clone())
            .with_action_run_id(run.action_run_id.clone())
            .with_idempotency_key(run.idempotency_key.clone())
            .with_run_id(Some(run.id.clone()));
        evidence.observed_at_ms = Some(run.updated_at_ms);
        evidence.push_source(ActionEvidenceSource::CloseRun);
        for leg in &run.legs {
            push_unique(&mut evidence.venues, &leg.venue);
            push_unique(&mut evidence.symbols, &leg.symbol);
            if let Some(order) = &leg.order {
                evidence.merge_order(order);
            }
        }
        if let Some(plan) = &run.unwind_plan {
            merge_unwind_plan(&mut evidence, plan);
        }
        if let Some(cost) = &run.cost_reconciliation {
            extend_unique(
                &mut evidence.order_ids,
                cost.evidence_order_ids.iter().cloned(),
            );
        }
        evidence
    }

    #[must_use]
    pub fn from_execution_run(run: &ExecutionRun) -> Self {
        let mut evidence = Self::default()
            .with_action_kind(ActionRunKind::HedgeConfirm)
            .with_request_id(run.evidence.request_id.clone())
            .with_run_id(Some(run.run_id.clone()))
            .with_ticket_id(Some(run.ticket_id.clone()));
        evidence.observed_at_ms = Some(run.updated_at_ms);
        evidence.push_source(ActionEvidenceSource::ExecutionRun);
        for leg in [&run.long_leg, &run.short_leg] {
            push_unique(&mut evidence.venues, &leg.exchange);
            push_unique(&mut evidence.symbols, &leg.symbol);
            extend_unique(&mut evidence.order_ids, leg.order_ids.iter().cloned());
            if let Some(identity) = &leg.identity {
                evidence.merge_identity(identity);
            }
        }
        for leg in [&run.evidence.long_leg, &run.evidence.short_leg] {
            if let Some(plan) = &leg.compile_plan {
                evidence.merge_client_order_policy(&plan.client_order_id_policy);
            }
        }
        evidence
    }

    #[must_use]
    pub fn from_order_record(order: &OrderRecord) -> Self {
        let mut evidence = Self {
            observed_at_ms: Some(order.updated_at_ms),
            ..Self::default()
        };
        evidence.push_source(ActionEvidenceSource::OrderRecord);
        evidence.merge_order(order);
        evidence
    }
}

fn merge_unwind_plan(evidence: &mut ActionEvidence, plan: &crate::CloseRunUnwindPlan) {
    for leg in plan
        .filled_legs
        .iter()
        .chain(&plan.failed_legs)
        .chain(&plan.compensation_candidates)
        .chain(&plan.remaining_positions)
    {
        push_unique(&mut evidence.venues, &leg.venue);
        push_unique(&mut evidence.symbols, &leg.symbol);
        push_optional(&mut evidence.order_ids, leg.order_id.as_deref());
        push_optional(
            &mut evidence.client_order_ids,
            leg.client_order_id.as_deref(),
        );
        push_optional(
            &mut evidence.exchange_order_ids,
            leg.exchange_order_id.as_deref(),
        );
    }
    for attempt in &plan.compensation_attempts {
        push_unique(&mut evidence.venues, &attempt.venue);
        push_unique(&mut evidence.symbols, &attempt.symbol);
        if let Some(order) = &attempt.order {
            evidence.merge_order(order);
        }
    }
    if let Some(manual) = &plan.manual_terminal_evidence {
        for leg in &manual.remaining_positions {
            push_unique(&mut evidence.venues, &leg.venue);
            push_unique(&mut evidence.symbols, &leg.symbol);
            push_optional(&mut evidence.order_ids, leg.order_id.as_deref());
        }
    }
}

fn close_run_action_kind(run: &CloseRun) -> ActionRunKind {
    match run.status {
        CloseRunStatus::CompensationSubmitted
        | CloseRunStatus::Compensated
        | CloseRunStatus::CompensationFailed => ActionRunKind::PortfolioCloseCompensation,
        CloseRunStatus::ManuallyResolved => ActionRunKind::PortfolioCloseManualTerminal,
        _ => match run.scope {
            CloseRunScope::Single => ActionRunKind::PortfolioClosePosition,
            CloseRunScope::Pair => ActionRunKind::PortfolioClosePair,
            CloseRunScope::All => ActionRunKind::PortfolioCloseAll,
        },
    }
}
