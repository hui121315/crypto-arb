//! Ordering regression helpers for historical close receipt tests.

use crate::state::action_state::ActionState;
use shared_types::CloseRun;

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
