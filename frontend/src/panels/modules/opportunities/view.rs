use crate::panels::modules::execution::ExecutionSelectionSeed;
use crate::panels::modules::opportunity_counts::{
    opportunity_empty_label, opportunity_kpi_placeholder, snapshot_clock, OpportunityCountMeta,
    OpportunityEmptyLabelInput,
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
use gloo_timers::callback::Interval;
use leptos::prelude::*;
use std::collections::HashSet;
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

type OpportunitySelectionScope = (
    Option<shared_types::StrategyKind>, String, Option<String>, Option<String>,
    OpportunityEligibilityFilter,
);

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
    let meta_signal = store.meta;
    let page_signal = store.page;
    let loading_signal = store.loading;
    let problem_signal = use_visible_opportunity_problem(stream_state);
    let load_cursor = store.load_cursor;
    let search = use_symbol_opportunities(runtime);
    let search_meta_signal = search.meta;
    let search_page_signal = search.page;
    let search_state = search.state;
    let query_current = search.query_current;
    let search_loading = Memo::new(move |_| {
        !query_current.get() || matches!(search_state.get(), LoadState::Loading)
    });
    let search_load_cursor = search.load_cursor;
    let clock = RwSignal::new(snapshot_clock());
    let interval = StoredValue::new_local(Some(Interval::new(1_000, move || {
        clock.set(snapshot_clock());
    })));
    on_cleanup(move || {
        interval.update_value(|slot| {
            slot.take();
        })
    });
    let list_meta = Memo::new(move |_| meta_signal.get().aged_at(clock.get()));
    let search_meta = Memo::new(move |_| search_meta_signal.get().aged_at(clock.get()));
    let live_quote_usable = Memo::new(move |_| {
        !loading_signal.get() && !stream_stale.get() && !list_meta.get().preview_age_expired()
            && opportunity_snapshot_usable(&stream_state.get(), &list_meta.get())
    });
    let search_quote_usable = Memo::new(move |_| {
        query_current.get() && !search_loading.get() && !search_meta.get().preview_age_expired()
            && opportunity_snapshot_usable(&search_state.get(), &search_meta.get())
    });
    let eligibility_filter = RwSignal::new(OpportunityEligibilityFilter::All);
    let projection = opportunity_projection(
        filter,
        &store,
        &search,
        live_quote_usable,
        search_quote_usable,
        eligibility_filter,
    );
    let symbol_search_active = projection.symbol_search_active;
    let rows = projection.rows;
    let effective_filter = projection.effective_filter;
    let filtered_rows = projection.filtered_rows;
    let eligibility_summary = projection.eligibility_summary;
    let visible_rows = projection.visible_rows;
    let quotes = projection.quotes;
    let quote_ready_ids = projection.quote_ready_ids;
    let active_meta = Memo::new(move |_| {
        if symbol_search_active.get() && query_current.get() {
            let mut meta = search_meta.get();
            if quotes.get().complete_live { meta.filtered_count = rows.with(Vec::len); }
            meta
        } else if symbol_search_active.get() {
            Default::default()
        } else {
            list_meta.get()
        }
    });
    let summary = Memo::new(move |_| {
        let ready = quote_ready_ids.get();
        let current = rows.with(|rows| rows.iter().filter(|row| ready.contains(&row.id)).cloned().collect::<Vec<_>>());
        let mut summary = summarize_rows(&current, &effective_filter.get(), &active_meta.get());
        if symbol_search_active.get() {
            summary.candidates = if quotes.get().complete_live {
                rows.with(Vec::len)
            } else {
                search_page_signal.get().map_or(0, |page| page.total_rows)
            };
        }
        summary.filtered_candidates = filtered_rows.with(Vec::len);
        summary.executable_candidates = filtered_rows.with(|rows| {
            rows.iter().filter(|row| row.execution_eligible && ready.contains(&row.id)).count()
        });
        summary
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
    let selected_quote_usable = Memo::new(move |_| quote_ready_ids.with(|ids| ids.contains(&selected_opp_id.get())));
    let kpi_placeholder = Memo::new(move |_| {
        if active_meta.get().preview_age_expired() {
            return Some("快照已过期".to_owned());
        }
        if !rows.with(Vec::is_empty) && quote_ready_ids.with(HashSet::is_empty)
            && !active_meta.get().rows_retained && !active_loading.get()
        {
            return Some("报价待更新".to_owned());
        }
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
            search_rows_count: rows.with(Vec::len),
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
    let display_search_page = Memo::new(move |_| {
        let mut page = search_page_signal.get()?;
        if quotes.get().complete_live {
            page.total_rows = rows.with(Vec::len);
            page.returned_count = page.total_rows;
        }
        Some(page)
    });
    let page_bindings = opportunity_page_bindings(OpportunityPageBindingInput {
        filter,
        symbol_search_active,
        page_signal,
        loading_signal,
        load_cursor,
        search_page_signal: display_search_page,
        search_loading,
        search_load_cursor,
    });
    let detail = use_opportunity_detail(runtime);
    let webhook = use_opportunity_webhook();
    Effect::new(move |previous: Option<OpportunitySelectionScope>| {
        let filter = filter.get();
        let scope = (filter.strategy, filter.query, runtime.list.cursor.get(),
            runtime.search.cursor.get(), eligibility_filter.get());
        if previous.is_some_and(|previous| previous != scope)
            && runtime.requested_opp_id.get_untracked().is_none()
        {
            selected_idx.set(0);
            selected_opp_id.set(String::new());
        }
        scope
    });
    bind_opportunity_selection(
        visible_rows,
        Memo::new(move |_| !active_loading.get() && !quote_ready_ids.with(HashSet::is_empty)),
        selected_idx,
        selected_opp_id,
        selected_detail,
        runtime.requested_opp_id,
    );
    Effect::new(move |_| {
        if runtime.requested_opp_id.get().is_some() {
            eligibility_filter.set(OpportunityEligibilityFilter::All);
        }
    });
    let callbacks = opportunity_callbacks(
        visible_rows,
        quote_ready_ids,
        active_meta,
        list_meta,
        Memo::new(move |_| quotes.get().live_ids),
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
            <Show when=move || runtime.requested_opp_id.get().is_some()
                || (!selected_opp_id.get().is_empty()
                    && !visible_rows.with(|rows| rows.iter().any(|row| row.id == selected_opp_id.get())))>
                <div class="opportunity-route-context" role="status">
                    <div>
                        <strong>{move || {
                            let wanted = runtime.requested_opp_id.get().unwrap_or_else(|| selected_opp_id.get());
                            let found = visible_rows.with(|rows| rows.iter().any(|row| row.id == wanted));
                            if active_loading.get() { "正在定位原机会" }
                            else if active_state.get().problem().is_some() { "读取异常，原机会待确认" }
                            else if !found { "当前列表未找到原机会" }
                            else if !selected_quote_usable.get() { "已定位原机会，当前数据依据待更新" }
                            else { "已定位原机会，构建仍需当前交易检查" }
                        }}</strong>
                        <span>{move || runtime.requested_opp_id.get().unwrap_or_else(|| selected_opp_id.get())}</span>
                        <small>"未找到可能是已失效或不在当前筛选/分页内；不会自动替换为其他机会。"</small>
                    </div>
                    <button class="btn-secondary" type="button" on:click=move |_| {
                        selected_opp_id.set(String::new());
                        runtime.requested_opp_id.set(None);
                    }>"取消定位"</button>
                </div>
            </Show>
            {opportunities_kpis(summary, kpi_placeholder)}
            <div class="opportunity-decision-rail">
                {opportunity_flow(Memo::new(move |_| visible_rows.with(|rows| rows.iter().find(|row| row.id == selected_opp_id.get()).cloned())), webhook.state, selected_quote_usable)}
            </div>
            <Surface title="候选机会" meta="报价 / 数据依据" class_name="full-surface">
                {opportunity_toolbar(OpportunityToolbarInput {
                    filter,
                    strategy_kinds_state,
                    stream_channel_state,
                    stream_stale,
                    list_state: stream_state,
                    meta_signal: list_meta,
                    problem_signal,
                    search_loading,
                    search_state,
                    search_meta_signal: search_meta,
                    search_source_label: Memo::new(move |_| {
                        let quotes = quotes.get();
                        let count = quotes.live_ids.len();
                        if count > 0 && !live_quote_usable.get() {
                            format!("WS 报价 {count} 条待更新；其他行按搜索快照核对")
                        } else if quotes.complete_live {
                            format!("完整 WS 窗口 · 当前匹配 {count} 条")
                        } else {
                            format!("WS 报价 {count} 条 · 搜索快照 {} 条", quotes.rows.len().saturating_sub(count))
                        }
                    }),
                    search_retry: Callback::new(move |_| search_load_cursor.run(runtime.search.cursor.get_untracked())),
                    list_is_paged: Memo::new(move |_| !symbol_search_active.get()
                        && runtime.list.cursor.with(Option::is_some)),
                    list_loading: loading_signal,
                    list_retry: Callback::new(move |_| load_cursor.run(runtime.list.cursor.get_untracked())),
                    list_home: Callback::new(move |_| load_cursor.run(None)),
                })}
                {opportunity_eligibility_filter(eligibility_summary, eligibility_filter, page_bindings.page)}
                <div class="opportunity-layout">
                    {opportunity_table(OpportunityTableInput {
                        opportunities: visible_rows,
                        selected_id: selected_opp_id,
                        page: page_bindings.page,
                        page_loading: page_bindings.loading,
                        empty_label: table_empty_label,
                        quote_ready_ids,
                        on_page: page_bindings.on_page,
                        on_select: callbacks.inspect,
                        on_evidence: callbacks.inspect_detail,
                        on_open: callbacks.open,
                    })}
                    <div class="opportunity-context-column">
                        {detail_panel(detail, selected_quote_usable, clock)}
                        <section class="opportunity-delivery-rail" aria-label="提醒投递">
                            {webhook_monitor_disclosure("交易检查通过机会 Webhook", WebhookEventKind::Opportunity, webhook.state,
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
                format!("{} · 数据依据通过", row.pair),
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
            DeterministicFlowStage::new("复查计划", "进入执行页生成", artifact_state),
            webhook_flow_stage(&webhook.get(), WebhookEventKind::Opportunity),
            DeterministicFlowStage::new("双腿提交", "尚未提交", DeterministicFlowState::Idle),
            DeterministicFlowStage::new("受理 / 结果", "等待运行单", DeterministicFlowState::Idle),
            DeterministicFlowStage::new("保护退出", "等待配对持仓", DeterministicFlowState::Idle),
            DeterministicFlowStage::new("复盘", "等待最终结果", DeterministicFlowState::Idle),
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
                title="查看当前选择的完整数据依据"
                on:click=move |_| {
                    focus_opportunity_element("opportunity-detail-panel", true);
                }
            >
                "查看当前数据依据"
            </button>
            <details class="opportunity-flow-details">
                <summary>"查看完整完整流程"</summary>
                {move || deterministic_flow_rail("套利执行完整流程", stages())}
            </details>
        </section>
    }
}

fn opportunity_flow_summary(row: Option<&OpportunityRow>) -> (&'static str, &'static str, String) {
    match row {
        Some(row) if row.execution_eligible => (
            "ready",
            "可进入交易检查",
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
                "{} · 测算边际 {}（未通过交易检查） · {}",
                opportunity_identity(row),
                row.one_cycle_net,
                row.execution_blockers
                    .first()
                    .map(String::as_str)
                    .unwrap_or("执行资格数据依据未通过")
            ),
        ),
        None => ("idle", "未选择候选", "从候选表选择一行查看数据依据".to_owned()),
    }
}

fn opportunity_identity(row: &OpportunityRow) -> String {
    format!("{} · {}→{}", row.pair, row.long_venue, row.short_venue)
}
