use super::*;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CloseRunLinkKey {
    run_id: String,
    ticket_id: String,
}

pub(super) fn attach_close_run_evidence(rows: &mut [ExecutedTrade], close_runs: &[CloseRun]) {
    if close_runs.is_empty() {
        return;
    }
    for row in rows {
        attach_row_close_run_evidence(row, close_runs);
    }
}

fn attach_row_close_run_evidence(row: &mut ExecutedTrade, close_runs: &[CloseRun]) {
    let keys = row_link_keys(row);
    if keys.is_empty() {
        return;
    }
    let mut attached = BTreeSet::<String>::new();
    for run in close_runs {
        for pair in run.legs.iter().filter_map(|leg| leg.pair_evidence.as_ref()) {
            let key = CloseRunLinkKey {
                run_id: pair.run_id.clone(),
                ticket_id: pair.ticket_id.clone(),
            };
            if !keys.contains(&key) || !attached.insert(run.id.clone()) {
                continue;
            }
            row.evidence
                .record_close_run_evidence(close_run_evidence(run, pair));
        }
    }
}

fn row_link_keys(row: &ExecutedTrade) -> BTreeSet<CloseRunLinkKey> {
    row.evidence
        .ledger_events
        .iter()
        .filter_map(|event| {
            Some(CloseRunLinkKey {
                run_id: event.order.run_id.clone()?,
                ticket_id: event.order.ticket_id.clone()?,
            })
        })
        .collect()
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
