use crate::state::action_state::ActionState;

use super::super::super::data::{run_is_released, ExecutionPreview};
use super::super::super::problem::execution_problem_text;
use super::super::execution_status_bar::run_state_label;
use shared_types::{ApiProblem, ExecutionRun};

pub(super) fn run_blocks_new_submission(
    run: Option<&ExecutionRun>,
    preview: &ExecutionPreview,
) -> bool {
    run.is_some_and(|run| run.opportunity_id == preview.opportunity_id && !run_is_released(run))
}

pub(super) fn run_matches_preview(run: &ExecutionRun, preview: &ExecutionPreview) -> bool {
    preview.ticket_id.as_deref() == Some(run.ticket_id.as_str())
}

pub(super) fn action_is_previous(state: &ActionState, preview: &ExecutionPreview) -> bool {
    state
        .evidence()
        .and_then(|evidence| evidence.ticket_id.as_deref())
        .is_some_and(|ticket| preview.ticket_id.as_deref() != Some(ticket))
}

pub(super) fn submit_button_label(
    run: Option<&ExecutionRun>,
    preview: &ExecutionPreview,
) -> String {
    if run_blocks_new_submission(run, preview) {
        if run.is_some_and(|run| run_matches_preview(run, preview)) {
            "已有执行".to_owned()
        } else {
            "原执行未收口".to_owned()
        }
    } else {
        format!("提交 {}", preview.execution_mode_label)
    }
}

pub(super) fn submit_button_title(
    run: Option<&ExecutionRun>,
    preview: &ExecutionPreview,
) -> &'static str {
    if run_blocks_new_submission(run, preview) {
        "当前机会已有未关闭执行，请先完成平仓或补救"
    } else {
        "提交当前已通过交易检查的双腿执行"
    }
}

pub(super) fn run_label(state: &ActionState, run: Option<&ExecutionRun>) -> String {
    match state {
        ActionState::Pending { label, .. } | ActionState::Failed { label, .. } => label.clone(),
        ActionState::Idle | ActionState::Accepted { .. } | ActionState::Succeeded { .. } => run
            .map(run_state_label)
            .unwrap_or_else(|| state.label().unwrap_or("草案待提交").to_owned()),
    }
}

pub(super) fn contextual_run_label(
    state: &ActionState,
    run: Option<&ExecutionRun>,
    has_selection: bool,
) -> String {
    let label = run_label(state, run);
    if !has_selection && (run.is_some() || !matches!(state, ActionState::Idle)) {
        format!("最近执行 · {label}")
    } else {
        label
    }
}

pub(super) fn remedy_detail(state: &ActionState) -> String {
    state.message("")
}

pub(super) fn action_detail(
    state: &ActionState,
    pair: &str,
    preview_problem: Option<&ApiProblem>,
) -> String {
    if missing_pair(pair)
        && matches!(
            state,
            ActionState::Accepted { .. } | ActionState::Succeeded { .. }
        )
    {
        return String::new();
    }
    let detail = match state {
        ActionState::Failed { .. }
            if state
                .problem()
                .is_some_and(|problem| problem.code == "HEDGE_PREVIEW_NOT_READY")
                && preview_problem.is_some() =>
        {
            execution_problem_text("预览阻断", preview_problem.unwrap())
        }
        ActionState::Idle => preview_problem.map_or_else(
            || "草案可编辑".to_owned(),
            |problem| execution_problem_text("预览阻断", problem),
        ),
        ActionState::Pending { .. } | ActionState::Accepted { .. } => state.message("提交中"),
        ActionState::Succeeded { .. } => state.label().unwrap_or("提交中").to_owned(),
        ActionState::Failed { .. } => state.message("提交失败"),
    };
    detail_with_pair(detail, pair)
}

fn detail_with_pair(detail: String, pair: &str) -> String {
    let pair = pair.trim();
    if missing_pair(pair) {
        detail
    } else {
        format!("{detail} · {pair}")
    }
}

fn missing_pair(pair: &str) -> bool {
    let pair = pair.trim();
    pair.is_empty() || pair == "-"
}
