use super::RealizedPnlRow;
use shared_types::{
    CloseRun, CloseRunCostComponent, CloseRunCostLedgerEvent, CloseRunCostReconciliation,
    CloseRunStatus, ExecutionLedgerQuality, PositionPairEvidence, ReviewCloseRunEvidence,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn apply_close_run_costs(
    rows: &mut BTreeMap<String, RealizedPnlRow>,
    close_runs: &[CloseRun],
) {
    if close_runs.is_empty() {
        return;
    }
    for row in rows.values_mut() {
        apply_row_close_run_costs(row, close_runs);
    }
}

fn apply_row_close_run_costs(row: &mut RealizedPnlRow, close_runs: &[CloseRun]) {
    let keys = row_close_run_link_keys(row);
    if keys.is_empty() {
        return;
    }
    let mut applied_runs = BTreeSet::<String>::new();
    let mut seen_events = row_cost_event_ids(row);
    for run in close_runs {
        if applied_runs.contains(&run.id) {
            continue;
        }
        let Some(pair) = matching_close_run_pair(run, &keys) else {
            continue;
        };
        row.evidence
            .record_close_run_evidence(close_run_evidence(run, pair));
        let delta = close_run_cost_delta(run, &mut seen_events);
        row.fee_usd += delta.fee_usd;
        row.slippage_usd += delta.slippage_usd;
        row.funding_usd += delta.funding_usd;
        // Gross PnL uses actual fills, so slippage is attribution, not another cash debit.
        row.net_pnl_usd += delta.funding_usd - delta.fee_usd - delta.manual_handling_usd;
        applied_runs.insert(run.id.clone());
    }
}

fn row_close_run_link_keys(row: &RealizedPnlRow) -> BTreeSet<(String, String)> {
    row.evidence
        .ledger_events
        .iter()
        .filter_map(|event| Some((event.order.run_id.clone()?, event.order.ticket_id.clone()?)))
        .collect()
}

fn row_cost_event_ids(row: &RealizedPnlRow) -> BTreeSet<String> {
    let mut ids = row
        .evidence
        .ledger_events
        .iter()
        .map(|event| event.event_id.clone())
        .collect::<BTreeSet<_>>();
    ids.extend(row.evidence.fill_event_ids.iter().cloned());
    ids.extend(row.evidence.fee_event_ids.iter().cloned());
    ids.extend(row.evidence.funding_event_ids.iter().cloned());
    ids.extend(row.evidence.slippage_event_ids.iter().cloned());
    ids
}

fn matching_close_run_pair<'a>(
    run: &'a CloseRun,
    keys: &BTreeSet<(String, String)>,
) -> Option<&'a PositionPairEvidence> {
    run.legs
        .iter()
        .filter_map(|leg| leg.pair_evidence.as_ref())
        .find(|pair| keys.contains(&(pair.run_id.clone(), pair.ticket_id.clone())))
}

#[derive(Debug, Clone, Copy, Default)]
struct CloseRunCostDelta {
    fee_usd: f64,
    slippage_usd: f64,
    funding_usd: f64,
    manual_handling_usd: f64,
}

fn close_run_cost_delta(run: &CloseRun, seen_events: &mut BTreeSet<String>) -> CloseRunCostDelta {
    let mut delta = run
        .cost_reconciliation
        .as_ref()
        .map_or_else(CloseRunCostDelta::default, |cost| {
            close_run_reconciled_cost_delta(cost, seen_events)
        });
    for event in close_run_actual_cost_events(run) {
        if !seen_events.insert(event.event_id.clone()) {
            continue;
        }
        delta.observe(event.component, event.amount_usd);
    }
    delta
}

impl CloseRunCostDelta {
    fn observe(&mut self, component: CloseRunCostComponent, amount_usd: f64) {
        match component {
            CloseRunCostComponent::Fee => self.fee_usd += amount_usd,
            CloseRunCostComponent::Slippage => self.slippage_usd += amount_usd,
            CloseRunCostComponent::Funding => self.funding_usd += amount_usd,
            CloseRunCostComponent::ManualHandling => self.manual_handling_usd += amount_usd,
        }
    }
}

fn close_run_actual_cost_events(run: &CloseRun) -> Vec<&CloseRunCostLedgerEvent> {
    let mut events = Vec::new();
    events.extend(
        run.cost_events
            .iter()
            .filter(|event| actual_cost_event(event)),
    );
    for leg in &run.legs {
        events.extend(
            leg.cost_events
                .iter()
                .filter(|event| actual_cost_event(event)),
        );
    }
    if let Some(plan) = run.unwind_plan.as_ref() {
        for attempt in &plan.compensation_attempts {
            events.extend(
                attempt
                    .cost_events
                    .iter()
                    .filter(|event| actual_cost_event(event)),
            );
        }
    }
    events
}

fn actual_cost_event(event: &CloseRunCostLedgerEvent) -> bool {
    event.quality == ExecutionLedgerQuality::Actual
        && event.amount_usd.is_finite()
        && !event.event_id.trim().is_empty()
}

fn close_run_reconciled_cost_delta(
    cost: &CloseRunCostReconciliation,
    seen_events: &mut BTreeSet<String>,
) -> CloseRunCostDelta {
    // An opaque total can include slippage and signed funding; never book it as a fee.
    let mut delta = CloseRunCostDelta::default();
    for (component, amount, event_ids) in [
        (
            CloseRunCostComponent::Fee,
            cost.close_fee_usd,
            cost.close_fee_event_ids.as_slice(),
        ),
        (
            CloseRunCostComponent::Slippage,
            cost.close_slippage_usd,
            cost.close_slippage_event_ids.as_slice(),
        ),
        (
            CloseRunCostComponent::Fee,
            cost.compensation_fee_usd,
            cost.compensation_fee_event_ids.as_slice(),
        ),
        (
            CloseRunCostComponent::Slippage,
            cost.compensation_slippage_usd,
            cost.compensation_slippage_event_ids.as_slice(),
        ),
        (
            CloseRunCostComponent::Funding,
            cost.funding_usd,
            cost.funding_event_ids.as_slice(),
        ),
        (
            CloseRunCostComponent::ManualHandling,
            cost.manual_handling_usd,
            cost.manual_handling_event_ids.as_slice(),
        ),
    ] {
        if let Some(amount) = component_cost_amount(amount, event_ids, seen_events) {
            delta.observe(component, amount);
        }
    }
    delta
}

fn component_cost_amount(
    amount: Option<f64>,
    event_ids: &[String],
    seen_events: &mut BTreeSet<String>,
) -> Option<f64> {
    let amount = amount.filter(|amount| amount.is_finite())?;
    if event_ids.is_empty()
        || event_ids.iter().any(|id| id.trim().is_empty())
        || event_ids
            .iter()
            .any(|event_id| seen_events.contains(event_id))
    {
        return None;
    }
    seen_events.extend(event_ids.iter().cloned());
    Some(amount)
}

fn close_run_evidence(run: &CloseRun, pair: &PositionPairEvidence) -> ReviewCloseRunEvidence {
    ReviewCloseRunEvidence {
        close_run_id: run.id.clone(),
        status: run.status,
        run_id: pair.run_id.clone(),
        ticket_id: pair.ticket_id.clone(),
        opportunity_id: pair.opportunity_id.clone(),
        matched_notional_usd: pair.matched_notional_usd,
        unwind_status: run.unwind_plan.as_ref().map(|plan| plan.status),
        compensation_attempt_count: run
            .unwind_plan
            .as_ref()
            .map_or(0, |plan| plan.compensation_attempts.len()),
        cost_reconciliation: run.cost_reconciliation.clone(),
    }
}

pub(super) fn close_run_cost_missing(evidence: &ReviewCloseRunEvidence) -> bool {
    match evidence.cost_reconciliation.as_ref() {
        Some(cost) => {
            close_run_fee_missing(evidence)
                || cost.missing_fields.iter().any(|field| {
                    !matches!(field.as_str(), "close_slippage" | "compensation_slippage")
                })
                || optional_component_missing(cost.funding_usd, &cost.funding_event_ids)
                || optional_component_missing(
                    cost.manual_handling_usd,
                    &cost.manual_handling_event_ids,
                )
        }
        None => close_run_status_requires_cost(evidence.status),
    }
}

pub(super) fn close_run_fee_missing(evidence: &ReviewCloseRunEvidence) -> bool {
    evidence.cost_reconciliation.as_ref().map_or_else(
        || close_run_status_requires_cost(evidence.status),
        |cost| {
            !component_proven(cost.close_fee_usd, &cost.close_fee_event_ids)
                || optional_component_missing(
                    cost.compensation_fee_usd,
                    &cost.compensation_fee_event_ids,
                )
                || cost
                    .missing_fields
                    .iter()
                    .any(|field| matches!(field.as_str(), "close_fee" | "compensation_fee"))
        },
    )
}

pub(super) fn close_run_slippage_missing(evidence: &ReviewCloseRunEvidence) -> bool {
    evidence.cost_reconciliation.as_ref().map_or_else(
        || close_run_status_requires_cost(evidence.status),
        |cost| {
            !component_proven(cost.close_slippage_usd, &cost.close_slippage_event_ids)
                || optional_component_missing(
                    cost.compensation_slippage_usd,
                    &cost.compensation_slippage_event_ids,
                )
                || cost.missing_fields.iter().any(|field| {
                    matches!(field.as_str(), "close_slippage" | "compensation_slippage")
                })
        },
    )
}

fn component_proven(amount: Option<f64>, event_ids: &[String]) -> bool {
    amount.is_some_and(f64::is_finite)
        && !event_ids.is_empty()
        && event_ids.iter().all(|id| !id.trim().is_empty())
}

fn optional_component_missing(amount: Option<f64>, event_ids: &[String]) -> bool {
    (amount.is_some() || !event_ids.is_empty()) && !component_proven(amount, event_ids)
}

fn close_run_status_requires_cost(status: CloseRunStatus) -> bool {
    matches!(
        status,
        CloseRunStatus::Succeeded
            | CloseRunStatus::Compensated
            | CloseRunStatus::ManuallyResolved
            | CloseRunStatus::UnwindRequired
            | CloseRunStatus::CompensationFailed
            | CloseRunStatus::Failed
    )
}
