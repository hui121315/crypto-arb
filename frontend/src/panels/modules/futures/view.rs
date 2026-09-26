use crate::panels::modules::opportunity_counts::{
    opportunity_empty_label, opportunity_kpi_placeholder, snapshot_clock,
    OpportunityEmptyLabelInput,
};
use crate::panels::modules::opportunity_eligibility::{
    opportunity_eligibility_filter, OpportunityEligibilityFilter, OpportunityEligibilitySummary,
};
use crate::panels::modules::ExecutionRuntime;
use crate::panels::shared::{ModuleHeader, Surface};
use crate::panels::workstation::ModuleId;
use crate::state::load_state::LoadState;
use crate::state::strategy_kinds::use_strategy_kinds;
use gloo_timers::callback::Interval;
use leptos::prelude::*;
use std::collections::HashSet;

use super::columns::default_visible_for_strategy;
use super::components::{futures_opportunity_table, FuturesOpportunityTableInput};
use super::data::{
    filter_rows, merge_symbol_futures_rows, summarize_rows, use_futures_opportunities,
    use_symbol_futures_opportunities, FuturesQuoteSnapshot, FuturesRowsProjection, FuturesRuntime,
    StrategyFilter,
};

#[path = "view/sections.rs"]
mod sections;
use sections::{
    arbitrage_stream_toolbar_signals, futures_kpis, futures_local_filter_active,
    futures_page_bindings, futures_position_entry_context, futures_snapshot_usable,
    futures_symbol_search_active, futures_toolbar, use_visible_futures_problem,
    FuturesPageBindingInput, FuturesToolbarInput,
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
    let search_current = search.query_current;
    let search_loading = Memo::new(move |_| {
        !search_current.get() || matches!(search_state.get(), LoadState::Loading)
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
    let symbol_search_active = Memo::new(move |_| futures_symbol_search_active(&filter.get()));
    let projection = Memo::new(move |_| {
        if symbol_search_active.get() {
            if !search_current.get() {
                return FuturesRowsProjection::default();
            }
            merge_symbol_futures_rows(
                FuturesQuoteSnapshot {
                    rows: &rows_signal.get(),
                    meta: &meta_signal.get(),
                    page: page_signal.get().as_ref(),
                },
                FuturesQuoteSnapshot {
                    rows: &search_rows_signal.get(),
                    meta: &search_meta_signal.get(),
                    page: search_page_signal.get().as_ref(),
                },
            )
        } else {
            FuturesRowsProjection { rows: rows_signal.get(), ..Default::default() }
        }
    });
    let rows = Memo::new(move |_| projection.get().rows);
    let live_quote_usable = Memo::new(move |_| {
        !loading_signal.get()
            && !stream_stale.get()
            && !list_meta.get().preview_age_expired()
            && futures_snapshot_usable(&stream_state.get(), &list_meta.get())
    });
    let search_quote_usable = Memo::new(move |_| {
        search_current.get()
            && !search_loading.get()
            && !search_meta.get().preview_age_expired()
            && futures_snapshot_usable(&search_state.get(), &search_meta.get())
    });
    let quote_ready_ids = Memo::new(move |_| {
        let searched = symbol_search_active.get();
        let search_ready = search_quote_usable.get();
        let live_ready = live_quote_usable.get();
        projection.with(|projection| projection.rows.iter().filter(|row| {
            if searched {
                search_ready && (!projection.live_ids.contains(&row.id) || live_ready)
            } else {
                live_ready
            }
        }).map(|row| row.id.clone()).collect::<HashSet<_>>())
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
        let ready = quote_ready_ids.get();
        filtered_rows.with(|rows| {
            OpportunityEligibilitySummary::from_rows_with_readiness(rows.iter().map(|row| (row.view.as_ref(), ready.contains(&row.id))))
        })
    });
    let visible_rows = Memo::new(move |_| {
        let eligibility = eligibility_filter.get();
        let ready = quote_ready_ids.get();
        filtered_rows.with(|rows| {
            rows.iter()
                .filter(|row| eligibility.matches_with_readiness(row.view.as_ref(), ready.contains(&row.id)))
                .cloned()
                .collect::<Vec<_>>()
        })
    });
    let active_meta = Memo::new(move |_| {
        if symbol_search_active.get() && search_current.get() {
            let mut meta = search_meta.get();
            if projection.get().complete_live {
                meta.filtered_count = rows.with(Vec::len);
            }
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
        summary.filtered_candidates = filtered_rows.with(Vec::len);
        summary
    });
    let active_strategy = Memo::new(move |_| filter.get().strategy);
    let kpi_placeholder = Memo::new(move |_| {
        if active_meta.get().preview_age_expired() {
            return Some("快照已过期".to_owned());
        }
        if !rows.with(Vec::is_empty) && quote_ready_ids.with(HashSet::is_empty)
            && !active_meta.get().rows_retained && !search_loading.get()
        {
            return Some("报价待更新".to_owned());
        }
        let state = if symbol_search_active.get() {
            if search_loading.get() {
                LoadState::Loading
            } else {
                search_state.get()
            }
        } else {
            stream_state.get()
        };
        opportunity_kpi_placeholder(&state, &active_meta.get(), filtered_rows.get().len())
            .map(str::to_owned)
    });
    let table_empty_label = futures_empty_label(FuturesEmptyLabelsInput {
        filter,
        active_meta,
        search_state,
        stream_state,
        problem_signal,
        search_rows: rows,
        filtered_rows,
        eligibility_filter,
    });
    let display_search_page = Memo::new(move |_| {
        let mut page = search_page_signal.get()?;
        if projection.get().complete_live {
            page.total_rows = rows.with(Vec::len);
            page.returned_count = page.total_rows;
        }
        Some(page)
    });
    let page_bindings = futures_page_bindings(FuturesPageBindingInput {
        filter,
        symbol_search_active,
        page_signal,
        loading_signal,
        load_cursor,
        search_page_signal: display_search_page,
        search_loading,
        search_load_cursor,
    });
    let on_build = Callback::new(move |opp: super::data::FuturesOpportunityRow| {
        if !quote_ready_ids.with_untracked(|ids| ids.contains(&opp.id)) {
            return;
        }
        let current_meta = if symbol_search_active.get_untracked() {
            search_meta_signal.get_untracked()
        } else {
            meta_signal.get_untracked()
        };
        if current_meta.aged_at(snapshot_clock()).preview_age_expired() {
            return;
        }
        if symbol_search_active.get_untracked()
            && projection.with_untracked(|projection| projection.live_ids.contains(&opp.id))
            && meta_signal.get_untracked().aged_at(snapshot_clock()).preview_age_expired()
        {
            return;
        }
        let Some(current) = visible_rows
            .get_untracked()
            .into_iter()
            .find(|row| row.id == opp.id && row.execution_eligible)
        else {
            return;
        };
        execution_runtime.seed_selection(current.execution_seed());
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
            meta_signal: list_meta,
            problem_signal,
            search_loading,
            search_state,
            search_meta_signal: search_meta,
            search_source_label: Memo::new(move |_| {
                let projected = projection.get();
                let count = projected.live_ids.len();
                if count > 0 && !live_quote_usable.get() {
                    format!("WS 报价 {count} 条待更新；其他行按搜索快照核对")
                } else if projected.complete_live {
                    format!("完整 WS 窗口 · 当前匹配 {count} 条")
                } else {
                    format!("WS 报价 {count} 条 · 搜索快照 {} 条", projected.rows.len().saturating_sub(count))
                }
            }),
            search_retry: Callback::new(move |()| {
                search_load_cursor.run(runtime.search.cursor.get_untracked())
            }),
            list_is_paged: Memo::new(move |_| {
                !symbol_search_active.get()
                    && runtime.list.cursor.with(Option::is_some)
            }),
            list_loading: loading_signal,
            list_retry: Callback::new(move |()| {
                load_cursor.run(runtime.list.cursor.get_untracked())
            }),
            list_home: Callback::new(move |()| load_cursor.run(None)),
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
            quote_ready_ids,
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
                    input.table.page,
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
            format!("{strategy}候选读取失败，可展开上方“查看原因”读取完整数据依据")
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
                format!("{strategy}候选读取失败，可展开上方“查看原因”读取完整数据依据")
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
