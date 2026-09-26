use super::*;
use std::borrow::Cow;

pub(super) fn for_row<'a>(
    run: &'a CloseRun,
    keys: &BTreeSet<(String, String)>,
) -> Cow<'a, CloseRun> {
    let matches = |leg: &&shared_types::CloseLeg| {
        leg.pair_evidence
            .as_ref()
            .is_some_and(|pair| keys.contains(&(pair.run_id.clone(), pair.ticket_id.clone())))
    };
    if run.legs.iter().all(|leg| matches(&leg)) {
        return Cow::Borrowed(run);
    }
    // A close-all receipt is shared by several trades. Only linked orders belong to this row.
    let mut scoped = run.clone();
    scoped.legs = run.legs.iter().filter(matches).cloned().collect();
    scoped.cost_events.clear();
    scoped.unwind_plan = None;
    let mut cost = CloseRunCostReconciliation::default();
    let mut seen_orders = BTreeSet::new();
    let mut seen_events = BTreeSet::new();
    for leg in &scoped.legs {
        let Some(order) = &leg.order else {
            cost.missing_fields.push("close_fee".into());
            continue;
        };
        if order.intent.id.trim().is_empty() || !seen_orders.insert(order.intent.id.clone()) {
            cost.missing_fields.push("close_fee".into());
            continue;
        }
        cost.evidence_order_ids.push(order.intent.id.clone());
        let ids = leg
            .cost_events
            .iter()
            .filter(|event| {
                actual_cost_event(event) && event.component == CloseRunCostComponent::Fee
            })
            .map(|event| event.event_id.clone())
            .collect::<BTreeSet<_>>();
        if let Some(fee) = order
            .filled_fee
            .filter(|fee| fee.is_finite() && !ids.is_empty() && ids.is_disjoint(&seen_events))
        {
            *cost.close_fee_usd.get_or_insert(0.0) += fee;
            seen_events.extend(ids.iter().cloned());
            cost.close_fee_event_ids.extend(ids);
        } else {
            cost.missing_fields.push("close_fee".into());
        }
        let mut has_slippage = false;
        for event in leg.cost_events.iter().filter(|event| {
            actual_cost_event(event) && event.component == CloseRunCostComponent::Slippage
        }) {
            if seen_events.insert(event.event_id.clone()) {
                has_slippage = true;
                *cost.close_slippage_usd.get_or_insert(0.0) += event.amount_usd;
                cost.close_slippage_event_ids.push(event.event_id.clone());
            }
        }
        if !has_slippage {
            cost.missing_fields.push("close_slippage".into());
        }
    }
    // Run-level funding, manual charges and compensation have no pair allocation.
    // Do not guess an equal split or repeat the complete charge on every trade.
    let unallocated = !run.cost_events.is_empty()
        || run
            .unwind_plan
            .as_ref()
            .is_some_and(|plan| !plan.compensation_attempts.is_empty())
        || run.cost_reconciliation.as_ref().is_some_and(|cost| {
            cost.funding_usd.is_some()
                || !cost.funding_event_ids.is_empty()
                || cost.manual_handling_usd.is_some()
                || !cost.manual_handling_event_ids.is_empty()
                || cost.compensation_fee_usd.is_some()
                || !cost.compensation_fee_event_ids.is_empty()
                || cost.compensation_slippage_usd.is_some()
                || !cost.compensation_slippage_event_ids.is_empty()
                || cost
                    .missing_fields
                    .iter()
                    .any(|field| !matches!(field.as_str(), "close_fee" | "close_slippage"))
        });
    if unallocated {
        cost.missing_fields.push("unallocated_close_cost".into());
    }
    cost.missing_fields.sort();
    cost.missing_fields.dedup();
    cost.evidence_event_ids
        .extend(cost.close_fee_event_ids.iter().cloned());
    cost.evidence_event_ids
        .extend(cost.close_slippage_event_ids.iter().cloned());
    if cost.missing_fields.is_empty() {
        cost.total_actual_cost_usd = cost
            .close_fee_usd
            .zip(cost.close_slippage_usd)
            .map(|(fee, slippage)| fee + slippage);
    }
    scoped.cost_reconciliation = Some(cost);
    for leg in &mut scoped.legs {
        // Fee events are cumulative snapshots; the order total above is the cash amount.
        leg.cost_events
            .retain(|event| event.component != CloseRunCostComponent::Fee);
    }
    Cow::Owned(scoped)
}
