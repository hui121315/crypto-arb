//! 动作账本表格与详情卡片渲染（分页表 / 行 / 详情）。
//! 纯文案派生见 `labels.rs`，标签页装配见父模块 `action_runs.rs`。

use crate::panels::modules::pagination::{page_controls, TableRuntimeHandle};
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{ActionRun, ApiProblem};

use super::super::problem_cell;
use super::labels::{
    hedge_confirm_result_summary, kind_label, mutation_detail, optional_text, problem_detail,
    problem_summary, result_json, status_class, status_label_for_run, time_label,
};
use super::ACTION_RUN_PAGE_SIZE;

pub(super) fn action_run_table(
    state: LoadState<Vec<ActionRun>>,
    table: &TableRuntimeHandle<ActionRun>,
    selected_id: RwSignal<Option<String>>,
) -> AnyView {
    let stale_problem = match state {
        LoadState::Ready(_) => None,
        LoadState::Stale { problem, .. } => Some(problem),
        LoadState::Error(problem) => return problem_cell("读取动作账本失败", &problem),
        LoadState::Loading => {
            return view! { <div class="empty-cell">"正在读取动作账本"</div> }.into_any();
        }
    };
    if table.total.get_untracked() == 0 {
        return view! {
            <>
                {stale_problem
                    .map(|problem| problem_cell(action_runs_stale_problem_label(false), &problem))}
                <div class="empty-cell">"暂无动作记录"</div>
            </>
        }
        .into_any();
    }
    let total = table.total;
    let current_page = table.current_page;
    let runtime = table.runtime;
    let visible_rows = move || {
        runtime
            .get()
            .rows
            .into_iter()
            .map(|run| action_run_row(run, selected_id))
            .collect_view()
    };
    view! {
        <>
            {stale_problem
                .map(|problem| problem_cell(action_runs_stale_problem_label(true), &problem))}
            <div class="table-wrap">
                <table class="clean-table settings-table action-runs-table">
                    <thead>
                        <tr>
                            <th>"动作"</th>
                            <th>"状态"</th>
                            <th>"request id"</th>
                            <th>"幂等键"</th>
                            <th>"目标"</th>
                            <th>"创建"</th>
                            <th>"更新"</th>
                            <th>"问题"</th>
                            <th>"详情"</th>
                        </tr>
                    </thead>
                    <tbody>{visible_rows}</tbody>
                </table>
            </div>
            {move || {
                (total.get() > ACTION_RUN_PAGE_SIZE).then(|| view! {
                    {page_controls(total, current_page, ACTION_RUN_PAGE_SIZE)}
                })
            }}
        </>
    }
    .into_any()
}

fn action_runs_stale_problem_label(has_cached_rows: bool) -> &'static str {
    if has_cached_rows {
        "动作账本刷新失败，显示上次结果"
    } else {
        "动作账本刷新失败，暂无已缓存记录"
    }
}

fn action_run_row(run: ActionRun, selected_id: RwSignal<Option<String>>) -> impl IntoView {
    let status = status_label_for_run(&run);
    let status_class = status_class(run.status);
    let kind = kind_label(run.kind);
    let id = run.id.clone();
    let row_id = id.clone();
    let detail_id = id.clone();
    let request_id = optional_text(run.request_id);
    let idempotency_key = optional_text(run.idempotency_key);
    let target = optional_text(run.target);
    let created = time_label(run.started_at_ms);
    let updated = time_label(run.updated_at_ms);
    let problem = problem_summary(run.problem);
    view! {
        <tr class=move || {
            if selected_id
                .get()
                .as_deref()
                .is_some_and(|selected| selected == row_id.as_str())
            {
                "active"
            } else {
                ""
            }
        }>
            <td>
                <strong>{kind}</strong>
                <em>{id}</em>
            </td>
            <td><span class=status_class>{status}</span></td>
            <td>{request_id}</td>
            <td>{idempotency_key}</td>
            <td>{target}</td>
            <td>{created}</td>
            <td>{updated}</td>
            <td>{problem}</td>
            <td>
                <button
                    class="row-action"
                    on:click=move |_| selected_id.set(Some(detail_id.clone()))
                >
                    "详情"
                </button>
            </td>
        </tr>
    }
}

pub(super) fn action_run_detail(
    state: LoadState<Option<ActionRun>>,
    selected_id: RwSignal<Option<String>>,
) -> AnyView {
    match state {
        LoadState::Ready(Some(run)) => action_run_detail_card(run, selected_id, None),
        LoadState::Ready(None) => {
            view! { <div class="empty-cell">"选择一条动作查看详情"</div> }.into_any()
        }
        LoadState::Stale {
            value: Some(run),
            problem,
        } => action_run_detail_card(run, selected_id, Some(problem)),
        LoadState::Stale {
            value: None,
            problem,
        } => problem_cell("动作详情刷新失败", &problem),
        LoadState::Error(problem) => problem_cell("读取动作详情失败", &problem),
        LoadState::Loading => view! { <div class="empty-cell">"正在读取动作详情"</div> }.into_any(),
    }
}

fn action_run_detail_card(
    run: ActionRun,
    selected_id: RwSignal<Option<String>>,
    stale_problem: Option<ApiProblem>,
) -> AnyView {
    let status = status_label_for_run(&run);
    let status_class = status_class(run.status);
    let kind = kind_label(run.kind);
    let id = run.id;
    let request_id = optional_text(run.request_id);
    let idempotency_key = optional_text(run.idempotency_key);
    let target = optional_text(run.target);
    let actor = run.actor;
    let message = run.message;
    let created = time_label(run.started_at_ms);
    let updated = time_label(run.updated_at_ms);
    let problem = problem_detail(run.problem);
    let mutation = mutation_detail(run.mutation);
    let hedge_summary = hedge_confirm_result_summary(run.result.as_ref());
    let result = result_json(run.result);
    view! {
        <>
            {stale_problem.map(|problem| problem_cell("动作详情刷新失败，显示上次结果", &problem))}
            <div class="settings-section">
                <label class="settings-control">
                    <span>"动作"</span>
                    <input readonly value=kind/>
                    <em>{id}</em>
                </label>
                <label class="settings-control">
                    <span>"状态"</span>
                    <span class=status_class>{status}</span>
                    <em>{message}</em>
                </label>
                <label class="settings-control">
                    <span>"目标"</span>
                    <input readonly value=target/>
                    <em>{actor}</em>
                </label>
                <label class="settings-control">
                    <span>"Request"</span>
                    <input readonly value=request_id/>
                    <em>{created}</em>
                </label>
                <label class="settings-control">
                    <span>"幂等键"</span>
                    <input readonly value=idempotency_key/>
                    <em>{updated}</em>
                </label>
                {problem.map(|text| view! {
                    <label class="settings-control">
                        <span>"失败原因"</span>
                        <input readonly value=text/>
                        <em>"ActionRun problem"</em>
                    </label>
                })}
                {hedge_summary.map(|text| view! {
                    <label class="settings-control">
                        <span>"对冲结果"</span>
                        <input readonly value=text/>
                        <em>"HedgeConfirm partial outcome"</em>
                    </label>
                })}
                {mutation.map(|text| view! {
                    <label class="settings-control env-template-box">
                        <span>"变更"</span>
                        <textarea readonly prop:value=text/>
                        <em>"ActionRun old/new diff"</em>
                    </label>
                })}
                <label class="settings-control env-template-box">
                    <span>"结果"</span>
                    <textarea readonly prop:value=result/>
                    <em>"ActionRun result payload"</em>
                </label>
                <div class="settings-actions">
                    <button class="row-action" on:click=move |_| selected_id.set(None)>
                        "关闭详情"
                    </button>
                </div>
            </div>
        </>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_action_runs_label_distinguishes_empty_cache() {
        assert_eq!(
            action_runs_stale_problem_label(true),
            "动作账本刷新失败，显示上次结果"
        );
        assert_eq!(
            action_runs_stale_problem_label(false),
            "动作账本刷新失败，暂无已缓存记录"
        );
    }
}
