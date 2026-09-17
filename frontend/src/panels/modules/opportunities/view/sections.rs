//! 机会模块的工具栏/KPI 子视图、分页绑定与可见 problem 收敛。
//! 顶层 `OpportunitiesModule` 组装见父模块 `view.rs`。

use leptos::prelude::*;
use shared_types::{ApiProblem, OpportunityListPage, StrategyKindInfo};

use crate::api::ws::WsChannelState;
use crate::panels::modules::opportunity_counts::OpportunityCountMeta;
use crate::panels::modules::opportunity_toolbar_state::{
    arbitrage_feed_status, arbitrage_feed_summary, list_state_message, problem_message,
    stream_channel_message, stream_problem_message,
};
use crate::state::context::use_global;
use crate::state::load_state::LoadState;

use super::super::components::{opportunity_filter_bar, strategy_chips};
use super::super::data::{OpportunityFilter, OpportunitySummary};
use super::support::opportunity_table_reset_key;

#[derive(Clone, Copy)]
pub(super) struct OpportunityToolbarInput {
    pub(super) filter: RwSignal<OpportunityFilter>,
    pub(super) strategy_kinds_state: RwSignal<LoadState<Vec<StrategyKindInfo>>>,
    pub(super) stream_channel_state: RwSignal<WsChannelState>,
    pub(super) stream_stale: RwSignal<bool>,
    pub(super) list_state: RwSignal<LoadState<()>>,
    pub(super) meta_signal: RwSignal<OpportunityCountMeta>,
    pub(super) problem_signal: Memo<Option<ApiProblem>>,
    pub(super) search_loading: Memo<bool>,
    pub(super) search_state: RwSignal<LoadState<()>>,
    pub(super) search_meta_signal: RwSignal<OpportunityCountMeta>,
}

pub(super) fn arbitrage_stream_toolbar_signals() -> (RwSignal<WsChannelState>, RwSignal<bool>) {
    let stream = use_global().arbitrage_stream;
    (stream.ws_channel_state, stream.stream_stale)
}

pub(super) fn opportunity_toolbar(input: OpportunityToolbarInput) -> impl IntoView {
    let feed_status = Memo::new(move |_| {
        arbitrage_feed_status(
            &input.list_state.get(),
            &input.stream_channel_state.get(),
            input.stream_stale.get(),
            input.problem_signal.get().is_some(),
        )
    });
    view! {
        <div class="module-toolbar opportunity-toolbar">
            <div class="opportunity-toolbar-controls">
                {strategy_chips(input.filter, input.strategy_kinds_state)}
                {opportunity_filter_bar(input.filter)}
            </div>
            <details class="futures-feed-status">
                {arbitrage_feed_summary(
                    feed_status,
                    input.stream_channel_state,
                    input.stream_stale,
                    input.meta_signal,
                )}
                <div class="futures-diagnostics" aria-live="polite">
                    <em class="settings-message">{move || input.meta_signal.get().freshness_label()}</em>
                    {move || stream_channel_message(&input.stream_channel_state.get())}
                    {move || list_state_message(&input.list_state.get())}
                    {move || {
                        let list_state = input.list_state.get();
                        stream_problem_message(
                            &input.stream_channel_state.get(),
                            input.problem_signal.get(),
                            list_state.problem(),
                        )
                    }}
                    <em class="settings-message">
                        {move || if input.search_loading.get() {
                            "品种搜索中".to_owned()
                        } else {
                            input.search_meta_signal.get().instrument_coverage_diagnostics
                        }}
                    </em>
                    {move || problem_message("品种搜索失败", input.search_state.get().problem().cloned())}
                </div>
            </details>
        </div>
    }
}

fn visible_opportunity_problem(
    stream_problem: Option<ApiProblem>,
    list_problem: Option<ApiProblem>,
) -> Option<ApiProblem> {
    stream_problem.or(list_problem)
}

pub(super) fn use_visible_opportunity_problem(
    list_state_signal: RwSignal<LoadState<()>>,
) -> Memo<Option<ApiProblem>> {
    let stream_state_signal = use_global().arbitrage_stream.state;
    Memo::new(move |_| {
        visible_opportunity_problem(
            stream_state_signal.get().problem().cloned(),
            list_state_signal.get().problem().cloned(),
        )
    })
}

#[derive(Clone, Copy)]
pub(super) struct OpportunityPageBindingInput {
    pub(super) filter: RwSignal<OpportunityFilter>,
    pub(super) symbol_search_active: Memo<bool>,
    pub(super) page_signal: RwSignal<Option<OpportunityListPage>>,
    pub(super) loading_signal: RwSignal<bool>,
    pub(super) load_cursor: Callback<Option<String>>,
    pub(super) search_page_signal: RwSignal<Option<OpportunityListPage>>,
    pub(super) search_loading: Memo<bool>,
    pub(super) search_load_cursor: Callback<Option<String>>,
}

#[derive(Clone, Copy)]
pub(super) struct OpportunityPageBindings {
    pub(super) page: Memo<Option<OpportunityListPage>>,
    pub(super) loading: Memo<bool>,
    pub(super) on_page: Callback<Option<String>>,
}

pub(super) fn opportunity_page_bindings(
    input: OpportunityPageBindingInput,
) -> OpportunityPageBindings {
    let reset_key = Memo::new(move |_| opportunity_table_reset_key(&input.filter.get()));
    let page = Memo::new(move |_| {
        if input.symbol_search_active.get() {
            input.search_page_signal.get()
        } else {
            input.page_signal.get()
        }
    });
    let loading = Memo::new(move |_| {
        if input.symbol_search_active.get() {
            input.search_loading.get()
        } else {
            input.loading_signal.get()
        }
    });
    Effect::new(move |last: Option<String>| {
        let key = reset_key.get();
        if last.as_ref().is_some_and(|prev| prev != &key) {
            input.load_cursor.run(None);
            input.search_load_cursor.run(None);
        }
        key
    });
    let on_page = Callback::new(move |cursor| {
        if input.symbol_search_active.get_untracked() {
            input.search_load_cursor.run(cursor);
        } else {
            input.load_cursor.run(cursor);
        }
    });
    OpportunityPageBindings {
        page,
        loading,
        on_page,
    }
}

/// 机会页专属的紧凑 stat 条：三组"数字 + 副文案"横排。
/// 紧凑展示候选、可预检数量与当前页最高费后净收益下限。
pub(super) fn opportunities_kpis(
    summary: Memo<OpportunitySummary>,
    kpi_placeholder: Memo<Option<String>>,
) -> impl IntoView {
    view! {
        <div class="scan-kpis">
            <div class="scan-kpi">
                <span>"候选"</span>
                <strong class="num">{move || kpi_or(kpi_placeholder.get(), summary.get().candidates.to_string())}</strong>
                <em>{move || kpi_or(kpi_placeholder.get(), format!("当前筛选 {}", summary.get().filtered_candidates))}</em>
            </div>
            <div
                class="scan-kpi"
                data-tone=move || {
                    if summary.get().executable_candidates > 0 { "ready" } else { "waiting" }
                }
            >
                <span>"可预检"</span>
                <strong class="num">{move || kpi_or(kpi_placeholder.get(), summary.get().executable_candidates.to_string())}</strong>
                <em>{move || kpi_or(
                    kpi_placeholder.get(),
                    if summary.get().executable_candidates > 0 {
                        "构建后仍会重验".to_owned()
                    } else {
                        "当前全部仅观察".to_owned()
                    },
                )}</em>
            </div>
            <div
                class="scan-kpi"
                data-tone=move || {
                    if summary.get().best_executable_net_bps.is_some() { "ready" } else { "neutral" }
                }
            >
                <span>"最佳费后净利"</span>
                <strong class="num">{move || {
                    let value = summary.get().best_executable_net_bps
                        .map_or_else(|| "—".to_owned(), |value| format!("{:+.3}%", value / 100.0));
                    kpi_or(kpi_placeholder.get(), value)
                }}</strong>
                <em>{move || {
                    let summary = summary.get();
                    let value = summary.best_executable_net_bps.map_or_else(
                        || "没有合格收益证据".to_owned(),
                        |_| summary.best_executable_pair,
                    );
                    kpi_or(kpi_placeholder.get(), value)
                }}</em>
            </div>
        </div>
    }
}

fn kpi_or(placeholder: Option<String>, value: String) -> String {
    placeholder.unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_problem_prefers_stream_problem() {
        let stream = ApiProblem::new("WS_PAYLOAD_DECODE", "ws failed").with_source("frontend-ws");
        let list = ApiProblem::new("UPSTREAM_HTTP", "list failed").with_source("rest");

        let visible = visible_opportunity_problem(Some(stream), Some(list));

        assert_eq!(
            visible.as_ref().map(|problem| problem.code.as_str()),
            Some("WS_PAYLOAD_DECODE")
        );
    }

    #[test]
    fn visible_problem_uses_list_problem_without_stream_problem() {
        let list = ApiProblem::new("UPSTREAM_HTTP", "list failed").with_source("rest");

        let visible = visible_opportunity_problem(None, Some(list));

        assert_eq!(
            visible.as_ref().map(|problem| problem.code.as_str()),
            Some("UPSTREAM_HTTP")
        );
    }
}
