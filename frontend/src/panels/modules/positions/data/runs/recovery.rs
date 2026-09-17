//! Restore the latest relevant `CloseRun` action state from portfolio snapshots.

use crate::state::action_state::ActionState;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{CloseRun, CloseRunScope, CloseRunStatus, PortfolioSnapshot};

use super::{close_run_action_state, PREVIOUS_CLOSE_RECORD_LABEL};

pub(in crate::panels::modules::positions) fn recover_position_close_state(
    state: RwSignal<ActionState>,
    snapshot_state: RwSignal<LoadState<PortfolioSnapshot>>,
) {
    recover_close_run_state(state, snapshot_state, |run| {
        matches!(run.scope, CloseRunScope::Single | CloseRunScope::Pair)
    });
}

pub(in crate::panels::modules::positions) fn recover_close_all_state(
    state: RwSignal<ActionState>,
    snapshot_state: RwSignal<LoadState<PortfolioSnapshot>>,
) {
    recover_close_run_state(state, snapshot_state, |run| run.scope == CloseRunScope::All);
}

pub(in crate::panels::modules::positions) fn recover_compensation_state(
    state: RwSignal<ActionState>,
    snapshot_state: RwSignal<LoadState<PortfolioSnapshot>>,
) {
    recover_close_run_state(state, snapshot_state, |run| {
        matches!(
            run.status,
            CloseRunStatus::UnwindRequired
                | CloseRunStatus::CompensationSubmitted
                | CloseRunStatus::Compensated
                | CloseRunStatus::CompensationFailed
                | CloseRunStatus::ManuallyResolved
        )
    });
}

fn recover_close_run_state(
    state: RwSignal<ActionState>,
    snapshot_state: RwSignal<LoadState<PortfolioSnapshot>>,
    includes: impl Fn(&CloseRun) -> bool + Copy + 'static,
) {
    Effect::new(move |_| {
        let snapshot = snapshot_state.get();
        let run = match &snapshot {
            LoadState::Ready(snapshot)
            | LoadState::Stale {
                value: snapshot, ..
            } => snapshot
                .recent_close_runs
                .iter()
                .filter(|run| includes(run))
                .max_by_key(|run| run.updated_at_ms),
            LoadState::Loading | LoadState::Error(_) => None,
        };
        if let Some(run) = run.filter(|run| should_recover_close_run(&state.get_untracked(), run)) {
            state.set(close_run_action_state(PREVIOUS_CLOSE_RECORD_LABEL, run));
        }
    });
}

pub(in crate::panels::modules::positions) fn should_recover_close_run(
    current: &ActionState,
    run: &CloseRun,
) -> bool {
    if current.is_pending() {
        return false;
    }
    let Some(evidence) = current.evidence() else {
        return true;
    };
    if evidence
        .idempotency_key
        .as_deref()
        .zip(run.idempotency_key.as_deref())
        .is_some_and(|(current_key, run_key)| current_key == run_key)
    {
        return true;
    }
    evidence
        .observed_at_ms
        .is_some_and(|observed_at_ms| run.updated_at_ms >= observed_at_ms)
}
