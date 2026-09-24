use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{
    OnchainBatchItemSnapshot, OnchainCexComparison, OnchainComparisonConfig,
    OnchainComparisonQuality, OnchainComparisonSnapshot, OnchainSpreadAlertMode,
};

use super::super::data::OnchainData;
use super::super::draft::OnchainConfigDraft;
use super::super::format::{chain_label, percent_label, quality_label, quality_tone, usd};
use super::market_state::{display_snapshot, focus_market};

#[derive(Clone, Copy, PartialEq, Eq)]
enum MarketFilter {
    All,
    Qualified,
    Attention,
}

pub(in crate::panels::modules::onchain) fn market_sidebar(
    draft: OnchainConfigDraft,
    data: OnchainData,
    open: RwSignal<bool>,
    market_selected: Callback<()>,
) -> impl IntoView {
    let filter = RwSignal::new(MarketFilter::All);
    let query = RwSignal::new(String::new());
    let snapshot = Memo::new(move |_| data.state.with(display_snapshot));
    let items = Memo::new(move |_| snapshot.with(|snapshot| snapshot.as_ref().map(|snapshot|
        snapshot.batch.items.iter()
            .filter(|item| !same_market(&snapshot.config, &item.config))
            .filter(|item| market_matches_query(&item.config, &query.get().trim().to_ascii_lowercase()))
            .filter(|item| batch_matches_filter(item, filter.get()))
            .cloned().collect::<Vec<_>>()).unwrap_or_default()));
    let show_current = move || snapshot.with(|snapshot| snapshot.as_ref().is_some_and(|snapshot|
        market_matches_query(&snapshot.config, &query.get().trim().to_ascii_lowercase())
            && current_matches_filter(snapshot, filter.get())));
    view! {
        <aside
            class=move || if open.get() { "onchain-market-rail is-open" } else { "onchain-market-rail" }
            aria-label="链上套利市场列表"
        >
            <header class="onchain-market-rail-header">
                <div>
                    <small>"ONCHAIN / CEX"</small>
                    <strong>"套利市场"</strong>
                </div>
                <div class="onchain-market-rail-header-actions">
                    <span class="onchain-market-capacity" title="已监控市场 / 队列上限">
                        <small>"监控"</small>
                        <strong class="num">{move || queue_capacity_label(data)}</strong>
                    </span>
                    <button
                        type="button"
                        class="onchain-market-rail-close"
                        aria-label="关闭市场列表"
                        on:click=move |_| market_selected.run(())
                    >"×"</button>
                </div>
            </header>
            <div class="onchain-market-rail-controls">
                <label class="onchain-market-search">
                    <span class="sr-only">"搜索市场"</span>
                    <input
                        type="search"
                        placeholder="搜索币种、链或交易所"
                        autocomplete="off"
                        spellcheck="false"
                        bind:value=query
                    />
                </label>
                <div class="onchain-market-filter" role="tablist" aria-label="市场过滤">
                    <button
                        type="button"
                        role="tab"
                        aria-selected=move || selected(filter, MarketFilter::All)
                        on:click=move |_| filter.set(MarketFilter::All)
                    >"全部"</button>
                    <button
                        type="button"
                        role="tab"
                        aria-selected=move || selected(filter, MarketFilter::Qualified)
                        on:click=move |_| filter.set(MarketFilter::Qualified)
                    >"费后达标"</button>
                    <button
                        type="button"
                        role="tab"
                        aria-selected=move || selected(filter, MarketFilter::Attention)
                        on:click=move |_| filter.set(MarketFilter::Attention)
                    >"需处理"</button>
                </div>
            </div>
            <div class="onchain-market-rail-columns" aria-hidden="true">
                <span>"链上市场"</span><span>"CEX 市场"</span><span>"净差 / 金额"</span>
            </div>
            <div class="onchain-market-list">
                {current_market_row(snapshot, show_current, market_selected)}
                <For each=move || items.get() key=|item| item.item_id.clone() children=move |initial| {
                    let item = Memo::new(move |_| items.with(|items| items.iter()
                        .find(|item| item.item_id == initial.item_id).cloned().unwrap_or_else(|| initial.clone())));
                    batch_market_row(draft, data, item, market_selected)
                }/>
                {move || (!show_current() && items.with(Vec::is_empty)).then(|| data.state.with(|state| match state {
                    LoadState::Loading => market_message("正在读取市场", "等待链上与 CEX 目录").into_any(),
                    LoadState::Error(problem) => market_message("市场读取失败", &problem.message).into_any(),
                    LoadState::Stale { .. } => market_message("行情待恢复", "旧快照不参与实时机会判断").into_any(),
                    _ => market_message(empty_filter_title(filter.get()),
                        if query.get().trim().is_empty() { empty_filter_detail(filter.get()) } else { "调整搜索条件后继续查看" }).into_any(),
                }))}
            </div>
            <footer class="onchain-market-rail-footer">
                {move || market_footer(data)}
            </footer>
        </aside>
    }
}

fn current_market_row(
    snapshot: Memo<Option<OnchainComparisonSnapshot>>,
    visible: impl Fn() -> bool + Send + Sync + 'static,
    market_selected: Callback<()>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class=move || snapshot.with(|snapshot| format!("onchain-market-row is-active {}", snapshot.as_ref().map_or("", |snapshot| quality_tone(snapshot.quality))))
            hidden=move || !visible()
            aria-current="true"
            title="当前市场"
            on:click=move |_| market_selected.run(())
        >
            {move || snapshot.with(|snapshot| snapshot.as_ref().map(current_market_cells))}
        </button>
    }
}

fn current_market_cells(snapshot: &OnchainComparisonSnapshot) -> impl IntoView {
    let config = &snapshot.config;
    let edge = current_edge(snapshot);
    let pair = format!("{}/{}", config.base_token, config.quote_token);
    let chain = chain_label(&config.chain);
    let venue = config.cex_venue.to_uppercase();
    let symbol = config.cex_symbol.clone();
    let state = if config.enabled {
        current_market_state(snapshot)
    } else {
        "已暂停"
    };
    let detail = market_result_detail(state, current_market_amount(snapshot));
    view! {
            <span class="onchain-market-row-pair">
                <strong>{pair}</strong>
                <small><span>{chain}</span><em>"当前"</em></small>
            </span>
            <span class="onchain-market-row-venue"><strong>{venue}</strong><small>{symbol}</small></span>
            <span class="onchain-market-row-edge"><strong class="num">{edge}</strong><small>{detail}</small></span>
    }
}

fn batch_market_row(
    draft: OnchainConfigDraft,
    data: OnchainData,
    item: Memo<OnchainBatchItemSnapshot>,
    market_selected: Callback<()>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class=move || item.with(|item| format!("onchain-market-row {}", quality_tone(item.quality)))
            title=move || item.with(|item| format!("载入 {}/{} · {} {}", item.config.base_token, item.config.quote_token,
                item.config.cex_venue.to_uppercase(), item.config.cex_symbol))
            disabled=move || data.saving.get()
            on:click=move |_| {
                if let Some(item) = item.try_get_untracked() {
                    if focus_market(draft, data, &item.item_id) { market_selected.run(()); }
                }
            }
        >
            {move || item.with(batch_market_cells)}
        </button>
    }
}

fn batch_market_cells(item: &OnchainBatchItemSnapshot) -> impl IntoView {
    let config = item.config.clone();
    let pair = format!("{}/{}", config.base_token, config.quote_token);
    let chain = chain_label(&config.chain);
    let venue = config.cex_venue.to_uppercase();
    let symbol = config.cex_symbol.clone();
    let edge = batch_edge(item);
    let state = batch_market_state(item);
    let detail = market_result_detail(state, item.observable_notional_usd);
    view! {
            <span class="onchain-market-row-pair"><strong>{pair}</strong><small>{chain}</small></span>
            <span class="onchain-market-row-venue"><strong>{venue}</strong><small>{symbol}</small></span>
            <span class="onchain-market-row-edge"><strong class="num">{edge}</strong><small>{detail}</small></span>
    }
}

fn market_message(title: &str, detail: &str) -> impl IntoView {
    view! {
        <div class="onchain-market-list-message" role="status">
            <strong>{title.to_owned()}</strong>
            <span>{detail.to_owned()}</span>
        </div>
    }
}

fn market_footer(data: OnchainData) -> AnyView {
    data.state.with(|state| {
        if matches!(state, LoadState::Stale { .. }) {
            return view! { <span>"队列状态"</span><strong>"状态待确认"</strong> }.into_any();
        }
        let Some(snapshot) = display_snapshot(state) else {
            return view! { <span>"队列状态"</span><strong class="num">"--"</strong> }.into_any();
        };
        let batch_items = snapshot
            .batch
            .items
            .iter()
            .filter(|item| !same_market(&snapshot.config, &item.config))
            .collect::<Vec<_>>();
        let qualified = usize::from(current_is_qualified(&snapshot))
            + batch_items
                .iter()
                .filter(|item| batch_is_qualified(item))
                .count();
        let attention = usize::from(current_needs_attention(&snapshot))
            + batch_items
                .iter()
                .filter(|item| batch_needs_attention(item))
                .count();
        view! {
            <span>"队列状态"</span>
            <strong class="num">{format!("{qualified} 达标 · {attention} 需处理")}</strong>
        }
        .into_any()
    })
}

fn queue_capacity_label(data: OnchainData) -> String {
    data.state.with(|state| {
        state.value().map_or_else(
            || "--/--".to_owned(),
            |snapshot| {
                format!(
                    "{}/{}",
                    snapshot.batch.items.len(),
                    snapshot.batch.max_items
                )
            },
        )
    })
}

fn current_edge(snapshot: &OnchainComparisonSnapshot) -> String {
    current_best_comparison(snapshot).map_or_else(
        || "--".to_owned(),
        |row| percent_label(comparison_edge(snapshot.quality, row)),
    )
}

fn current_market_amount(snapshot: &OnchainComparisonSnapshot) -> Option<f64> {
    current_best_comparison(snapshot).map(|row| row.observable_notional_usd)
}

fn current_best_comparison(snapshot: &OnchainComparisonSnapshot) -> Option<&OnchainCexComparison> {
    snapshot
        .comparisons
        .iter()
        .filter(|row| comparison_edge(snapshot.quality, row).is_finite())
        .max_by(|left, right| {
            comparison_edge(snapshot.quality, left)
                .total_cmp(&comparison_edge(snapshot.quality, right))
        })
}

fn comparison_edge(quality: OnchainComparisonQuality, row: &OnchainCexComparison) -> f64 {
    if matches!(
        quality,
        OnchainComparisonQuality::RawCrossQuote | OnchainComparisonQuality::RawCustomPair
    ) {
        row.gross_spread_bps
    } else {
        row.net_spread_bps
    }
}

fn batch_edge(item: &OnchainBatchItemSnapshot) -> String {
    batch_sort_edge_option(item)
        .filter(|value| value.is_finite())
        .map_or_else(|| "--".to_owned(), percent_label)
}

fn batch_sort_edge_option(item: &OnchainBatchItemSnapshot) -> Option<f64> {
    let raw = item.config.spread_alert.mode == OnchainSpreadAlertMode::RawObservation
        || matches!(
            item.quality,
            OnchainComparisonQuality::RawCrossQuote | OnchainComparisonQuality::RawCustomPair
        );
    if raw {
        item.best_gross_spread_bps
    } else {
        item.best_net_spread_bps
    }
}

fn market_result_detail(state: &str, amount_usd: Option<f64>) -> String {
    amount_usd.filter(|amount| amount.is_finite()).map_or_else(
        || state.to_owned(),
        |amount| format!("{state} · {}", usd(amount)),
    )
}

fn current_market_state(snapshot: &OnchainComparisonSnapshot) -> &'static str {
    if current_is_qualified(snapshot) {
        "费后达标"
    } else {
        quality_label(snapshot.quality)
    }
}

fn batch_market_state(item: &OnchainBatchItemSnapshot) -> &'static str {
    if batch_is_qualified(item) {
        "费后达标"
    } else {
        quality_label(item.quality)
    }
}

fn current_is_qualified(snapshot: &OnchainComparisonSnapshot) -> bool {
    snapshot.comparisons.iter().any(|row| {
        verified_net_meets_threshold(
            snapshot.config.spread_alert.mode,
            snapshot.quality,
            Some(row.net_spread_bps),
            snapshot.config.spread_alert.min_net_spread_bps,
        )
    })
}

fn batch_is_qualified(item: &OnchainBatchItemSnapshot) -> bool {
    verified_net_meets_threshold(
        item.config.spread_alert.mode,
        item.quality,
        item.best_net_spread_bps,
        item.config.spread_alert.min_net_spread_bps,
    )
}

fn current_matches_filter(snapshot: &OnchainComparisonSnapshot, filter: MarketFilter) -> bool {
    match filter {
        MarketFilter::All => true,
        MarketFilter::Qualified => current_is_qualified(snapshot),
        MarketFilter::Attention => current_needs_attention(snapshot),
    }
}

fn batch_matches_filter(item: &OnchainBatchItemSnapshot, filter: MarketFilter) -> bool {
    match filter {
        MarketFilter::All => true,
        MarketFilter::Qualified => batch_is_qualified(item),
        MarketFilter::Attention => batch_needs_attention(item),
    }
}

fn current_needs_attention(snapshot: &OnchainComparisonSnapshot) -> bool {
    snapshot.provider_problem.is_some()
        || snapshot.cex_problem.is_some()
        || quality_needs_attention(snapshot.quality)
}

fn batch_needs_attention(item: &OnchainBatchItemSnapshot) -> bool {
    item.provider_problem.is_some()
        || item.cex_problem.is_some()
        || quality_needs_attention(item.quality)
}

fn quality_needs_attention(quality: OnchainComparisonQuality) -> bool {
    matches!(
        quality,
        OnchainComparisonQuality::Stale
            | OnchainComparisonQuality::LowLiquidity
            | OnchainComparisonQuality::MappingInvalid
            | OnchainComparisonQuality::UpstreamUnavailable
    )
}

const fn empty_filter_title(filter: MarketFilter) -> &'static str {
    match filter {
        MarketFilter::All => "没有匹配市场",
        MarketFilter::Qualified => "暂无费后达标市场",
        MarketFilter::Attention => "当前没有待处理市场",
    }
}

const fn empty_filter_detail(filter: MarketFilter) -> &'static str {
    match filter {
        MarketFilter::All => "监控队列当前为空",
        MarketFilter::Qualified => "当前没有达到各市场收益门槛的实时结果",
        MarketFilter::Attention => "链上 Provider 与 CEX 行情当前没有已知异常",
    }
}

fn verified_net_meets_threshold(
    mode: OnchainSpreadAlertMode,
    quality: OnchainComparisonQuality,
    edge_bps: Option<f64>,
    minimum_bps: f64,
) -> bool {
    mode == OnchainSpreadAlertMode::VerifiedNet
        && quality == OnchainComparisonQuality::Fresh
        && edge_bps
            .is_some_and(|edge| edge.is_finite() && edge > 0.0 && edge >= minimum_bps.max(0.0))
}

fn market_matches_query(config: &OnchainComparisonConfig, query: &str) -> bool {
    query.is_empty()
        || [
            config.base_token.as_str(),
            config.quote_token.as_str(),
            config.chain.as_str(),
            config.provider.as_str(),
            config.base_mint.as_str(),
            config.quote_mint.as_str(),
            config.cex_venue.as_str(),
            config.cex_symbol.as_str(),
        ]
        .into_iter()
        .any(|value| value.to_ascii_lowercase().contains(query))
}

fn same_market(left: &OnchainComparisonConfig, right: &OnchainComparisonConfig) -> bool {
    [
        (left.chain.as_str(), right.chain.as_str()),
        (left.provider.as_str(), right.provider.as_str()),
        (left.base_mint.as_str(), right.base_mint.as_str()),
        (left.quote_mint.as_str(), right.quote_mint.as_str()),
        (left.cex_venue.as_str(), right.cex_venue.as_str()),
        (left.cex_symbol.as_str(), right.cex_symbol.as_str()),
    ]
    .into_iter()
    .all(|(left, right)| left.trim().eq_ignore_ascii_case(right.trim()))
}

fn selected(filter: RwSignal<MarketFilter>, target: MarketFilter) -> &'static str {
    if filter.get() == target {
        "true"
    } else {
        "false"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_filter_matches_pair_venue_and_chain() {
        let mut config = OnchainComparisonConfig::default();
        config.provider = "jupiter_api_key".to_owned();
        config.base_mint = "ContractAddressExample".to_owned();
        assert!(market_matches_query(&config, "sol"));
        assert!(market_matches_query(&config, "binance"));
        assert!(market_matches_query(&config, "jupiter"));
        assert!(market_matches_query(&config, "contractaddress"));
        assert!(!market_matches_query(&config, "definitely-missing"));
    }

    #[test]
    fn attention_filter_only_contains_actionable_market_failures() {
        assert!(quality_needs_attention(
            OnchainComparisonQuality::MappingInvalid
        ));
        assert!(quality_needs_attention(
            OnchainComparisonQuality::UpstreamUnavailable
        ));
        assert!(!quality_needs_attention(OnchainComparisonQuality::Fresh));
        assert!(!quality_needs_attention(
            OnchainComparisonQuality::NoNetProfit
        ));
    }

    #[test]
    fn market_result_keeps_quality_and_current_size_together() {
        assert_eq!(
            market_result_detail("费后达标", Some(125.0)),
            "费后达标 · $125.00"
        );
        assert_eq!(market_result_detail("等待数据", None), "等待数据");
    }

    #[test]
    fn same_market_uses_the_full_route_identity() {
        let left = OnchainComparisonConfig::default();
        let mut right = left.clone();
        assert!(same_market(&left, &right));

        right.cex_symbol = "ETH/USDT".to_owned();
        assert!(!same_market(&left, &right));
    }

    #[test]
    fn qualified_filter_respects_the_configured_net_threshold() {
        assert!(!verified_net_meets_threshold(
            OnchainSpreadAlertMode::VerifiedNet,
            OnchainComparisonQuality::Fresh,
            Some(19.9),
            20.0,
        ));
        assert!(verified_net_meets_threshold(
            OnchainSpreadAlertMode::VerifiedNet,
            OnchainComparisonQuality::Fresh,
            Some(20.0),
            20.0,
        ));
        assert!(!verified_net_meets_threshold(
            OnchainSpreadAlertMode::RawObservation,
            OnchainComparisonQuality::Fresh,
            Some(200.0),
            20.0,
        ));
    }
}
