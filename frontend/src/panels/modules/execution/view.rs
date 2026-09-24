use crate::panels::shared::{ModuleHeader, OrdersList, Surface};
use leptos::prelude::*;

use super::components::{
    action_bar, artifact_inbox, execution_artifact_panel, execution_deterministic_flow, execution_status_bar,
    execution_ticket, leg_panel, params_panel, risk_preview, run_requires_attention,
    run_state_label, slippage_ladder, workflow_status,
};
use super::data::{
    use_cancel_run_orders_action, use_confirm_hedge_action, use_execution_artifact,
    ExecutionRuntime,
};
use super::draft::ExecutionDraft;
pub(in crate::panels) fn execution_module(runtime: ExecutionRuntime) -> impl IntoView {
    let selection = Memo::new(move |_| runtime.selection().get());
    let draft = ExecutionDraft::new(selection, runtime);
    let artifact = use_execution_artifact(draft.preview);
    let reviewed = RwSignal::new(false);
    let action = use_confirm_hedge_action(
        draft.preview,
        draft.execution_run,
        draft.orders,
        draft.runtime_refresh_nonce,
        draft.confirm,
    );
    let remedy = use_cancel_run_orders_action(
        draft.runtime_refresh_nonce,
        draft.cancel_state,
        draft.order_queue,
        draft.all_orders,
    );
    let has_selection = Memo::new(move |_| !selection.get().opportunity_id.trim().is_empty());
    let run_is_current = Memo::new(move |_| {
        let selection = selection.get();
        let preview = draft.preview.get();
        draft
            .execution_run
            .get()
            .as_ref()
            .is_some_and(|run| selection.matches_run(preview.ticket_id.as_deref(), run))
    });
    let current_runtime_visible = Memo::new(move |_| {
        has_selection.get()
            && (run_is_current.get()
                || draft.execution_run_seed_problem.get().is_some()
                || draft.execution_run_stream_problem.get().is_some())
    });
    let previous_run_visible =
        Memo::new(move |_| draft.execution_run.get().is_some() && !run_is_current.get());
    let current_runtime_open = Memo::new(move |_| {
        run_is_current.get()
            || draft.execution_run_seed_problem.get().is_some()
            || draft.execution_run_stream_problem.get().is_some()
    });
    let previous_runtime_open = Memo::new(move |_| {
        let run = draft.execution_run.get();
        draft.execution_run_seed_problem.get().is_some()
            || draft.execution_run_stream_problem.get().is_some()
            || run.as_ref().is_some_and(run_requires_attention)
    });
    let runtime_disclosure_label = Memo::new(move |_| {
        if run_is_current.get() {
            "当前执行状态"
        } else if draft.execution_run.get().is_some() {
            "完整运行证据"
        } else {
            "执行数据"
        }
    });
    let runtime_disclosure_summary = Memo::new(move |_| {
        if draft.execution_run_stream_problem.get().is_some() {
            return "执行流异常 · 查看详情".to_owned();
        }
        if draft.execution_run_seed_problem.get().is_some() {
            return "恢复失败 · 查看详情".to_owned();
        }
        draft
            .execution_run
            .get()
            .as_ref()
            .map(|run| format!("{} · 查看完整记录", run_state_label(run)))
            .unwrap_or_else(|| "等待运行单".to_owned())
    });
    view! {
        <section class="module-page execution-page">
            <ModuleHeader title="对冲执行"/>
            {artifact_inbox(artifact.clock)}
            <Show when=move || runtime.route_notice.get().is_some()>
                <p class="execution-history-context" role="status">{move || runtime.route_notice.get()}</p>
            </Show>
            {execution_deterministic_flow(selection, draft.preview, artifact, draft.execution_run)}
            <Show when=move || !has_selection.get() && action.recovery.blocked()>
                <section class="execution-actionbar execution-recovery" role="status">
                    <div class="run-state">
                        <span>"原提交结果待核验"</span>
                        <em class="run-state-detail">{move || action.recovery.storage_problem.get().map(|problem| problem.message)
                            .unwrap_or_else(|| "保留原请求，查询结果不会再次下单".into())}</em>
                    </div>
                    <button class="dryrun-action" disabled=move || action.recovery.sending.get()
                        on:click=move |_| draft.runtime_refresh_nonce.update(|value| *value = value.wrapping_add(1))>
                        "查询提交结果"
                    </button>
                </section>
            </Show>
            <Show
                when=move || has_selection.get()
                fallback=move || execution_idle_workspace(
                    draft,
                    run_is_current,
                    previous_run_visible,
                    runtime_disclosure_label,
                    runtime_disclosure_summary,
                    previous_runtime_open,
                    Memo::new(move |_| runtime.route_notice.get().is_some()),
                )
            >
                <div class="execution-grid">
                    <Surface title="执行票据" meta="当前 / 预检 / 提交" class_name="execution-main">
                        {execution_ticket(selection)}
                        {leg_panel(selection, draft)}
                        <div class="execution-workbench">
                            <div class="execution-config-column">
                                {params_panel(selection, draft, draft.preview_state)}
                                {slippage_ladder(draft)}
                            </div>
                            <div class="execution-risk-column">
                                {risk_preview(draft.preview, draft.preview_state)}
                            </div>
                        </div>
                        {execution_artifact_panel(artifact, reviewed)}
                        {action_bar(selection, draft, artifact, reviewed, action, remedy)}
                        <Show when=move || current_runtime_visible.get()>
                            {execution_runtime_disclosure(
                                draft,
                                runtime_disclosure_label,
                                runtime_disclosure_summary,
                                current_runtime_open,
                            )}
                        </Show>
                    </Surface>
                    <Surface title="订单与终态" meta="当前 / 上一笔" class_name="execution-side">
                        <OrdersList
                            run=draft.execution_run
                            run_is_current=run_is_current
                            orders=draft.orders
                            seed_problem=draft.order_seed_problem
                            stream_problem=draft.order_stream_problem
                            channel_state=draft.order_channel_state
                        />
                        <Show when=move || previous_run_visible.get()>
                            <section class="execution-side-history" aria-label="历史执行结果">
                                <div class="execution-history-context" role="note">
                                    <div>
                                        <strong>"历史结果 · 只读"</strong>
                                        <span>"订单和运行证据来自上一笔执行，不会作为当前票据或提交依据。"</span>
                                    </div>
                                    <a href="#review">"查看复盘"</a>
                                </div>
                                {execution_runtime_disclosure(
                                    draft,
                                    runtime_disclosure_label,
                                    runtime_disclosure_summary,
                                    previous_runtime_open,
                                )}
                            </section>
                        </Show>
                    </Surface>
                </div>
            </Show>
        </section>
    }
}

fn execution_idle_workspace(
    draft: ExecutionDraft,
    run_is_current: Memo<bool>,
    previous_run_visible: Memo<bool>,
    runtime_disclosure_label: Memo<&'static str>,
    runtime_disclosure_summary: Memo<String>,
    previous_runtime_open: Memo<bool>,
    requested_context: Memo<bool>,
) -> impl IntoView {
    let show_all_orders = RwSignal::new(false);
    let global_run = RwSignal::new(None);
    let history_scope_available = Memo::new(move |_| {
        !requested_context.get()
            && previous_run_visible.get()
            && draft
                .all_orders
                .with(|all| draft.orders.with(|run| all.len() > run.len()))
    });
    let showing_all_orders = Memo::new(move |_| {
        !requested_context.get()
            && (!previous_run_visible.get()
                || (history_scope_available.get() && show_all_orders.get()))
    });
    view! {
        <Surface
            title="订单与终态"
            meta="当前无票据 · 只读历史"
            class_name="execution-idle"
        >
            <div class="execution-idle-layout">
                <section class="execution-idle-history" aria-label="历史订单与终态">
                    <div class="execution-history-context" role="note">
                        <div>
                            <strong>{move || if requested_context.get() {
                                "指定运行 · 只读"
                            } else if showing_all_orders.get() {
                                "最近订单 · 只读"
                            } else {
                                "上一笔执行 · 只读"
                            }}</strong>
                            <span>"不会自动成为新的票据、预检或提交依据。"</span>
                        </div>
                        <a href="#review">"查看复盘"</a>
                    </div>
                    <Show when=move || history_scope_available.get()>
                        <div class="execution-history-scope" role="tablist" aria-label="历史订单范围">
                            <button
                                type="button"
                                role="tab"
                                aria-selected=move || (!show_all_orders.get()).to_string()
                                on:click=move |_| show_all_orders.set(false)
                            >
                                {move || format!("上一笔 {}", draft.orders.with(Vec::len))}
                            </button>
                            <button
                                type="button"
                                role="tab"
                                aria-selected=move || show_all_orders.get().to_string()
                                on:click=move |_| show_all_orders.set(true)
                            >
                                {move || format!("最近订单 {}", draft.all_orders.with(Vec::len))}
                            </button>
                        </div>
                    </Show>
                    <Show
                        when=move || !showing_all_orders.get()
                        fallback=move || view! {
                            <OrdersList
                                run=global_run
                                run_is_current=run_is_current
                                orders=draft.all_orders
                                seed_problem=draft.order_seed_problem
                                stream_problem=draft.order_stream_problem
                                channel_state=draft.order_channel_state
                            />
                        }
                    >
                        <Show when=move || !requested_context.get() || draft.execution_run.get().is_some()
                            fallback=|| view! { <p role="status">"指定运行尚未读取成功；不会显示其他运行的订单作为替代。"</p> }>
                            <OrdersList
                                run=draft.execution_run
                                run_is_current=run_is_current
                                orders=draft.orders
                                seed_problem=draft.order_seed_problem
                                stream_problem=draft.order_stream_problem
                                channel_state=draft.order_channel_state
                            />
                        </Show>
                    </Show>
                    <Show when=move || (previous_run_visible.get() || requested_context.get()) && !showing_all_orders.get()>
                        {execution_runtime_disclosure(
                            draft,
                            runtime_disclosure_label,
                            runtime_disclosure_summary,
                            previous_runtime_open,
                        )}
                    </Show>
                </section>
            </div>
        </Surface>
    }
}

fn execution_runtime_disclosure(
    draft: ExecutionDraft,
    label: Memo<&'static str>,
    summary: Memo<String>,
    open: Memo<bool>,
) -> impl IntoView {
    view! {
        <details class="execution-runtime-disclosure" open=move || open.get()>
            <summary>
                <span>{move || label.get()}</span>
                <strong>{move || summary.get()}</strong>
            </summary>
            <div class="execution-runtime-stack">
                {workflow_status(draft.workflow_view, draft.workflow_provenance)}
                {execution_status_bar(
                    draft.preview,
                    draft.workflow_view,
                    draft.execution_run,
                    draft.execution_run_seed_problem,
                    draft.execution_run_stream_problem,
                    draft.execution_run_channel_state,
                )}
            </div>
        </details>
    }
}
