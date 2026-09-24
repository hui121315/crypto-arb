use crate::panels::modules::execution::ExecutionSelectionSeed;
use crate::panels::modules::opportunity_counts::{
    opportunity_empty_label, opportunity_kpi_placeholder, OpportunityEmptyLabelInput,
};
use crate::panels::modules::opportunity_eligibility::{
    opportunity_eligibility_filter, OpportunityEligibilityFilter, OpportunityEligibilitySummary,
};
use crate::panels::modules::opportunity_toolbar_state::opportunity_snapshot_usable;
use crate::panels::modules::ExecutionRuntime;
use crate::panels::shared::{
    deterministic_flow_rail, webhook_flow_stage, webhook_monitor_disclosure,
    DeterministicFlowStage, DeterministicFlowState, ModuleHeader, Surface,
};
use crate::panels::workstation::ModuleId;
use crate::state::load_state::LoadState;
use crate::state::strategy_kinds::use_strategy_kinds;
use leptos::prelude::*;
use shared_types::{WebhookEventKind, WebhookRuntimeStatus};
use wasm_bindgen::JsCast;

use super::components::{detail_panel, opportunity_table, OpportunityTableInput};
use super::data::{
    detail_seed_at, detail_seed_from_row, filter_rows, merge_symbol_opportunity_rows,
    summarize_rows, use_opportunities, use_opportunity_detail, use_opportunity_webhook,
    use_symbol_opportunities, OpportunitiesRuntime, OpportunityDetailSeed, OpportunityRow,
};

mod interactions;
mod projection;
mod sections;
mod support;

use interactions::{bind_opportunity_selection, focus_opportunity_element, opportunity_callbacks};
use projection::opportunity_projection;
use sections::{
    arbitrage_stream_toolbar_signals, opportunities_kpis, opportunity_page_bindings,
    opportunity_toolbar, use_visible_opportunity_problem, OpportunityPageBindingInput,
    OpportunityToolbarInput,
};
use support::{opportunity_local_filter_active, opportunity_symbol_search_active};

pub(in crate::panels) fn opportunities_module(
    runtime: OpportunitiesRuntime,
    execution_runtime: ExecutionRuntime,
    active_module: RwSignal<ModuleId>,
) -> impl IntoView {
    let (selected_idx, selected_opp_id) = (runtime.selected_idx, runtime.selected_opp_id);
    let selected_detail = runtime.selected_detail;
    let filter = runtime.filter;
    let strategy_kinds = use_strategy_kinds();
    let strategy_kinds_state = strategy_kinds.state;
    let (stream_channel_state, stream_stale) = arbitrage_stream_toolbar_signals();
    let store = use_opportunities(runtime);
    let stream_state = store.state;
    let rows_signal = store.rows;
    let meta_signal = store.meta;
    let page_signal = store.page;
    let loading_signal = store.loading;
    let problem_signal = use_visible_opportunity_problem(stream_state);
    let load_cursor = store.load_cursor;
    let search = use_symbol_opportunities(runtime);
    let search_rows_signal = search.rows;
    let search_meta_signal = search.meta;
    let search_page_signal = search.page;
    let search_state = search.state;
    let query_current = search.query_current;
    let search_loading = Memo::new(move |_| {
        !query_current.get() || matches!(search_state.get(), LoadState::Loading)
    });
    let search_load_cursor = search.load_cursor;
    let eligibility_filter = RwSignal::new(OpportunityEligibilityFilter::All);
    let projection = opportunity_projection(
        filter,
        rows_signal,
        search_rows_signal,
        search_meta_signal,
        search_page_signal,
        query_current,
        eligibility_filter,
    );
    let symbol_search_active = projection.symbol_search_active;
    let rows = projection.rows;
    let effective_filter = projection.effective_filter;
    let filtered_rows = projection.filtered_rows;
    let eligibility_summary = projection.eligibility_summary;
    let visible_rows = projection.visible_rows;
    let active_meta = Memo::new(move |_| {
        if symbol_search_active.get() {
            search_meta_signal.get()
        } else {
            meta_signal.get()
        }
    });
    let summary = Memo::new(move |_| {
        summarize_rows(&rows.get(), &effective_filter.get(), &active_meta.get())
    });
    let active_state = Memo::new(move |_| {
        if symbol_search_active.get() {
            search_state.get()
        } else {
            stream_state.get()
        }
    });
    let active_loading = Memo::new(move |_| {
        if symbol_search_active.get() {
            search_loading.get()
        } else {
            loading_signal.get()
        }
    });
    let snapshot_usable = Memo::new(move |_| {
        !active_loading.get() && opportunity_snapshot_usable(&active_state.get(), &active_meta.get())
    });
    let kpi_placeholder = Memo::new(move |_| {
        opportunity_kpi_placeholder(
            &active_state.get(),
            &active_meta.get(),
            filtered_rows.get().len(),
        )
        .map(str::to_owned)
    });
    let empty_label = Memo::new(move |_| {
        let filter = filter.get();
        let meta = active_meta.get();
        let search_state = search_state.get();
        opportunity_empty_label(&OpportunityEmptyLabelInput {
            noun: "候选机会",
            stream_state: &stream_state.get(),
            meta: &meta,
            stream_problem: problem_signal.get().as_ref(),
            search_loading: matches!(search_state, LoadState::Loading),
            search_problem: search_state.problem(),
            search_active: opportunity_symbol_search_active(&filter),
            search_rows_count: search_rows_signal.get().len(),
            local_filter_active: opportunity_local_filter_active(&filter),
        })
    });
    let table_empty_label = Memo::new(move |_| {
        let eligibility = eligibility_filter.get();
        if !eligibility.is_all() && filtered_rows.with(|rows| !rows.is_empty()) {
            format!("本页没有“{}”候选", eligibility.label())
        } else {
            empty_label.get()
        }
    });
    let page_bindings = opportunity_page_bindings(OpportunityPageBindingInput {
        filter,
        symbol_search_active,
        page_signal,
        loading_signal,
        load_cursor,
        search_page_signal,
        search_loading,
        search_load_cursor,
    });
    let detail = use_opportunity_detail(runtime);
    let webhook = use_opportunity_webhook();
    bind_opportunity_selection(visible_rows, selected_idx, selected_opp_id, selected_detail, runtime.requested_opp_id);
    Effect::new(move |_| {
        if runtime.requested_opp_id.get().is_some() {
            eligibility_filter.set(OpportunityEligibilityFilter::All);
        }
    });
    let callbacks = opportunity_callbacks(
        visible_rows,
        snapshot_usable,
        selected_idx,
        selected_opp_id,
        selected_detail,
        runtime.requested_opp_id,
        execution_runtime,
        active_module,
    );
    view! {
        <section class="module-page opportunities-page">
            <ModuleHeader title="机会扫描"/>
            <Show when=move || runtime.requested_opp_id.get().is_some()>
                <div class="opportunity-route-context" role="status">
                    <div>
                        <strong>{move || {
                            let found = visible_rows.with(|rows| rows.iter().any(|row| Some(&row.id) == runtime.requested_opp_id.get().as_ref()));
                            if active_loading.get() { "正在定位原机会" }
                            else if active_state.get().problem().is_some() { "读取异常，原机会待确认" }
                            else if !found { "当前列表未找到原机会" }
                            else if !snapshot_usable.get() { "已定位原机会，当前证据待更新" }
                            else { "已定位原机会，构建仍需当前预检" }
                        }}</strong>
                        <span>{move || runtime.requested_opp_id.get()}</span>
                        <small>"未找到可能是已失效或不在当前筛选/分页内；不会自动替换为其他机会。"</small>
                    </div>
                    <button class="btn-secondary" type="button" on:click=move |_| runtime.requested_opp_id.set(None)>"取消定位"</button>
                </div>
            </Show>
            {opportunities_kpis(summary, kpi_placeholder)}
            <div class="opportunity-decision-rail">
                {opportunity_flow(Memo::new(move |_| visible_rows.with(|rows| rows.iter().find(|row| row.id == selected_opp_id.get()).cloned())), webhook.state, snapshot_usable)}
            </div>
            <Surface title="候选机会" meta="实时快照" class_name="full-surface">
                {opportunity_toolbar(OpportunityToolbarInput {
                    filter,
                    strategy_kinds_state,
                    stream_channel_state,
                    stream_stale,
                    list_state: stream_state,
                    meta_signal,
                    problem_signal,
                    search_loading,
                    search_state,
                    search_meta_signal,
                    search_retry: Callback::new(move |_| search_load_cursor.run(runtime.search.cursor.get_untracked())),
                })}
                {opportunity_eligibility_filter(eligibility_summary, eligibility_filter)}
                <div class="opportunity-layout">
                    {opportunity_table(OpportunityTableInput {
                        opportunities: visible_rows,
                        selected_id: selected_opp_id,
                        page: page_bindings.page,
                        page_loading: page_bindings.loading,
                        empty_label: table_empty_label,
                        snapshot_usable,
                        on_page: page_bindings.on_page,
                        on_select: callbacks.inspect,
                        on_evidence: callbacks.inspect_detail,
                        on_open: callbacks.open,
                    })}
                    <div class="opportunity-context-column">
                        {detail_panel(detail)}
                        <section class="opportunity-delivery-rail" aria-label="提醒投递">
                            {webhook_monitor_disclosure("预检通过机会 Webhook", WebhookEventKind::Opportunity, webhook.state,
                                webhook.action_problem, webhook.test, Some(webhook.feedback))}
                        </section>
                    </div>
                </div>
            </Surface>
        </section>
    }
}

fn opportunity_flow(
    selected: Memo<Option<OpportunityRow>>,
    webhook: RwSignal<LoadState<WebhookRuntimeStatus>>,
    usable: Memo<bool>,
) -> impl IntoView {
    let summary = Memo::new(move |_| {
        let row = selected.get();
        if row.is_some() && !usable.get() {
            (
                "blocked",
                "等待新快照",
                "当前候选仅供观察；读取恢复后可构建".to_owned(),
            )
        } else {
            opportunity_flow_summary(row.as_ref())
        }
    });
    let stages = move || {
        let selected = selected.get();
        let row = selected.as_ref();
        let qualified = match row {
            Some(row) if row.execution_eligible && usable.get() => DeterministicFlowStage::new(
                "资格判定",
                format!("{} · 证据通过", row.pair),
                DeterministicFlowState::Complete,
            ),
            Some(row) => DeterministicFlowStage::new(
                "资格判定",
                row.execution_blockers
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "仅观察".to_owned()),
                DeterministicFlowState::Blocked,
            ),
            None => {
                DeterministicFlowStage::new("资格判定", "选择候选", DeterministicFlowState::Idle)
            }
        };
        let artifact_state = if usable.get() && row.is_some_and(|row| row.execution_eligible) {
            DeterministicFlowState::Current
        } else {
            DeterministicFlowState::Idle
        };
        vec![
            qualified,
            DeterministicFlowStage::new("工件重验", "进入执行页生成", artifact_state),
            webhook_flow_stage(&webhook.get(), WebhookEventKind::Opportunity),
            DeterministicFlowStage::new("双腿提交", "尚未提交", DeterministicFlowState::Idle),
            DeterministicFlowStage::new("ACK / 终态", "等待运行单", DeterministicFlowState::Idle),
            DeterministicFlowStage::new("保护退出", "等待配对持仓", DeterministicFlowState::Idle),
            DeterministicFlowStage::new("复盘", "等待终态", DeterministicFlowState::Idle),
        ]
    };
    view! {
        <section class="opportunity-readiness" data-state=move || summary.get().0 aria-live="polite">
            <div class="opportunity-readiness-current">
                <span>"当前判断"</span>
                <strong>{move || summary.get().1}</strong>
                <em title=move || summary.get().2>{move || summary.get().2}</em>
            </div>
            <button
                type="button"
                class="opportunity-locate-action"
                aria-controls="opportunity-detail-panel"
                disabled=move || selected.get().is_none()
                title="查看当前选择的完整证据"
                on:click=move |_| {
                    focus_opportunity_element("opportunity-detail-panel", true);
                }
            >
                "查看当前证据"
            </button>
            <details class="opportunity-flow-details">
                <summary>"查看完整闭环"</summary>
                {move || deterministic_flow_rail("套利执行闭环", stages())}
            </details>
        </section>
    }
}

fn opportunity_flow_summary(row: Option<&OpportunityRow>) -> (&'static str, &'static str, String) {
    match row {
        Some(row) if row.execution_eligible => (
            "ready",
            "可进入预检",
            format!(
                "{} · 当前费后净利 {} · 构建后重验",
                opportunity_identity(row),
                row.one_cycle_net
            ),
        ),
        Some(row) => (
            "blocked",
            "仅观察",
            format!(
                "{} · 测算边际 {}（未通过预检） · {}",
                opportunity_identity(row),
                row.one_cycle_net,
                row.execution_blockers
                    .first()
                    .map(String::as_str)
                    .unwrap_or("执行资格证据未通过")
            ),
        ),
        None => ("idle", "未选择候选", "从候选表选择一行查看证据".to_owned()),
    }
}

fn opportunity_identity(row: &OpportunityRow) -> String {
    format!("{} · {}→{}", row.pair, row.long_venue, row.short_venue)
}
