use super::*;
use shared_types::{CloseLegStatus, CloseRun, CloseRunScope, CloseRunStatus, PositionSide};

pub(crate) fn project_close_run_update(
    state: &AppState,
    close_run: &CloseRun,
) -> Vec<ExecutionRun> {
    if close_run.status != CloseRunStatus::Succeeded || close_run.scope == CloseRunScope::Single {
        return Vec::new();
    }
    let mut updated = Vec::new();
    for mut entry in state.execution_runs().iter_mut() {
        let run = entry.value_mut();
        if run.state == ExecutionRunState::Closed
            || !close_run_covers_execution_pair(close_run, &run.run_id)
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

fn close_run_covers_execution_pair(close_run: &CloseRun, run_id: &str) -> bool {
    let mut long_closed = false;
    let mut short_closed = false;
    for leg in &close_run.legs {
        if leg.status != CloseLegStatus::Filled {
            continue;
        }
        let Some(evidence) = leg
            .pair_evidence
            .as_ref()
            .filter(|evidence| evidence.run_id == run_id)
        else {
            continue;
        };
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
