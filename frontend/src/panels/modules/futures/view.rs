use crate::panels::modules::opportunity_counts::{
    opportunity_empty_label, opportunity_kpi_placeholder, OpportunityEmptyLabelInput,
};
use crate::panels::modules::opportunity_eligibility::{
    opportunity_eligibility_filter, OpportunityEligibilityFilter, OpportunityEligibilitySummary,
};
use crate::panels::modules::ExecutionRuntime;
use crate::panels::shared::{ModuleHeader, Surface};
use crate::panels::workstation::ModuleId;
use crate::state::load_state::LoadState;
use crate::state::strategy_kinds::use_strategy_kinds;
use leptos::prelude::*;

use super::columns::default_visible_for_strategy;
use super::components::{futures_opportunity_table, FuturesOpportunityTableInput};
use super::data::{
    filter_rows, merge_symbol_futures_rows, summarize_rows, use_futures_opportunities,
    use_symbol_futures_opportunities, FuturesRuntime, StrategyFilter,
};

#[path = "view/sections.rs"]
mod sections;
use sections::{
    arbitrage_stream_toolbar_signals, futures_kpis, futures_local_filter_active,
    futures_page_bindings, futures_position_entry_context, futures_symbol_search_active,
    futures_toolbar, use_visible_futures_problem, FuturesPageBindingInput, FuturesToolbarInput,
};

#[derive(Clone, Copy)]
struct FuturesWorkspaceViewInput {
    position_entry_symbol: RwSignal<Option<String>>,
    filter: RwSignal<super::data::FuturesFilter>,
    summary: Memo<super::data::FuturesSummary>,
    kpi_placeholder: Memo<Option<String>>,
    active_strategy: Memo<StrategyFilter>,
    toolbar: FuturesToolbarInput,
    eligibility_summary: Memo<OpportunityEligibilitySummary>,
    eligibility_filter: RwSignal<OpportunityEligibilityFilter>,
    table: FuturesOpportunityTableInput,
}

#[derive(Clone, Copy)]
struct FuturesEmptyLabelsInput {
    filter: RwSignal<super::data::FuturesFilter>,
    active_meta: Memo<crate::panels::modules::opportunity_counts::OpportunityCountMeta>,
    search_state: RwSignal<LoadState<()>>,
    stream_state: RwSignal<LoadState<()>>,
    problem_signal: Memo<Option<shared_types::ApiProblem>>,
    search_rows: Memo<Vec<super::data::FuturesOpportunityRow>>,
    filtered_rows: Memo<Vec<super::data::FuturesOpportunityRow>>,
    eligibility_filter: RwSignal<OpportunityEligibilityFilter>,
}

pub(in crate::panels) fn futures_module(
    runtime: FuturesRuntime,
    execution_runtime: ExecutionRuntime,
    active_module: RwSignal<ModuleId>,
) -> impl IntoView {
    let filter = runtime.filter;
    let visible = RwSignal::new(default_visible_for_strategy(
        filter.get_untracked().strategy,
    ));
    let strategy_kinds = use_strategy_kinds();
    let strategy_kinds_state = strategy_kinds.state;
    let (stream_channel_state, stream_stale) = arbitrage_stream_toolbar_signals();
    let store = use_futures_opportunities(runtime);
    let stream_state = store.state;
    let rows_signal = store.rows;
    let meta_signal = store.meta;
    let page_signal = store.page;
    let loading_signal = store.loading;
    let problem_signal = use_visible_futures_problem(stream_state);
    let load_cursor = store.load_cursor;
    let search = use_symbol_futures_opportunities(runtime);
    let search_rows_signal = search.rows;
    let search_meta_signal = search.meta;
    let search_page_signal = search.page;
    let search_state = search.state;
    let search_loading = Memo::new(move |_| matches!(search_state.get(), LoadState::Loading));
    let search_load_cursor = search.load_cursor;
    let symbol_search_active = Memo::new(move |_| futures_symbol_search_active(&filter.get()));
    let rows = Memo::new(move |_| {
        if symbol_search_active.get() {
            let canonical_symbol = search_meta_signal.get().filter_symbol;
            merge_symbol_futures_rows(
                &rows_signal.get(),
                &search_rows_signal.get(),
                canonical_symbol.as_deref(),
            )
        } else {
            rows_signal.get()
        }
    });
    let effective_filter = Memo::new(move |_| {
        let mut active = filter.get();
        if symbol_search_active.get() {
            active.query.clear();
        }
        active
    });
    let filtered_rows = Memo::new(move |_| filter_rows(&rows.get(), &effective_filter.get()));
    let eligibility_filter = RwSignal::new(OpportunityEligibilityFilter::All);
    let eligibility_summary = Memo::new(move |_| {
        filtered_rows.with(|rows| {
            OpportunityEligibilitySummary::from_rows(rows.iter().map(|row| row.view.as_ref()))
        })
    });
    let visible_rows = Memo::new(move |_| {
        let eligibility = eligibility_filter.get();
        filtered_rows.with(|rows| {
            rows.iter()
                .filter(|row| eligibility.matches(row.view.as_ref()))
                .cloned()
                .collect::<Vec<_>>()
        })
    });
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
    let active_strategy = Memo::new(move |_| filter.get().strategy);
    let kpi_placeholder = Memo::new(move |_| {
        opportunity_kpi_placeholder(
            &stream_state.get(),
            &active_meta.get(),
            filtered_rows.get().len(),
        )
        .map(str::to_owned)
    });
    let table_empty_label = futures_empty_label(FuturesEmptyLabelsInput {
        filter,
        active_meta,
        search_state,
        stream_state,
        problem_signal,
        search_rows: search_rows_signal,
        filtered_rows,
        eligibility_filter,
    });
    let page_bindings = futures_page_bindings(FuturesPageBindingInput {
        filter,
        symbol_search_active,
        page_signal,
        loading_signal,
        load_cursor,
        search_page_signal,
        search_loading,
        search_load_cursor,
    });
    let on_build = Callback::new(move |opp: super::data::FuturesOpportunityRow| {
        execution_runtime.seed_selection(opp.execution_seed());
        active_module.set(ModuleId::Execution);
    });
    Effect::new(move |_| {
        visible.set(default_visible_for_strategy(filter.get().strategy));
    });
    futures_workspace_view(FuturesWorkspaceViewInput {
        position_entry_symbol: runtime.position_entry_symbol,
        filter,
        summary,
        kpi_placeholder,
        active_strategy,
        toolbar: FuturesToolbarInput {
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
        },
        eligibility_summary,
        eligibility_filter,
        table: FuturesOpportunityTableInput {
            opportunities: visible_rows,
            visible,
            page: page_bindings.page,
            page_loading: page_bindings.loading,
            empty_label: table_empty_label,
            on_page: page_bindings.on_page,
            on_build,
        },
    })
}

fn futures_empty_label(input: FuturesEmptyLabelsInput) -> Memo<String> {
    let empty_label = Memo::new(move |_| {
        let filter = input.filter.get();
        let meta = input.active_meta.get();
        let search_state = input.search_state.get();
        opportunity_empty_label(&OpportunityEmptyLabelInput {
            noun: "候选策略",
            stream_state: &input.stream_state.get(),
            meta: &meta,
            stream_problem: input.problem_signal.get().as_ref(),
            search_loading: matches!(search_state, LoadState::Loading),
            search_problem: search_state.problem(),
            search_active: futures_symbol_search_active(&filter),
            search_rows_count: input.search_rows.get().len(),
            local_filter_active: futures_local_filter_active(&filter),
        })
    });
    let table_empty_label = Memo::new(move |_| {
        let eligibility = input.eligibility_filter.get();
        if !eligibility.is_all() && input.filtered_rows.with(|rows| !rows.is_empty()) {
            return format!("本页没有“{}”候选", eligibility.label());
        }
        let current_filter = input.filter.get();
        if input.filtered_rows.with(Vec::is_empty)
            && !futures_symbol_search_active(&current_filter)
            && !futures_local_filter_active(&current_filter)
        {
            return futures_strategy_empty_label(
                current_filter.strategy,
                &input.stream_state.get(),
                input.active_meta.get().status,
                input.problem_signal.get().is_some(),
            );
        }
        empty_label.get()
    });
    table_empty_label
}

fn futures_workspace_view(input: FuturesWorkspaceViewInput) -> impl IntoView {
    view! {
        <section class="module-page futures-workstation">
            <ModuleHeader title="期货套利"/>
            {futures_position_entry_context(input.position_entry_symbol, input.filter)}
            {futures_kpis(input.summary, input.kpi_placeholder, input.active_strategy)}
            <Surface
                title="交易所数据对比"
                meta="按真实交易所行情聚合"
                class_name="full-surface futures-results-surface"
            >
                {futures_toolbar(input.toolbar)}
                {opportunity_eligibility_filter(
                    input.eligibility_summary,
                    input.eligibility_filter,
                )}
                {futures_opportunity_table(input.table)}
            </Surface>
        </section>
    }
}

fn futures_strategy_empty_label(
    strategy: StrategyFilter,
    state: &LoadState<()>,
    status: shared_types::OpportunityEnvelopeStatus,
    has_stream_problem: bool,
) -> String {
    let strategy = strategy.label();
    match state {
        LoadState::Loading => format!("{strategy}候选加载中，等待首批结果"),
        LoadState::Error(problem) | LoadState::Stale { problem, .. }
            if matches!(
                problem.code.as_str(),
                shared_types::problem::codes::OPPORTUNITY_SNAPSHOT_STALE
                    | "OPPORTUNITY_ENVELOPE_STALE"
            ) =>
        {
            format!("{strategy}快照已过期，正在等待下一轮扫描")
        }
        LoadState::Error(_) => {
            format!("{strategy}候选读取失败，可展开上方“查看原因”读取完整证据")
        }
        LoadState::Stale { .. } => format!("{strategy}候选数据降级，数据源恢复中"),
        LoadState::Ready(()) if has_stream_problem => {
            format!("{strategy}机会流降级，当前没有可展示候选")
        }
        LoadState::Ready(()) => match status {
            shared_types::OpportunityEnvelopeStatus::Warming => {
                format!("{strategy}候选预热中，等待首批结果")
            }
            shared_types::OpportunityEnvelopeStatus::Degraded => {
                format!("{strategy}数据源降级，当前没有可展示候选")
            }
            shared_types::OpportunityEnvelopeStatus::Error => {
                format!("{strategy}候选读取失败，可展开上方“查看原因”读取完整证据")
            }
            shared_types::OpportunityEnvelopeStatus::Stale => {
                format!("{strategy}快照已过期，正在等待下一轮扫描")
            }
            shared_types::OpportunityEnvelopeStatus::Fresh => {
                format!("{strategy}当前暂无候选，可切换上方策略查看其它市场结构")
            }
        },
    }
}
