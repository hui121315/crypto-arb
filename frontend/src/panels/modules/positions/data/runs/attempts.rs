use shared_types::{CloseRun, CloseRunScope, CloseRunStatus, PortfolioSnapshot, PositionRow};

use super::failures::close_run_retry_anchor;

pub(in crate::panels::modules::positions) fn latest_position_close_attempt_anchor(
    snapshot: &PortfolioSnapshot,
    row: &PositionRow,
    expected_leg_count: usize,
) -> Option<(String, i64)> {
    snapshot
        .recent_close_runs
        .iter()
        .filter(|run| run.expected_leg_count == expected_leg_count)
        .filter(|run| close_run_contains_position(run, row))
        .filter_map(close_run_next_attempt_anchor)
        .max_by_key(|(_, updated_at_ms)| *updated_at_ms)
}

pub(in crate::panels::modules::positions) fn latest_close_all_attempt_anchor(
    snapshot: &PortfolioSnapshot,
) -> Option<(String, i64)> {
    snapshot
        .recent_close_runs
        .iter()
        .filter(|run| run.scope == CloseRunScope::All)
        .filter_map(close_run_next_attempt_anchor)
        .max_by_key(|(_, updated_at_ms)| *updated_at_ms)
}

pub(in crate::panels::modules::positions) fn close_run_next_attempt_anchor(
    run: &CloseRun,
) -> Option<(String, i64)> {
    match run.status {
        CloseRunStatus::Succeeded
        | CloseRunStatus::Compensated
        | CloseRunStatus::ManuallyResolved => Some((run.id.clone(), run.updated_at_ms)),
        CloseRunStatus::Failed => close_run_retry_anchor(run),
        CloseRunStatus::Submitted
        | CloseRunStatus::PartiallySubmitted
        | CloseRunStatus::UnwindRequired
        | CloseRunStatus::CompensationSubmitted
        | CloseRunStatus::CompensationFailed => None,
    }
}

fn close_run_contains_position(run: &CloseRun, row: &PositionRow) -> bool {
    run.legs.iter().any(|leg| {
        leg.venue.eq_ignore_ascii_case(&row.venue)
            && leg.symbol.eq_ignore_ascii_case(&row.symbol)
            && leg.side == row.side
    })
}
