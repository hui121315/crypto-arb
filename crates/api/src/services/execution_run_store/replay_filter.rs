use shared_types::ExecutionRun;
use std::path::Path;

use super::MIN_PERSISTED_RUN_TIMESTAMP_MS;

pub(super) fn filter_invalid_replay_runs(
    runs: &mut Vec<ExecutionRun>,
    allow_synthetic: bool,
) -> usize {
    if allow_synthetic {
        return 0;
    }
    let before = runs.len();
    runs.retain(valid_persisted_run);
    before.saturating_sub(runs.len())
}

fn valid_persisted_run(run: &ExecutionRun) -> bool {
    run.created_at_ms >= MIN_PERSISTED_RUN_TIMESTAMP_MS
        && run.updated_at_ms >= MIN_PERSISTED_RUN_TIMESTAMP_MS
        && !run.run_id.trim().is_empty()
        && !run.ticket_id.trim().is_empty()
        && !run.opportunity_id.trim().is_empty()
}

pub(super) fn report_replay_health(
    path: Option<&Path>,
    replay_failures: usize,
    filtered_runs: usize,
) {
    report_replay_failure(replay_failures);
    report_filtered_runs(path, filtered_runs);
}

fn report_replay_failure(replay_failures: usize) {
    if replay_failures > 0 {
        tracing::warn!(replay_failures, "execution run ledger replay degraded");
    }
}

fn report_filtered_runs(path: Option<&Path>, filtered_runs: usize) {
    if filtered_runs > 0 {
        tracing::warn!(
            path = path.map_or("disabled", |value| value.to_str().unwrap_or("non-utf8")),
            filtered_runs,
            "ignored impossible execution run replay rows"
        );
    }
}
