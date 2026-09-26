use crate::state::action_state::ActionState;
use crate::state::load_state::LoadState;
use leptos::prelude::*;

use super::super::data::{
    artifact_is_ready, artifact_validation_is_ready, cancelable_order_ids,
    cancelable_order_ids_with_records, run_needs_position_close, run_orders_have_fill,
    CancelRunOrdersAction, ConfirmHedgeAction, ExecutionArtifactRuntime, ExecutionPreview,
};
use super::super::draft::ExecutionDraft;
use super::super::selection::ExecutionSelection;
use shared_types::{
    ApiProblem, DeterministicExecutionArtifact, ExecutionArtifactValidationResponse, ExecutionRun,
};

#[path = "action_bar/confirm.rs"]
mod confirm;
#[path = "action_bar/context_detail.rs"]
mod context_detail;
#[path = "action_bar/labels.rs"]
mod labels;
#[path = "action_bar/outcome_detail.rs"]
mod outcome_detail;
#[path = "action_bar/ticket_refresh.rs"]
mod ticket_refresh;
use confirm::confirm_request;
use labels::*;
use ticket_refresh::use_ticket_refresh;

pub(in crate::panels::modules::execution) fn action_bar(
    selection: Memo<ExecutionSelection>,
    draft: ExecutionDraft,
    artifact: ExecutionArtifactRuntime,
    reviewed: RwSignal<bool>,
    action: ConfirmHedgeAction,
    remedy: CancelRunOrdersAction,
) -> impl IntoView {
    let preview = artifact.preview;
    let artifact_state = artifact.state;
    let artifact_validation = artifact.validation;
    let preview_state = draft.preview_state;
    let execution_run = draft.execution_run;
    let preview_refresh_nonce = draft.preview_nonce;
    let runtime_refresh_nonce = draft.runtime_refresh_nonce;
    let ticket_clock_ms = use_ticket_refresh(
        preview,
        execution_run,
        action.state,
        preview_refresh_nonce,
        artifact.clock,
        action.recovery,
    );
    let can_submit = submit_enabled_memo(SubmitEnabledInputs {
        action_state: action.state,
        preview_state,
        preview,
        execution_run,
        ticket_clock_ms,
        artifact_state,
        artifact_validation,
        reviewed,
        recovery: action.recovery,
        cancel_recovery: remedy.recovery,
    });
    let visible_run_label = visible_run_label_memo(action.state, preview, execution_run, selection, artifact.expired);
    let previous_action = Memo::new(move |_| action_is_previous(&action.state.get(), &preview.get()));
    let preview_problem = Memo::new(move |_| preview_state.with(|state| state.problem().cloned()));
    let remedy_is_current = Memo::new(move |_| {
        let state = remedy.state.get();
        execution_run.get().is_some_and(|run| {
            run_matches_preview(&run, &preview.get())
                && state
                    .evidence()
                    .and_then(|evidence| evidence.run_id.as_deref())
                    == Some(run.run_id.as_str())
        })
    });

    let refresh_preview = move |_| {
        runtime_refresh_nonce.update(|value| *value = value.wrapping_add(1));
        if !action.recovery.blocked() && !remedy.recovery.blocked() {
            reset_preview(action, reviewed, preview_refresh_nonce);
        }
    };
    let reset = move |_| reset_action_bar(action, remedy);
    let can_cancel_orders = can_cancel_orders_memo(remedy.state, execution_run, draft.all_orders);
    let cancel_orders_visible = cancel_orders_visible_memo(execution_run);
    let needs_position_close = needs_position_close_memo(execution_run, draft.all_orders);
    let reset_visible = Memo::new(move |_| {
        !action.recovery.blocked() && !remedy.recovery.blocked()
            && (!matches!(action.state.get(), ActionState::Idle)
                || !matches!(remedy.state.get(), ActionState::Idle))
    });
    let cancel_orders = move |_| {
        if remedy.state.get_untracked().is_pending() || remedy.recovery.blocked() {
            return;
        }
        let Some(run) = execution_run.get_untracked() else {
            return;
        };
        remedy.submit.run(run);
    };
    let submit = move |_| {
        if action.state.get_untracked().is_pending() || action.recovery.blocked() || remedy.recovery.blocked() {
            return;
        }
        let current_preview = preview.get_untracked();
        let now_ms = current_preview.current_time_ms();
        artifact.clock.set(now_ms);
        if current_preview.ticket_needs_refresh_at(now_ms) {
            preview_refresh_nonce.update(|value| *value = value.wrapping_add(1));
            return;
        }
        let artifact_ready =
            artifact_is_ready(&artifact_state.get_untracked(), &current_preview, now_ms);
        let validation_ready = artifact_validation_is_ready(
            &artifact_validation.get_untracked(),
            &artifact_state.get_untracked(),
            &current_preview,
            now_ms,
        );
        if !can_submit.get_untracked() || !artifact_ready || !validation_ready {
            let problem = preview_state.with_untracked(|state| state.problem().cloned());
            action.state.set(if !artifact_ready {
                blocked_state("执行计划尚未通过校验")
            } else if !validation_ready {
                blocked_state("请先完成执行计划的服务端重新验证")
            } else if !reviewed.get_untracked() {
                blocked_state("请先核对并勾选执行计划")
            } else {
                preview_blocked_state(problem)
            });
            return;
        }
        match confirm_request(current_preview) {
            Some(request) => action.submit.run(request),
            None => action.state.set(blocked_state("需要 API 预览")),
        }
    };

    view! {
        <section class="execution-actionbar">
            <div class="run-state">
                <span>{move || visible_run_label.get()}</span>
                <em class="run-state-detail">
                    {move || {
                        let state = action.state.get();
                        let problem = preview_problem.get();
                        state.problem().or(problem.as_ref()).map(|problem| {
                            let text: String = problem.message.chars().take(96).collect();
                            if text.len() < problem.message.len() { format!("{text}...") } else { text }
                        })
                            .unwrap_or_else(|| selection.get().pair)
                    }}
                </em>
                <Show when=move || !matches!(remedy.state.get(), ActionState::Idle)>
                    <em class="remedy-state">
                        {move || {
                            let state = remedy.state.get();
                            let label = state.label().unwrap_or_default();
                            if remedy_is_current.get() { label.to_owned() } else { format!("上次撤单 · {label}") }
                        }}
                    </em>
                </Show>
                <Show when=move || remedy_is_current.get() && remedy.state.get().problem().is_some_and(|problem|
                    problem.code == shared_types::problem::codes::ORDER_ACCOUNT_MISMATCH)>
                    <em class="run-state-detail" role="status" aria-label="撤单处理提示">
                        "恢复原账户后核对剩余挂单，不会自动重发。"
                        {move || needs_position_close.get().then_some("已有成交；撤单不等于平仓，请到持仓页处理。")}
                    </em>
                </Show>
                <Show when=move || !matches!(action.state.get(), ActionState::Idle) || preview_problem.get().is_some()>
                    <details class="execution-disclosure action-feedback">
                        <summary>{move || if previous_action.get() { "上次提交处理结果" } else { "提交处理结果" }}</summary>
                        <div class="execution-receipt-body">
                            <p>{move || action_detail(&action.state.get(), &selection.get().pair, preview_problem.get().as_ref())}</p>
                        </div>
                    </details>
                </Show>
                <Show when=move || !matches!(remedy.state.get(), ActionState::Idle)>
                    <details class="execution-disclosure cancel-feedback">
                        <summary>{move || if remedy_is_current.get() { "撤单处理结果" } else { "上次撤单处理结果" }}</summary>
                        <div class="execution-receipt-body">
                            <p>{move || remedy_detail(&remedy.state.get())}</p>
                            <pre>{move || remedy.state.get().problem().and_then(|problem| problem.details.as_ref())
                                .and_then(|details| serde_json::to_string_pretty(details).ok())}</pre>
                        </div>
                    </details>
                </Show>
                {outcome_detail::confirm_outcome(action.last_outcome)}
                {context_detail::confirm_context(action.context)}
                <Show when=move || action.recovery.blocked()>
                    <em class="run-state-detail" role="status">
                        {move || action.recovery.storage_problem.get().map(|problem| problem.message)
                            .unwrap_or_else(|| "原提交尚在核对；刷新只查询原单，不重新下单".into())}
                    </em>
                </Show>
            </div>
            <div class="execution-action-buttons">
                <button
                    class="dryrun-action"
                    disabled=move || action.recovery.sending.get() || remedy.recovery.blocked()
                    on:click=refresh_preview
                >
                    {move || if action.recovery.blocked() { "查询提交结果" } else { "刷新预览" }}
                </button>
                <button
                    class="confirm-action primary"
                    class:live=move || preview.get().execution_mode_label == "实盘"
                    title=move || submit_button_title(execution_run.get().as_ref(), &preview.get())
                    disabled=move || action.state.get().is_pending() || !can_submit.get()
                    on:click=submit
                >
                    {move || submit_button_label(execution_run.get().as_ref(), &preview.get())}
                </button>
                <Show when=move || cancel_orders_visible.get()>
                    <button
                        class="confirm-action cancel"
                        title="撤销当前 run 未成交腿的交易所挂单"
                        disabled=move || !can_cancel_orders.get() || remedy.recovery.blocked()
                        on:click=cancel_orders
                    >
                        {move || if matches!(remedy.state.get(), ActionState::Accepted { .. }) { "撤单待确认" } else { "撤单" }}
                    </button>
                </Show>
                <Show when=move || needs_position_close.get()>
                    <a
                        class="confirm-action live close-handoff"
                        href=move || execution_run.get().map(|run| crate::panels::routing::execution_run_href(crate::panels::workstation::ModuleId::Positions, &run))
                        title="run 已有成交敞口：平仓走持仓模块的快照校验流程"
                    >
                        "去持仓平仓"
                    </a>
                    <a class="confirm-action" href=move || execution_run.get().map(|run| crate::panels::routing::execution_run_href(crate::panels::workstation::ModuleId::Review, &run))>"关联复盘"</a>
                </Show>
                <Show when=move || reset_visible.get()>
                    <button
                        class="confirm-action reset"
                        title="仅重置本地提交状态，不撤单、不平仓"
                        disabled=move || action.state.get().is_pending() || action.recovery.blocked()
                            || matches!(remedy.state.get(), ActionState::Pending { .. } | ActionState::Accepted { .. })
                        on:click=reset
                    >
                        "重置状态"
                    </button>
                </Show>
            </div>
        </section>
    }
}

fn reset_preview(
    action: ConfirmHedgeAction,
    reviewed: RwSignal<bool>,
    preview_refresh_nonce: RwSignal<u64>,
) {
    if action.recovery.blocked() {
        return;
    }
    action.state.set(ActionState::Idle);
    action.last_outcome.set(None);
    action.context.set(None);
    reviewed.set(false);
    preview_refresh_nonce.update(|value| *value = value.wrapping_add(1));
}

fn reset_action_bar(action: ConfirmHedgeAction, remedy: CancelRunOrdersAction) {
    if action.recovery.blocked()
        || remedy.recovery.blocked()
        || matches!(
            remedy.state.get_untracked(),
            ActionState::Pending { .. } | ActionState::Accepted { .. }
        )
    {
        return;
    }
    action.state.set(ActionState::Idle);
    action.last_outcome.set(None);
    action.context.set(None);
    remedy.state.set(ActionState::Idle);
}

#[derive(Clone, Copy)]
struct SubmitEnabledInputs {
    action_state: RwSignal<ActionState>,
    preview_state: RwSignal<LoadState<ExecutionPreview>>,
    preview: Memo<ExecutionPreview>,
    execution_run: RwSignal<Option<ExecutionRun>>,
    ticket_clock_ms: RwSignal<i64>,
    artifact_state: RwSignal<LoadState<Option<DeterministicExecutionArtifact>>>,
    artifact_validation: RwSignal<LoadState<Option<ExecutionArtifactValidationResponse>>>,
    reviewed: RwSignal<bool>,
    recovery: super::super::data::SubmissionRecovery,
    cancel_recovery: super::super::data::CancelRecovery,
}

fn submit_enabled_memo(inputs: SubmitEnabledInputs) -> Memo<bool> {
    Memo::new(move |_| {
        let now_ms = inputs.ticket_clock_ms.get();
        let current_preview = inputs.preview.get();
        !inputs.action_state.get().is_pending()
            && !inputs.recovery.blocked()
            && !inputs.cancel_recovery.blocked()
            && inputs
                .preview_state
                .with(|state| ready_preview_can_submit(state, now_ms))
            && current_preview.can_submit_at(now_ms)
            && artifact_is_ready(&inputs.artifact_state.get(), &current_preview, now_ms)
            && artifact_validation_is_ready(
                &inputs.artifact_validation.get(),
                &inputs.artifact_state.get(),
                &current_preview,
                now_ms,
            )
            && inputs.reviewed.get()
            && !run_blocks_new_submission(inputs.execution_run.get().as_ref(), &current_preview)
    })
}

fn visible_run_label_memo(
    action_state: RwSignal<ActionState>,
    preview: Memo<ExecutionPreview>,
    execution_run: RwSignal<Option<ExecutionRun>>,
    selection: Memo<ExecutionSelection>,
    expired: Memo<bool>,
) -> Memo<String> {
    Memo::new(move |_| {
        let current_preview = preview.get();
        let current_run = execution_run.get();
        let state = action_state.get();
        let previous_action = action_is_previous(&state, &current_preview);
        let draft_state = if previous_action { &ActionState::Idle } else { &state };
        let label = if matches!(draft_state, ActionState::Idle)
            && !current_run.as_ref().is_some_and(|run| run_matches_preview(run, &current_preview))
            && expired.get()
        {
            "票据已过期 · 请刷新预览".into()
        } else {
            contextual_run_label(
                draft_state,
                current_run
                    .as_ref()
                    .filter(|run| run_matches_preview(run, &current_preview)),
                !selection.get().opportunity_id.trim().is_empty(),
            )
        };
        if previous_action {
            let previous_run = current_run.as_ref().filter(|run| {
                state.evidence().and_then(|evidence| evidence.run_id.as_deref())
                    == Some(run.run_id.as_str())
            });
            format!("{label} · 上一笔：{}", run_label(&state, previous_run))
        } else {
            label
        }
    })
}

fn can_cancel_orders_memo(
    remedy_state: RwSignal<ActionState>,
    execution_run: RwSignal<Option<ExecutionRun>>,
    orders: Memo<Vec<shared_types::OrderRecord>>,
) -> Memo<bool> {
    Memo::new(move |_| {
        !matches!(
            remedy_state.get(),
            ActionState::Pending { .. } | ActionState::Accepted { .. }
        ) && execution_run
            .get()
            .is_some_and(|run| !cancelable_order_ids_with_records(&run, &orders.get()).is_empty())
    })
}

fn cancel_orders_visible_memo(execution_run: RwSignal<Option<ExecutionRun>>) -> Memo<bool> {
    Memo::new(move |_| {
        execution_run
            .get()
            .is_some_and(|run| !cancelable_order_ids(&run).is_empty())
    })
}

fn needs_position_close_memo(
    execution_run: RwSignal<Option<ExecutionRun>>,
    orders: Memo<Vec<shared_types::OrderRecord>>,
) -> Memo<bool> {
    Memo::new(move |_| {
        execution_run.get().is_some_and(|run| {
            run_needs_position_close(&run) || run_orders_have_fill(&run, &orders.get())
        })
    })
}

fn blocked_state(message: &'static str) -> ActionState {
    ActionState::failed(
        "提交阻断",
        ApiProblem::new("HEDGE_PREVIEW_NOT_READY", message),
    )
}

fn preview_blocked_state(problem: Option<ApiProblem>) -> ActionState {
    problem.map_or_else(
        || blocked_state("需要先通过 API 预览"),
        |problem| ActionState::failed("预览阻断", problem),
    )
}

fn ready_preview_can_submit(state: &LoadState<ExecutionPreview>, now_ms: i64) -> bool {
    matches!(state, LoadState::Ready(preview) if preview.can_submit_at(now_ms))
}

#[cfg(test)]
#[path = "action_bar/tests.rs"]
mod tests;
