use super::*;

pub(in crate::services::close_runs) fn apply_order_update(
    run: &mut CloseRun,
    record: &OrderRecord,
) -> bool {
    let mut changed = false;
    for leg in &mut run.legs {
        changed |= apply_leg_update(leg, record);
    }
    changed |= apply_compensation_order_update(run, record);
    changed |= apply_finality_success(run, record);
    if changed {
        refresh_run_summary(run);
    }
    changed
}

pub(in crate::services::close_runs) fn apply_finality_problem(
    run: &mut CloseRun,
    order_id: &str,
    problem: &ApiProblem,
    checked_at_ms: i64,
) -> bool {
    if !run_matches_order_id(run, order_id) {
        return false;
    }
    let finality_checked_at_ms = latest_positive_time(run.finality_checked_at_ms, checked_at_ms);
    if run.finality_problem.as_ref() == Some(problem)
        && run.finality_checked_at_ms == finality_checked_at_ms
    {
        return false;
    }
    run.finality_problem = Some(problem.clone());
    run.finality_checked_at_ms = finality_checked_at_ms;
    run.updated_at_ms = run.updated_at_ms.max(checked_at_ms);
    true
}

pub(in crate::services::close_runs) fn apply_finality_success(
    run: &mut CloseRun,
    record: &OrderRecord,
) -> bool {
    if record.last_update_source != OrderUpdateSource::OrderQuery {
        return false;
    }
    if !run_matches_record(run, record) {
        return false;
    }
    let finality_checked_at_ms =
        latest_positive_time(run.finality_checked_at_ms, record.updated_at_ms);
    if run.finality_problem.is_none() && run.finality_checked_at_ms == finality_checked_at_ms {
        return false;
    }
    run.finality_problem = None;
    run.finality_checked_at_ms = finality_checked_at_ms;
    true
}

pub(in crate::services::close_runs) fn latest_positive_time(
    current: Option<i64>,
    candidate: i64,
) -> Option<i64> {
    if candidate <= 0 {
        return current;
    }
    Some(current.map_or(candidate, |value| value.max(candidate)))
}
