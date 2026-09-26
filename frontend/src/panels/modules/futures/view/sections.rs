//! 期货模块的工具栏/KPI 子视图、分页绑定与可见 problem 收敛。
//! 顶层 `FuturesModule` 组装见父模块 `view.rs`。

use leptos::prelude::*;
use shared_types::{ApiProblem, OpportunityListPage, StrategyKindInfo};

use crate::api::ws::WsChannelState;
use crate::panels::modules::instrument_search::symbol_search_query;
use crate::panels::modules::opportunity_counts::OpportunityCountMeta;
use crate::panels::modules::opportunity_toolbar_state::{
    arbitrage_feed_status as futures_feed_status, arbitrage_feed_summary, arbitrage_stream_recovery, list_state_message,
    problem_message, stream_channel_message, stream_problem_message, ArbitrageFeedStatus,
    compact_snapshot_age_label,
};
use crate::state::context::use_global;
use crate::state::load_state::LoadState;

#[cfg(test)]
use crate::api::ws::WsStatus;
#[cfg(test)]
use crate::panels::modules::opportunity_toolbar_state::ArbitrageFeedStatus as FuturesFeedStatus;

use super::super::components::{futures_filter_bar, strategy_chips};
use super::super::data::{FuturesFilter, StrategyFilter};

#[path = "sections/kpis.rs"]
mod kpis;

pub(super) use kpis::futures_kpis;

#[derive(Clone, Copy)]
pub(super) struct FuturesToolbarInput {
    pub(super) filter: RwSignal<FuturesFilter>,
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

pub(super) fn futures_position_entry_context(
    source_symbol: RwSignal<Option<String>>,
    filter: RwSignal<FuturesFilter>,
) -> impl IntoView {
    view! {
        <Show when=move || source_symbol.with(Option::is_some)>
            <div class="futures-position-entry-context" role="note">
                <div>
                    <span>{move || source_symbol.with(|symbol| {
                        format!("来自 {} 未配对仓位", symbol.as_deref().unwrap_or("未知品种"))
                    })}</span>
                    <strong>{move || {
                        let query = filter.get().query;
                        let query = query.trim();
                        format!("{} · 独立双腿筛选", if query.is_empty() { "全部品种" } else { query })
                    }}</strong>
                </div>
                <p>"构建会新开完整双腿，不会补齐或接管来源仓位。"</p>
                <a href="#positions">"返回持仓"</a>
            </div>
        </Show>
    }
}

pub(super) fn futures_toolbar(input: FuturesToolbarInput) -> impl IntoView {
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
            && futures_snapshot_usable(&input.list_state.get(), &input.meta_signal.get())
        {
            return ArbitrageFeedStatus {
                label: "本页候选快照",
                tone: "is-warming",
            };
        }
        futures_feed_status(
            &input.list_state.get(),
            &input.stream_channel_state.get(),
            input.stream_stale.get(),
            input.problem_signal.get().is_some(),
        )
    });
    view! {
        <div class="futures-toolbar">
            <div class="futures-toolbar-primary">
                {strategy_chips(input.filter, input.strategy_kinds_state)}
                {futures_filter_bar(input.filter)}
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
                    {move || input.search_loading.get().then(|| view! { <em class="settings-message">"品种搜索中"</em> })}
                    {move || problem_message("品种搜索失败", input.search_state.get().problem().cloned())}
                    {move || {
                        let filter = input.filter.get();
                        let meta = input.search_meta_signal.get();
                        futures_symbol_search_active(&filter)
                            .then(|| view! { <em class="settings-message">{format!("搜索快照 · {}", meta.freshness_label())}</em> })
                    }}
                </div>
            </details>
            {arbitrage_stream_recovery()}
            <Show when=move || futures_symbol_search_active(&input.filter.get())>
                <div class="futures-search-status" role="status" class:is-error=move || input.search_state.get().problem().is_some() || input.search_meta_signal.get().preview_age_expired()>
                    <div class="futures-search-summary">
                    <span>{move || {
                        let query = input.filter.get().query.trim().to_ascii_uppercase();
                        if input.search_loading.get() {
                            format!("{query} · 搜索中")
                        } else if input.search_state.get().problem().is_some()
                            && !futures_snapshot_usable(&input.search_state.get(), &input.search_meta_signal.get()) {
                            format!("{query} · 搜索失败，暂不可构建；查看原因了解详情")
                        } else if input.search_meta_signal.get().preview_age_expired() {
                            format!("{query} · 搜索快照已过期，暂不可构建")
                        } else if input.search_state.get().problem().is_some() {
                            format!("{query} · 部分数据缺失，保留已核对候选")
                        } else {
                            format!("{query} · 搜索快照 · {}", compact_snapshot_age_label(&input.search_meta_signal.get()))
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
                <div class="futures-search-status futures-page-status" role="status"
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

pub(super) use crate::panels::modules::opportunity_toolbar_state::opportunity_snapshot_usable as futures_snapshot_usable;

fn visible_futures_problem(
    stream_problem: Option<ApiProblem>,
    list_problem: Option<ApiProblem>,
) -> Option<ApiProblem> {
    stream_problem.or(list_problem)
}

pub(super) fn use_visible_futures_problem(
    list_state_signal: RwSignal<LoadState<()>>,
) -> Memo<Option<ApiProblem>> {
    let stream_state_signal = use_global().arbitrage_stream.state;
    Memo::new(move |_| {
        visible_futures_problem(
            stream_state_signal.get().problem().cloned(),
            list_state_signal.get().problem().cloned(),
        )
    })
}

#[derive(Clone, Copy)]
pub(super) struct FuturesPageBindingInput {
    pub(super) filter: RwSignal<FuturesFilter>,
    pub(super) symbol_search_active: Memo<bool>,
    pub(super) page_signal: RwSignal<Option<OpportunityListPage>>,
    pub(super) loading_signal: RwSignal<bool>,
    pub(super) load_cursor: Callback<Option<String>>,
    pub(super) search_page_signal: Memo<Option<OpportunityListPage>>,
    pub(super) search_loading: Memo<bool>,
    pub(super) search_load_cursor: Callback<Option<String>>,
}

#[derive(Clone, Copy)]
pub(super) struct FuturesPageBindings {
    pub(super) page: Memo<Option<OpportunityListPage>>,
    pub(super) loading: Memo<bool>,
    pub(super) on_page: Callback<Option<String>>,
}

pub(super) fn futures_page_bindings(input: FuturesPageBindingInput) -> FuturesPageBindings {
    let reset_key = Memo::new(move |_| futures_table_reset_key(&input.filter.get()));
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
    FuturesPageBindings {
        page,
        loading,
        on_page,
    }
}

fn futures_table_reset_key(filter: &FuturesFilter) -> String {
    format!(
        "{}|{:.4}|{}",
        strategy_filter_key(filter.strategy),
        filter.min_net_pct,
        filter.query.trim().to_ascii_lowercase()
    )
}

pub(super) fn futures_local_filter_active(filter: &FuturesFilter) -> bool {
    !filter.query.trim().is_empty() || filter.min_net_pct > 0.0
}

pub(super) fn futures_symbol_search_active(filter: &FuturesFilter) -> bool {
    symbol_search_query(&filter.query).is_some()
}

fn strategy_filter_key(filter: StrategyFilter) -> &'static str {
    match filter {
        StrategyFilter::PerpCross => "perp_cross",
        StrategyFilter::PerpPriceSpread => "perp_price_spread",
        StrategyFilter::SpotPerp => "spot_perp",
        StrategyFilter::CrossSpotPerp => "cross_spot_perp",
        StrategyFilter::SpotCross => "spot_cross",
    }
}

#[cfg(test)]
#[path = "sections_tests.rs"]
mod tests;
