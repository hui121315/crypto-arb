use super::*;
use shared_types::{CloseRun, CloseRunScope, CloseRunStatus, PositionSide};

pub(crate) fn project_close_run_update(
    state: &AppState,
    close_run: &CloseRun,
) -> Vec<ExecutionRun> {
    if close_run.status != CloseRunStatus::Succeeded || close_run.scope == CloseRunScope::Single
        || !close_run.has_complete_fills() || close_run.finality_problem.is_some() || close_run.problem.is_some() {
        return Vec::new();
    }
    let store = state.execution_run_store();
    let _projection_guard = store.lock_projection();
    let mut updated = Vec::new();
    for mut entry in state.execution_runs().iter_mut() {
        let run = entry.value_mut();
        if run.state == ExecutionRunState::Closed
            || !close_run_covers_execution_pair(close_run, run)
        {
            continue;
        }
        close_execution_run(run, close_run);
        refresh_workflow_view(run);
        state.execution_run_store().append(run);
        append_run_finality(
            state,
            run,
            OrderUpdateSource::Internal,
            Some(close_run.id.as_str()),
            None,
            close_run.updated_at_ms,
        );
        updated.push(run.clone());
    }
    updated
}

fn close_run_covers_execution_pair(close_run: &CloseRun, run: &ExecutionRun) -> bool {
    let mut long_closed = false;
    let mut short_closed = false;
    for leg in &close_run.legs {
        let Some(evidence) = leg
            .pair_evidence
            .as_ref()
            .filter(|evidence| shared_types::AutomationExecutionReceipt::matches_pair(run, evidence))
        else {
            continue;
        };
        let (opened, partner, opposite) = match leg.side {
            PositionSide::Long => (&run.long_leg, &run.short_leg, PositionSide::Short),
            PositionSide::Short => (&run.short_leg, &run.long_leg, PositionSide::Long),
        };
        let Some(quantity) = opened.filled_quantity.filter(|quantity| quantity.is_finite() && *quantity > 0.0) else { continue; };
        let tolerance = f64::EPSILON * 32.0 * quantity;
        if evidence.side != leg.side || evidence.partner_side != opposite
            || !shared_types::venue_names_equal(&leg.venue, &opened.exchange)
            || !shared_types::venue_names_equal(&evidence.venue, &leg.venue)
            || !shared_types::venue_names_equal(&evidence.partner_venue, &partner.exchange)
            || !evidence.symbol.eq_ignore_ascii_case(&leg.symbol)
            || !exchange::strip_common_suffixes(&opened.symbol).eq_ignore_ascii_case(&exchange::strip_common_suffixes(&leg.symbol))
            || !exchange::strip_common_suffixes(&partner.symbol).eq_ignore_ascii_case(&exchange::strip_common_suffixes(&evidence.partner_symbol))
            || (leg.quantity - quantity).abs() > tolerance
            || !evidence.leg_filled_quantity.is_finite()
            || (evidence.leg_filled_quantity - quantity).abs() > tolerance {
            continue;
        }
        match evidence.side {
            PositionSide::Long => long_closed = true,
            PositionSide::Short => short_closed = true,
        }
    }
    long_closed && short_closed
}

fn close_execution_run(run: &mut ExecutionRun, close_run: &CloseRun) {
    run.state = ExecutionRunState::Closed;
    run.net_exposure_usd = 0.0;
    run.recovery_action = None;
    run.unwind_problem = None;
    run.finality_problem = None;
    run.finality_checked_at_ms = Some(close_run.updated_at_ms);
    run.status_reason = "配对平仓已完成".to_owned();
    run.updated_at_ms = run.updated_at_ms.max(close_run.updated_at_ms);
    append_close_run_transition(run, close_run);
}
