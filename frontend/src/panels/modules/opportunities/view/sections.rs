//! 机会模块的工具栏/KPI 子视图、分页绑定与可见 problem 收敛。
//! 顶层 `OpportunitiesModule` 组装见父模块 `view.rs`。

use leptos::prelude::*;
use shared_types::{ApiProblem, OpportunityListPage, StrategyKindInfo};

use crate::api::ws::WsChannelState;
use crate::panels::modules::opportunity_counts::OpportunityCountMeta;
use crate::panels::modules::opportunity_toolbar_state::{
    arbitrage_feed_status, arbitrage_feed_summary, arbitrage_stream_recovery, list_state_message, opportunity_snapshot_usable,
    problem_message, stream_channel_message, stream_problem_message, ArbitrageFeedStatus,
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
    pub(super) meta_signal: Memo<OpportunityCountMeta>,
    pub(super) problem_signal: Memo<Option<ApiProblem>>,
    pub(super) search_loading: Memo<bool>,
    pub(super) search_state: RwSignal<LoadState<()>>,
    pub(super) search_meta_signal: Memo<OpportunityCountMeta>,
    pub(super) search_source_label: Memo<String>,
    pub(super) search_retry: Callback<()>,
    pub(super) list_is_paged: Memo<bool>,
    pub(super) list_loading: RwSignal<bool>,
    pub(super) list_retry: Callback<()>,
    pub(super) list_home: Callback<()>,
}

pub(super) fn arbitrage_stream_toolbar_signals() -> (RwSignal<WsChannelState>, RwSignal<bool>) {
    let stream = use_global().arbitrage_stream;
    (stream.ws_channel_state, stream.stream_stale)
}

pub(super) fn opportunity_toolbar(input: OpportunityToolbarInput) -> impl IntoView {
    let feed_status = Memo::new(move |_| {
        if input.list_is_paged.get() && input.list_loading.get() {
            return ArbitrageFeedStatus {
                label: "分页读取中",
                tone: "is-warming",
            };
        }
        if input.meta_signal.get().preview_age_expired() {
            return ArbitrageFeedStatus {
                label: "本页候选快照过期",
                tone: "is-degraded",
            };
        }
        if input.list_is_paged.get()
            && !input.list_loading.get()
            && opportunity_snapshot_usable(&input.list_state.get(), &input.meta_signal.get())
        {
            return ArbitrageFeedStatus {
                label: "本页候选快照",
                tone: "is-warming",
            };
        }
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
                    input.meta_signal.into(),
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
            {arbitrage_stream_recovery()}
            <Show when=move || super::support::opportunity_symbol_search_active(&input.filter.get())>
                <div class="futures-search-status" role="status" class:is-error=move || input.search_state.get().problem().is_some() || input.search_meta_signal.get().preview_age_expired()>
                    <div class="futures-search-summary">
                    <span>{move || {
                        let query = input.filter.get().query.trim().to_ascii_uppercase();
                        if input.search_loading.get() {
                            format!("{query} · 搜索中")
                        } else if input.search_state.get().problem().is_some()
                            && !opportunity_snapshot_usable(&input.search_state.get(), &input.search_meta_signal.get()) {
                            format!("{query} · 搜索失败，暂不可构建；详情查看原因")
                        } else if input.search_meta_signal.get().preview_age_expired() {
                            format!("{query} · 搜索快照已过期，暂不可构建")
                        } else if input.search_state.get().problem().is_some() {
                            format!("{query} · 部分数据缺失，保留已核对候选")
                        } else {
                            format!("{query} · 搜索快照 · {}", crate::panels::modules::opportunity_toolbar_state::compact_snapshot_age_label(&input.search_meta_signal.get()))
                        }
                    }}</span>
                    <small>{move || input.search_source_label.get()}</small>
                    </div>
                    <Show when=move || input.search_state.get().problem().is_some() || input.search_meta_signal.get().preview_age_expired()>
                        <button type="button" disabled=move || input.search_loading.get()
                            on:click=move |_| input.search_retry.run(())>"重新搜索"</button>
                    </Show>
                </div>
            </Show>
            <Show when=move || input.list_is_paged.get()>
                <div class="futures-search-status opportunity-page-status" role="status"
                    class:is-error=move || input.meta_signal.get().preview_age_expired() || input.list_state.get().problem().is_some()>
                    <span>{move || if input.list_loading.get() {
                        "本页快照刷新中".to_owned()
                    } else if input.meta_signal.get().preview_age_expired() {
                        "本页快照已过期，保留报价供查看；暂不可构建".to_owned()
                    } else if input.list_state.get().problem().is_some() {
                        "分页读取异常，保留上一份报价；暂不可构建".to_owned()
                    } else {
                        "分页快照 · 首页 WS 不更新本页".to_owned()
                    }}</span>
                    <button type="button" disabled=move || input.list_loading.get()
                        on:click=move |_| input.list_retry.run(())>"刷新当前页"</button>
                    <button type="button"
                        on:click=move |_| input.list_home.run(())>"返回实时首页"</button>
                </div>
            </Show>
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
    pub(super) search_page_signal: Memo<Option<OpportunityListPage>>,
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
            if input
                .page_signal
                .get_untracked()
                .is_some_and(|page| page.start_offset > 0)
            {
                input.load_cursor.run(None);
            }
            if input
                .search_page_signal
                .get_untracked()
                .is_some_and(|page| page.start_offset > 0)
            {
                input.search_load_cursor.run(None);
            }
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
                <strong class="num">{move || kpi_value_or(kpi_placeholder.get(), summary.get().candidates.to_string())}</strong>
                <em>{move || kpi_or(kpi_placeholder.get(), format!("当前筛选 {}", summary.get().filtered_candidates))}</em>
            </div>
            <div
                class="scan-kpi"
                data-tone=move || {
                    if kpi_placeholder.get().is_some() { "neutral" }
                    else if summary.get().executable_candidates > 0 { "ready" } else { "waiting" }
                }
            >
                <span>"可检查交易"</span>
                <strong class="num">{move || kpi_value_or(kpi_placeholder.get(), summary.get().executable_candidates.to_string())}</strong>
                <em>{move || kpi_or(
                    kpi_placeholder.get(),
                    if summary.get().executable_candidates > 0 {
                        "当前页 · 构建后重验".to_owned()
                    } else {
                        "当前页全部仅观察".to_owned()
                    },
                )}</em>
            </div>
            <div
                class="scan-kpi"
                data-tone=move || {
                    if kpi_placeholder.get().is_none() && summary.get().best_executable_net_bps.is_some() { "ready" } else { "neutral" }
                }
            >
                <span>"最佳费后净利"</span>
                <strong class="num">{move || {
                    let value = summary.get().best_executable_net_bps
                        .map_or_else(|| "—".to_owned(), |value| format!("{:+.3}%", value / 100.0));
                    kpi_value_or(kpi_placeholder.get(), value)
                }}</strong>
                <em>{move || {
                    let summary = summary.get();
                    let value = summary.best_executable_net_bps.map_or_else(
                        || "没有合格收益数据依据".to_owned(),
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

fn kpi_value_or(placeholder: Option<String>, value: String) -> String {
    placeholder.map_or(value, |_| "—".to_owned())
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
