//! Stable action rows and a selected receipt; technical identifiers stay in detail.
use crate::panels::modules::pagination::{page_controls, TableRuntimeHandle};
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{ActionRun, ActionRunKind, ApiProblem};

use super::super::problem_message;
use super::labels::{
    hedge_confirm_result_summary, kind_label, mutation_detail, optional_text, problem_detail,
    problem_summary, result_json, status_class_for_run, status_label_for_run, time_label,
};
use super::ACTION_RUN_PAGE_SIZE;

pub(super) fn action_run_table(
    state: RwSignal<LoadState<Vec<ActionRun>>>,
    table: TableRuntimeHandle<ActionRun>,
    selected_id: RwSignal<Option<String>>,
) -> impl IntoView {
    let runtime = table.runtime;
    let total = table.total;
    let current_page = table.current_page;
    view! {
        {move || match state.get() {
            LoadState::Loading => view! { <div class="empty-cell">"正在读取动作账本"</div> }.into_any(),
            LoadState::Error(problem) => ledger_problem("读取动作账本失败", &problem),
            LoadState::Stale { problem, value } => ledger_problem(action_runs_stale_problem_label(!value.is_empty()), &problem),
            _ => ().into_any(),
        }}
        <div class="table-wrap">
            <table class="clean-table settings-table action-runs-table">
                <thead><tr><th>"动作"</th><th>"状态"</th><th class="action-run-target">"目标"</th><th class="action-run-updated">"更新"</th><th>"详情"</th></tr></thead>
                <tbody>
                    <For each=move || runtime.get().rows key=|run| run.id.clone() children=move |initial| {
                        let run = Memo::new(move |_| runtime.with(|table| table.rows.iter().find(|run| run.id == initial.id).cloned().unwrap_or_else(|| initial.clone())));
                        action_run_row(run, selected_id)
                    }/>
                    <Show when=move || state.with(|state| state.value().is_some()) && total.get() == 0>
                        <tr><td colspan="5" class="empty-cell">"暂无动作记录"</td></tr>
                    </Show>
                </tbody>
            </table>
        </div>
        <Show when=move || { total.get() > ACTION_RUN_PAGE_SIZE }>
            {page_controls(total, current_page, ACTION_RUN_PAGE_SIZE)}
        </Show>
    }
}

fn action_runs_stale_problem_label(has_cached_rows: bool) -> &'static str {
    if has_cached_rows {
        "动作账本刷新失败，显示上次结果"
    } else {
        "动作账本刷新失败，暂无已缓存记录"
    }
}

fn action_run_row(run: Memo<ActionRun>, selected_id: RwSignal<Option<String>>) -> impl IntoView {
    view! {
        <tr data-action-id=move || run.get().id class:active=move || selected_id.get().as_deref() == Some(run.get().id.as_str())>
            <td><strong>{move || kind_label(run.get().kind)}</strong>
                <em class="action-run-mobile-time">{move || optional_text(run.get().target)}</em>
                <em class="action-run-mobile-time">{move || time_label(run.get().updated_at_ms)}</em>
            </td>
            <td title=move || problem_summary(run.get().problem)><span class=move || status_class_for_run(&run.get())>{move || status_label_for_run(&run.get())}</span></td>
            <td class="action-run-target">{move || optional_text(run.get().target)}</td>
            <td class="action-run-updated">{move || time_label(run.get().updated_at_ms)}</td>
            <td><button type="button" class="row-action" aria-pressed=move || selected_id.get().as_deref() == Some(run.get().id.as_str())
                on:click=move |_| selected_id.set(Some(run.get_untracked().id))>"详情"</button></td>
        </tr>
    }
}

pub(super) fn action_run_detail(
    state: RwSignal<LoadState<Option<ActionRun>>>,
    selected_id: RwSignal<Option<String>>,
) -> impl IntoView {
    let selected = Memo::new(move |_| {
        state
            .with(|state| state.value().cloned().flatten())
            .filter(|run| selected_id.get().as_deref() == Some(run.id.as_str()))
    });
    view! {
        {move || match state.get() {
            LoadState::Loading => view! { <div class="empty-cell">"正在读取动作详情"</div> }.into_any(),
            LoadState::Error(problem) => ledger_problem("读取动作详情失败", &problem),
            LoadState::Stale { problem, .. } => ledger_problem("动作详情刷新失败，显示上次结果", &problem),
            LoadState::Ready(None) => view! { <div class="empty-cell">"选择一条动作查看详情"</div> }.into_any(),
            _ => ().into_any(),
        }}
        <For each=move || { selected.get().into_iter().collect::<Vec<_>>() } key=|run| run.id.clone() children=move |initial| {
            let run = Memo::new(move |_| selected.get().unwrap_or_else(|| initial.clone()));
            action_run_detail_card(run, selected_id)
        }/>
    }
}

fn action_run_detail_card(
    run: Memo<ActionRun>,
    selected_id: RwSignal<Option<String>>,
) -> impl IntoView {
    view! {
        <section class="settings-action-detail" aria-label="动作详情">
            <header><strong>{move || kind_label(run.get().kind)}</strong>
                <span class=move || status_class_for_run(&run.get())>{move || status_label_for_run(&run.get())}</span>
                <button type="button" class="row-action" on:click=move |_| selected_id.set(None)>"关闭详情"</button>
            </header>
            <p>{move || run.get().message}</p>
            <dl>
                <div><dt>"目标"</dt><dd>{move || optional_text(run.get().target)}</dd></div>
                <div><dt>"操作者"</dt><dd>{move || run.get().actor}</dd></div>
                <div><dt>"创建时间"</dt><dd>{move || time_label(run.get().started_at_ms)}</dd></div>
                <div><dt>"更新时间"</dt><dd>{move || time_label(run.get().updated_at_ms)}</dd></div>
            </dl>
            {move || problem_detail(run.get().problem).map(|text| view! { <p class="settings-message is-error" role="alert">{text}</p> })}
            {move || { let run = run.get(); (run.kind == ActionRunKind::HedgeConfirm).then(|| hedge_confirm_result_summary(run.result.as_ref())).flatten().map(|text| view! { <p class="settings-message">{text}</p> }) }}
            <details><summary>"请求与变更数据依据"</summary>
                <dl>
                    <div><dt>"动作编号"</dt><dd>{move || run.get().id}</dd></div>
                    <div><dt>"Request"</dt><dd>{move || optional_text(run.get().request_id)}</dd></div>
                    <div><dt>"幂等键"</dt><dd>{move || optional_text(run.get().idempotency_key)}</dd></div>
                </dl>
                {move || mutation_detail(run.get().mutation).map(|text| view! { <p>{text}</p> })}
                <pre>{move || result_json(run.get().result)}</pre>
            </details>
        </section>
    }
}

fn ledger_problem(label: &str, problem: &ApiProblem) -> AnyView {
    view! { <p class="settings-message is-error" role="alert">{problem_message(label, problem)}</p> }.into_any()
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
