use crate::state::action_state::ActionState;
use crate::state::load_state::LoadState;
use shared_types::{ApiProblem, CloseRun, CloseRunStatus, PortfolioSnapshot};

use super::super::components::SectionData;
use super::super::data::{
    is_current_partial_snapshot_problem, portfolio_account_access, PortfolioAccountAccess,
    PREVIOUS_CLOSE_RECORD_LABEL,
};

pub(super) fn loaded_snapshot(state: &LoadState<PortfolioSnapshot>) -> Option<&PortfolioSnapshot> {
    match state {
        LoadState::Ready(snapshot)
        | LoadState::Stale {
            value: snapshot, ..
        } => Some(snapshot),
        LoadState::Loading | LoadState::Error(_) => None,
    }
}

pub(super) fn snapshot_account_access(
    state: &LoadState<PortfolioSnapshot>,
) -> PortfolioAccountAccess {
    loaded_snapshot(state)
        .map(portfolio_account_access)
        .unwrap_or_default()
}

pub(super) fn actionable_snapshot_problem(
    state: &LoadState<PortfolioSnapshot>,
) -> Option<ApiProblem> {
    state
        .problem()
        .filter(|problem| !is_current_partial_snapshot_problem(problem))
        .cloned()
}

pub(super) fn should_render_close_runs_surface<T>(section: &SectionData<Vec<T>>) -> bool {
    !section.value.is_empty()
}

pub(super) fn snapshot_section<T: Default>(
    state: &LoadState<PortfolioSnapshot>,
    map: impl FnOnce(&PortfolioSnapshot) -> T,
) -> SectionData<T> {
    match state {
        LoadState::Loading => SectionData::loading(),
        LoadState::Ready(snapshot) => SectionData::ready(map(snapshot)),
        LoadState::Stale { value, problem } if is_current_partial_snapshot_problem(problem) => {
            SectionData::ready(map(value))
        }
        LoadState::Stale { value, problem } => SectionData::stale(map(value), problem),
        LoadState::Error(problem) => SectionData::error(problem),
    }
}

pub(super) fn actionable_close_runs(snapshot: &PortfolioSnapshot) -> Vec<CloseRun> {
    snapshot
        .recent_close_runs
        .iter()
        .filter(|run| {
            run.finality_problem.is_some()
                || matches!(
                    run.status,
                    CloseRunStatus::UnwindRequired
                        | CloseRunStatus::CompensationSubmitted
                        | CloseRunStatus::CompensationFailed
                )
        })
        .cloned()
        .collect()
}

pub(super) fn position_action_message(state: &ActionState) -> String {
    let Some(label) = state.label() else {
        return String::new();
    };
    let Some(problem) = state.problem() else {
        return label.to_owned();
    };

    format!("{label}：{}", position_problem_message(problem))
}

fn position_problem_message(problem: &ApiProblem) -> String {
    if problem.code == shared_types::problem::codes::RISK_BLOCKED {
        let protected = problem.message.contains("ProtectedPosition");
        let venue_blocked = problem.message.contains("ExchangeNotAllowed");
        return match (protected, venue_blocked) {
            (true, true) => {
                "该仓位受保护，且当前交易所不在允许平仓范围；请在“控制”中核对保护名单与交易所范围"
                    .to_owned()
            }
            (true, false) => "该仓位受保护；请在“控制”中核对保护名单".to_owned(),
            (false, true) => "当前交易所不在允许平仓范围；请在“控制”中核对交易所范围".to_owned(),
            (false, false) => problem.message.clone(),
        };
    }
    problem.message.clone()
}

fn position_problem_evidence(problem: &ApiProblem) -> String {
    let mut parts = Vec::new();
    if !problem.code.trim().is_empty() {
        parts.push(format!("code {}", problem.code));
    }
    if let Some(source) = problem.source.as_deref() {
        parts.push(format!("source {source}"));
    }
    if let Some(status) = problem.status {
        parts.push(format!("HTTP {status}"));
    }
    if let Some(request_id) = problem.request_id.as_deref() {
        parts.push(format!("request_id {request_id}"));
    }
    if let Some(retry_after_ms) = problem.retry_after_ms {
        parts.push(format!("retry {retry_after_ms}ms"));
    }
    if let Some(details) = problem.details.as_ref() {
        parts.push(format!("details {details}"));
    }
    parts.join(" · ")
}

pub(super) fn position_action_evidence(state: &ActionState) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(summary) = state
        .evidence()
        .map(shared_types::ActionEvidence::summary)
        .filter(|summary| !summary.is_empty())
    {
        parts.push(summary);
    }
    if let Some(problem) = state.problem() {
        let evidence = position_problem_evidence(problem);
        if !evidence.is_empty() {
            parts.push(evidence);
        }
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub(super) fn completed_previous_close_summary(state: &ActionState) -> Option<&str> {
    let ActionState::Succeeded { label, .. } = state else {
        return None;
    };
    label
        .strip_prefix(PREVIOUS_CLOSE_RECORD_LABEL)
        .and_then(|summary| summary.strip_prefix('：'))
}
