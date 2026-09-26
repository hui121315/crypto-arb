//! 执行状态条视图组件：状态/阶段/提示/成本/成交腿装配。
//! 纯文案格式化见 `format.rs`，运行态派生见 `state.rs`，测试见 `testing.rs`。

#[path = "execution_status_bar/format.rs"]
mod format;
#[path = "execution_status_bar/state.rs"]
mod state;
#[cfg(test)]
#[path = "execution_status_bar/testing.rs"]
mod testing;

use leptos::prelude::*;
use shared_types::{
    ApiProblem, ExecutionRun, ExecutionRunLeg, ExecutionRunLegEvidence, ExecutionRunState,
    ExecutionRunTimelineEvent, HedgeTicketView,
};

use crate::api::ws::WsChannelState;
use crate::panels::modules::execution::data::ExecutionPreview;

use format::{
    actual_cost_text, delta_cost_text, fill_text, funding_actual_text, leg_evidence_text,
    leg_label, money, open_actual_cost_text, timeline_event_class, timeline_event_meta,
    timeline_event_title,
};
use state::{
    reason_text_with_channel, stage_class, state_notice_class, state_notice_text, state_text,
    status_meta_text_with_channel,
};
pub(in crate::panels::modules::execution) use state::{run_requires_attention, run_state_label};

const STAGES: &[(u8, &str)] = &[
    (1, "交易检查"),
    (2, "第一腿"),
    (3, "第二腿"),
    (4, "补救"),
    (5, "最终结果"),
];

pub(in crate::panels::modules::execution) fn execution_status_bar(
    preview: Memo<ExecutionPreview>,
    workflow: RwSignal<Option<HedgeTicketView>>,
    run: RwSignal<Option<ExecutionRun>>,
    seed_problem: RwSignal<Option<ApiProblem>>,
    stream_problem: RwSignal<Option<ApiProblem>>,
    channel_state: RwSignal<WsChannelState>,
    submission_pending: Memo<bool>,
    read_only: bool,
) -> impl IntoView {
    let visible_run = Memo::new(move |_| {
        let preview = preview.get();
        let workflow = workflow.get();
        let workflow_run_id = workflow
            .as_ref()
            .and_then(|view| view.execution_run.as_ref())
            .and_then(|view| view.key.run());
        visible_run_for_context(run.get(), preview.ticket_id.as_deref(), workflow_run_id)
    });
    // Keep render subscriptions inside this view's lifetime, not on shared WS state.
    let status_meta = Memo::new(move |_| {
        status_meta_text_with_channel(
            visible_run.get().as_ref(),
            seed_problem.get().as_ref(),
            stream_problem.get().as_ref(),
            Some(&channel_state.get()),
        )
    });
    let reason = Memo::new(move |_| {
        reason_text_with_channel(
            visible_run.get().as_ref(),
            seed_problem.get().as_ref(),
            stream_problem.get().as_ref(),
            Some(&channel_state.get()),
        )
    });
    view! {
        <section class="execution-status-bar">
            <div class="execution-section-head">
                <div>
                    <span>"执行状态"</span>
                    <strong>{move || if submission_pending.get() { "原提交待核对".to_owned() }
                        else if read_only && visible_run.get().is_none() { "原执行记录待确认".to_owned() }
                        else { state_text(visible_run.get().as_ref()) }}</strong>
                </div>
                <em>{move || status_meta.get()}</em>
            </div>
            <div class="execution-steps">
                <For
                    each=move || STAGES.to_vec()
                    key=|step| step.0
                    children=move |(stage, label)| {
                        view! {
                            <span class=move || stage_class(visible_run.get().as_ref(), stage)>{label}</span>
                        }
                    }
                />
            </div>
            <RunStateNotice run=visible_run/>
            <p>{move || if read_only && visible_run.get().is_none() && seed_problem.get().is_none()
                && stream_problem.get().is_none() && channel_state.get().last_error.is_none() {
                    "仅查询原执行记录，不会创建或提交订单".to_owned()
                } else { reason.get() }}</p>
            <CostSummary run=visible_run/>
            <FillRows run=visible_run/>
            <RunTimeline run=visible_run/>
        </section>
    }
}

#[component]
fn RunStateNotice(run: Memo<Option<ExecutionRun>>) -> impl IntoView {
    view! {
        <Show when=move || run.get().as_ref().and_then(state_notice_text).is_some()>
            <div class=move || {
                run.get()
                    .as_ref()
                    .map(state_notice_class)
                    .unwrap_or("execution-state-notice")
            }>
                {move || {
                    run.get()
                        .as_ref()
                        .and_then(state_notice_text)
                        .unwrap_or_default()
                }}
            </div>
        </Show>
    }
}

#[component]
fn CostSummary(run: Memo<Option<ExecutionRun>>) -> impl IntoView {
    view! {
        <Show when=move || run.get().and_then(|row| row.cost_reconciliation).is_some()>
            <div class="execution-cost-summary">
                {move || {
                    run.get().and_then(|row| {
                        let is_closed = row.state == ExecutionRunState::Closed;
                        row.cost_reconciliation
                            .map(|cost| cost_summary_view(&cost, is_closed))
                    })
                }}
            </div>
        </Show>
    }
}

#[component]
fn FillRows(run: Memo<Option<ExecutionRun>>) -> impl IntoView {
    view! {
        <Show when=move || run.get().is_some()>
            <div class="execution-fill-grid">
                {move || {
                    run.get().map(|run| {
                        view! {
                            <FillLeg leg=run.long_leg evidence=run.evidence.long_leg/>
                            <FillLeg leg=run.short_leg evidence=run.evidence.short_leg/>
                        }
                    })
                }}
            </div>
        </Show>
    }
}

#[component]
fn FillLeg(leg: ExecutionRunLeg, evidence: ExecutionRunLegEvidence) -> impl IntoView {
    let evidence_text = leg_evidence_text(&leg, &evidence);
    let evidence_title = evidence_text.clone();
    view! {
        <div>
            <strong>{leg_label(&leg)}</strong>
            <span>{fill_text(&leg)}</span>
            <em title=evidence_title>{evidence_text}</em>
        </div>
    }
}

#[component]
fn RunTimeline(run: Memo<Option<ExecutionRun>>) -> impl IntoView {
    view! {
        <Show when=move || run.get().is_some_and(|run| !run.evidence.events.is_empty())>
            <div class="execution-timeline">
                <div class="execution-timeline-head">
                    <strong>"执行时间线"</strong>
                    <span>{move || timeline_count_text(run.get().as_ref())}</span>
                </div>
                <For
                    each=move || {
                        run.get()
                            .map(|run| run.evidence.events)
                            .unwrap_or_default()
                    }
                    key=|event| event.event_id.clone()
                    children=timeline_event_view
                />
            </div>
        </Show>
    }
}

fn visible_run_for_context(
    run: Option<ExecutionRun>,
    preview_ticket_id: Option<&str>,
    workflow_run_id: Option<&str>,
) -> Option<ExecutionRun> {
    match preview_ticket_id {
        Some(ticket_id) => run.filter(|run| {
            run.ticket_id == ticket_id || workflow_run_id == Some(run.run_id.as_str())
        }),
        None => run,
    }
}

fn timeline_event_view(event: ExecutionRunTimelineEvent) -> impl IntoView {
    let class = timeline_event_class(&event);
    let title = timeline_event_title(&event);
    let meta = timeline_event_meta(&event);
    view! {
        <div class=class data-event-kind=format!("{:?}", event.kind)>
            <strong>{title}</strong>
            <span>{event.message}</span>
            <em>{meta}</em>
        </div>
    }
}

fn timeline_count_text(run: Option<&ExecutionRun>) -> String {
    let Some(run) = run else {
        return String::new();
    };
    let visible = run.evidence.events.len();
    if run.evidence.dropped_event_count == 0 {
        format!("{visible} 条")
    } else {
        format!(
            "{visible} 条 · 已归档 {} 条",
            run.evidence.dropped_event_count
        )
    }
}

fn cost_summary_view(
    cost: &shared_types::ExecutionCostReconciliation,
    is_closed: bool,
) -> impl IntoView {
    let estimated = money(cost.estimated_total_cost_usd);
    let open_actual = open_actual_cost_text(cost.actual_open_cost_usd);
    let funding_actual = funding_actual_text(
        cost.actual_funding_usd,
        cost.funding_event_ids.len(),
        is_closed,
    );
    let actual = actual_cost_text(cost.actual_cost_usd, is_closed);
    let delta = delta_cost_text(cost.cost_delta_usd, is_closed);
    view! {
        <span>"预估 "{estimated}</span>
        <span>{open_actual}</span>
        <span>{funding_actual}</span>
        <span>{actual}</span>
        <span>{delta}</span>
    }
}
