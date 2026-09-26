use super::data::{use_gate_crossex_data, GateCrossExData, GateCrossExRuntime};
use crate::panels::shared::operation_journal::settings_recovery_panel;
use crate::panels::shared::ModuleHeader;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{
    GateCrossExMode, GateCrossExModeSnapshot, GateCrossExProduct, GateCrossExRouteCatalogRow,
    GateCrossExRuntimeState, InstrumentListingStatus, GATE_CROSSEX_SELECTED_ROUTE_LIMIT,
};

pub(in crate::panels) fn gate_crossex_module(runtime: GateCrossExRuntime) -> impl IntoView {
    let data = use_gate_crossex_data(runtime);
    let controls = NodeRef::<leptos::html::Aside>::new();
    let quotes = NodeRef::<leptos::html::Div>::new();
    view! {
        <section class="module-page gate-crossex-page">
            <div class="gate-crossex-heading">
                <ModuleHeader title="CrossEx"/>
                <button type="button" class="btn-secondary" aria-controls="crossex-controls"
                    on:click=move |_| if let Some(element) = controls.get() {
                        element.scroll_into_view_with_bool(true);
                        let _ = element.focus();
                    }>"管理路由"</button>
            </div>
            {runtime_strip(data)}
            {settings_recovery_panel(data.journal, data.recheck)}
            <div class="gate-crossex-workbench">
                <div class="gate-crossex-main" id="crossex-quotes" node_ref=quotes tabindex="-1">
                    {candidate_table(data)}
                    {route_table(data)}
                </div>
                {control_rail(data, controls, quotes)}
            </div>
        </section>
    }
}

fn controls_disabled(data: GateCrossExData) -> bool {
    data.journal.locked() || !matches!(data.status.get(), LoadState::Ready(_))
}

fn selected_routes(data: GateCrossExData) -> Vec<String> {
    data.status
        .get()
        .value()
        .map(|row| row.config.selected_routes.clone())
        .unwrap_or_default()
}

fn control_rail(
    data: GateCrossExData,
    controls: NodeRef<leptos::html::Aside>,
    quotes: NodeRef<leptos::html::Div>,
) -> impl IntoView {
    view! {
        <aside class="gate-crossex-control" id="crossex-controls" node_ref=controls tabindex="-1" aria-label="Gate CrossEx 模式与路由">
            <header class="workbench-rail-header">
                <strong>"监控设置"</strong><span class="read-only-flag">"仅观察"</span>
            </header>
            <div class="gate-crossex-segmented" role="group" aria-label="CrossEx 模式">
                {[ (GateCrossExMode::Disabled, "关闭"), (GateCrossExMode::Monitor, "监控") ].into_iter().map(|(mode, label)| view! {
                    <button type="button"
                        class:is-active=move || data.status.get().value().is_some_and(|row| row.config.mode == mode)
                        aria-pressed=move || data.status.get().value().is_some_and(|row| row.config.mode == mode).to_string()
                        disabled=move || controls_disabled(data)
                        on:click=move |_| data.set_mode.run(mode)>{label}</button>
                }).collect_view()}
            </div>
            <form class="gate-crossex-threshold" on:submit=move |event| { event.prevent_default(); data.save_minimum.run(()); }>
                <label for="crossex-minimum">"最小毛价差 (%)"</label>
                <div>
                    <input id="crossex-minimum" type="text" inputmode="decimal"
                        prop:value=move || data.minimum.get()
                        disabled=move || controls_disabled(data)
                        on:input=move |event| { data.minimum_dirty.set(true); data.minimum.set(event_target_value(&event)); }/>
                    <button type="submit" class="btn-secondary"
                        disabled=move || controls_disabled(data) || !data.minimum_dirty.get()>"应用"</button>
                </div>
            </form>
            <output class="gate-crossex-notice" aria-live="polite">{move || data.notice.get().unwrap_or_default()}</output>
            {route_catalog(data)}
            <button type="button" class="btn-secondary gate-crossex-return" aria-controls="crossex-quotes"
                on:click=move |_| if let Some(element) = quotes.get() {
                    element.scroll_into_view_with_bool(true);
                    let _ = element.focus();
                }>"返回行情"</button>
        </aside>
    }
}

fn route_catalog(data: GateCrossExData) -> impl IntoView {
    let selected_only = RwSignal::new(false);
    let selected_matches = Memo::new(move |_| {
        let query = data.search.get().trim().to_ascii_uppercase();
        selected_routes(data)
            .into_iter()
            .filter(|native| native.contains(&query))
            .collect::<Vec<_>>()
    });
    let rows = Memo::new(move |_| {
        data.catalog
            .get()
            .value()
            .map(|row| row.routes.clone())
            .unwrap_or_default()
    });
    view! {
        <div class="gate-crossex-catalog">
            <label class="workbench-field">
                <span>"路由筛选"</span>
                <input type="search" placeholder="BTC / Kraken / USDT"
                    prop:value=move || data.search.get()
                    on:input=move |event| data.search.set(event_target_value(&event))/>
            </label>
            <div class="gate-crossex-route-count">
                <span>{move || format!("已选 {} / {}", selected_routes(data).len(), GATE_CROSSEX_SELECTED_ROUTE_LIMIT)}</span>
                <label><input type="checkbox" prop:checked=move || selected_only.get()
                    on:change=move |event| selected_only.set(event_target_checked(&event))/><span>"只看已选"</span></label>
            </div>
            <div class="gate-crossex-route-picker">
                <For each=move || if selected_only.get() { Vec::new() } else { rows.get() }
                    key=|row| (row.native_symbol.clone(), row.listing_status == InstrumentListingStatus::Trading)
                    children=move |row| route_option(data, row)/>
                <For each=move || if selected_only.get() { selected_matches.get() } else { Vec::new() }
                    key=|native| native.clone() children=move |native| selected_option(data, native)/>
                {move || if selected_only.get() {
                    selected_matches.get().is_empty().then(|| "当前筛选下没有已选路由".to_owned())
                } else { match data.catalog.get() {
                    LoadState::Loading => Some("正在读取路由…".to_owned()),
                    LoadState::Error(problem) | LoadState::Stale { problem, .. } => Some(format!("路由读取失败：{}", problem.message)),
                    LoadState::Ready(row) if row.routes.is_empty() => Some("没有匹配的官方路由".to_owned()),
                    _ => None,
                }}.map(|label| view! { <p class="gate-crossex-route-state">{label}</p> })}
            </div>
            <div class="gate-crossex-catalog-footer">
                <span>{move || if selected_only.get() {
                    format!("显示 {} / 已选 {}", selected_matches.get().len(), selected_routes(data).len())
                } else { data.catalog.get().value().map(|row| format!("显示 {} / {}", row.routes.len(), row.total)).unwrap_or_default() }}</span>
                <button type="button" class="btn-secondary" disabled=move || matches!(data.catalog.get(), LoadState::Loading)
                    on:click=move |_| data.refresh_catalog.run(())>"重读路由"</button>
            </div>
        </div>
    }
}

fn selected_option(data: GateCrossExData, native: String) -> impl IntoView {
    let native = StoredValue::new(native);
    view! {
        <label class="gate-crossex-route-option" title=move || native.get_value()>
            <input type="checkbox" aria-label=move || native.get_value()
                prop:checked=move || { let _ = data.saving.get(); selected_routes(data).contains(&native.get_value()) }
                disabled=move || controls_disabled(data)
                on:change=move |_| data.remove_route.run(native.get_value())/>
            <span><strong>{move || native.get_value()}</strong><small>"已保存路由"</small></span>
        </label>
    }
}

fn route_option(data: GateCrossExData, row: GateCrossExRouteCatalogRow) -> impl IntoView {
    let native = StoredValue::new(row.native_symbol);
    let trading = row.listing_status == InstrumentListingStatus::Trading;
    let checked = Memo::new(move |_| selected_routes(data).contains(&native.get_value()));
    view! {
        <label class="gate-crossex-route-option" title=move || native.get_value()>
            <input type="checkbox" aria-label=move || native.get_value() prop:checked=move || { let _ = data.saving.get(); checked.get() }
                disabled=move || controls_disabled(data) || (!checked.get() && (!trading || selected_routes(data).len() >= GATE_CROSSEX_SELECTED_ROUTE_LIMIT))
                on:change=move |_| data.toggle_route.run(native.get_value())/>
            <span><strong>{format!("{}/{}", row.base_asset, row.quote_asset)}</strong>
                <small>{format!("{} · {}", row.underlying_venue.to_ascii_uppercase(), product_label(row.product))}</small></span>
            {(!trading).then(|| view! { <small>"已停用"</small> })}
        </label>
    }
}

fn runtime_strip(data: GateCrossExData) -> impl IntoView {
    view! {
        <section class="gate-crossex-runtime" aria-label="CrossEx 运行状态">
            <div><span>"行情状态"</span>
                <strong class=move || if visible_problem(data).is_some() { "is-danger" } else { "" }>
                    {move || data.market.get().label().unwrap_or_else(|| status_label(&data.status.get()))}
                </strong>
            </div>
            <div><span>"官方路由"</span><strong class="num">{move || data.status.get().value().map(|row| row.catalog_count.to_string()).unwrap_or_else(|| "--".to_owned())}</strong></div>
            <div><span>"WS 报价 / 已选"</span><strong class="num">{move || data.status.get().value().map(|row| format!("{} / {}", data.market.get().live_count, row.selected_count)).unwrap_or_else(|| "--".to_owned())}</strong></div>
            <div><span>"毛价差候选"</span><strong class="num">{move || data.status.get().value().map(|_| data.market.get().candidate_count.to_string()).unwrap_or_else(|| "--".to_owned())}</strong></div>
            <button type="button" class="btn-secondary" disabled=move || data.reading.get() || data.saving.get()
                on:click=move |_| data.refresh.run(())>{move || if data.reading.get() { "读取中" } else { "刷新状态" }}</button>
            {move || visible_problem(data)
                .map(|problem| view! { <p class="gate-crossex-problem" role="status">{problem.message}</p> })}
        </section>
    }
}

fn visible_problem(data: GateCrossExData) -> Option<shared_types::ApiProblem> {
    let state = data.status.get();
    state
        .problem()
        .cloned()
        .or_else(|| data.market.get().problem)
        .or_else(|| state.value().and_then(|row| row.problem.clone()))
}

fn candidate_table(data: GateCrossExData) -> impl IntoView {
    let rows = Memo::new(move |_| {
        data.status
            .get()
            .value()
            .map(|row| row.candidates.clone())
            .unwrap_or_default()
    });
    view! {
        <section class="gate-crossex-panel">
            <header><strong>"跨路由毛价差"</strong><span>"未扣费用 · 仅观察"</span></header>
            <div class="workbench-table-wrap">
                <table class="workbench-table clean-table gate-crossex-candidate-table" data-table-budget="bounded-small">
                    <thead><tr><th>"市场"</th><th class="num">"买入 / Ask"</th><th class="num">"卖出 / Bid"</th><th class="num">"毛价差"</th><th class="num gate-crossex-time">"双腿时效"</th></tr></thead>
                    <tbody>
                        <For each=move || rows.get() key=|row| (row.long_route.clone(), row.short_route.clone())
                            children=move |initial| {
                                let row = Memo::new(move |_| rows.get().into_iter().find(|row| row.long_route == initial.long_route && row.short_route == initial.short_route).unwrap_or_else(|| initial.clone()));
                                view! { <tr>
                                    <td class="gate-crossex-market"><strong>{move || format!("{}/{}", row.get().base_asset, row.get().quote_asset)}</strong><small>{move || product_label(row.get().product)}</small></td>
                                    <td class="num gate-crossex-buy" data-label="买入 / Ask"><strong>{move || price(row.get().long_ask)}</strong><small title=move || row.get().long_route>{move || route_venue(&row.get().long_route)}</small></td>
                                    <td class="num gate-crossex-sell" data-label="卖出 / Bid"><strong>{move || price(row.get().short_bid)}</strong><small title=move || row.get().short_route>{move || route_venue(&row.get().short_route)}</small></td>
                                    <td class="num gate-crossex-spread" data-label="毛价差" class:is-positive=move || data.market.get().current(row.get().synchronized_at_ms)><strong>{move || format!("+{:.3}%", row.get().gross_spread_pct)}</strong><small>{move || if data.market.get().current(row.get().synchronized_at_ms) { "待净收益核对" } else { "上次快照" }}</small></td>
                                    <td class="num gate-crossex-time" data-label="双腿时效">{move || quote_age(data, row.get().synchronized_at_ms)}</td>
                                </tr> }
                            }/>
                    </tbody>
                </table>
                {move || rows.get().is_empty().then(|| view! { <div class="workbench-table-empty">{move || if data.market.get().problem.is_some() { "行情时效待恢复，等待更新或刷新状态" } else { empty_message(&data.status.get()) }}</div> })}
            </div>
        </section>
    }
}

fn route_table(data: GateCrossExData) -> impl IntoView {
    let selected = Memo::new(move |_| selected_routes(data));
    view! {
        <section class="gate-crossex-panel">
            <header><strong>"已选路由行情"</strong>
                <button type="button" class="row-action" disabled=move || controls_disabled(data) || selected.get().is_empty()
                    on:click=move |_| data.clear_routes.run(())>"清空已选"</button>
            </header>
            <div class="workbench-table-wrap">
                <table class="workbench-table clean-table gate-crossex-route-table" data-table-budget="bounded-small">
                    <thead><tr><th>"市场 / 路由"</th><th class="num">"Bid"</th><th class="num">"Ask"</th><th class="num">"Last"</th><th class="num">"状态"</th><th class="gate-crossex-route-action">"操作"</th></tr></thead>
                    <tbody><For each=move || selected.get() key=|key| key.clone() children=move |native| {
                        let native = StoredValue::new(native);
                        let row = Memo::new(move |_| data.status.get().value().and_then(|snapshot| snapshot.routes.iter().find(|row| row.native_symbol == native.get_value()).cloned()));
                        view! { <tr>
                            <td class="gate-crossex-market" title=move || native.get_value()><strong>{move || row.get().map(|row| format!("{}/{}", row.base_asset, row.quote_asset)).unwrap_or_else(|| native.get_value())}</strong>
                                <small>{move || row.get().map(|row| format!("{} · {}", row.underlying_venue.to_ascii_uppercase(), product_label(row.product))).unwrap_or_default()}</small></td>
                            <td class="num gate-crossex-bid" data-label="Bid"><span class="gate-crossex-bid-value">{move || row.get().map(|row| price(row.bid)).unwrap_or_else(|| "--".to_owned())}</span></td>
                            <td class="num gate-crossex-ask" data-label="Ask">{move || row.get().map(|row| price(row.ask)).unwrap_or_else(|| "--".to_owned())}</td>
                            <td class="num gate-crossex-last" data-label="Last">{move || row.get().map(|row| price(row.last)).unwrap_or_else(|| "--".to_owned())}</td>
                            <td class="num gate-crossex-time" data-label="状态">{move || row.get().map(|row| quote_age(data, row.observed_at_ms)).unwrap_or_else(|| {
                                if data.status.get().problem().is_some() { "读取失败" }
                                else if data.status.get().value().is_some_and(|row| row.config.mode == GateCrossExMode::Disabled) { "已关闭" }
                                else { "等待报价" }.to_owned()
                            })}</td>
                            <td class="gate-crossex-route-action"><button type="button" class="row-action"
                                aria-label=move || format!("移除 {}", native.get_value())
                                disabled=move || controls_disabled(data)
                                on:click=move |_| data.remove_route.run(native.get_value())>"移除"</button></td>
                        </tr> }
                    }/></tbody>
                </table>
                {move || selected.get().is_empty().then(|| view! { <div class="workbench-table-empty">"尚未选择监控路由"</div> })}
            </div>
        </section>
    }
}

fn quote_age(data: GateCrossExData, observed: i64) -> String {
    let state = data.status.get();
    if state.problem().is_some() {
        return "上次快照".to_owned();
    }
    let market = data.market.get();
    match market.age(observed) {
        Some(age) if market.current(observed) => format!("{:.1}s", age as f64 / 1000.0),
        Some(age) => format!("过期 · {:.1}s", age as f64 / 1000.0),
        None => "时效未知".to_owned(),
    }
}

fn route_venue(native: &str) -> String {
    native.split('_').next().unwrap_or(native).to_owned()
}

const fn product_label(product: GateCrossExProduct) -> &'static str {
    match product {
        GateCrossExProduct::Spot => "现货",
        GateCrossExProduct::Future => "永续",
    }
}

fn status_label(state: &LoadState<GateCrossExModeSnapshot>) -> &'static str {
    match state {
        LoadState::Loading => "读取中",
        LoadState::Error(_) => "读取失败",
        LoadState::Stale { .. } => "上次快照",
        LoadState::Ready(snapshot) => match snapshot.runtime_state {
            GateCrossExRuntimeState::Disabled => "已关闭",
            GateCrossExRuntimeState::WaitingForRegistry => "等待官方路由",
            GateCrossExRuntimeState::Warming if snapshot.selected_count == 0 => "待选择路由",
            GateCrossExRuntimeState::Warming => "等待 WS 报价",
            GateCrossExRuntimeState::Live => "WS 实时",
            GateCrossExRuntimeState::Degraded => "部分行情未就绪",
        },
    }
}

fn empty_message(state: &LoadState<GateCrossExModeSnapshot>) -> &'static str {
    match state {
        LoadState::Loading => "正在读取监控状态…",
        LoadState::Error(_) | LoadState::Stale { .. } => "监控状态读取失败，请刷新后重试",
        LoadState::Ready(row) => match row.runtime_state {
            GateCrossExRuntimeState::Disabled => "监控已关闭",
            GateCrossExRuntimeState::WaitingForRegistry => "等待官方路由目录",
            _ if row.selected_count < 2 => "选择同币种、同报价币、不同场所的两条路由",
            GateCrossExRuntimeState::Warming => "等待所选路由的 WS 报价",
            GateCrossExRuntimeState::Degraded => "部分路由缺少报价，暂未发现符合阈值的毛价差",
            _ => "当前没有达到阈值的跨路由毛价差",
        },
    }
}

fn price(value: f64) -> String {
    if !value.is_finite() || value <= 0.0 {
        "--".to_owned()
    } else if value >= 1_000.0 {
        format!("{value:.2}")
    } else if value >= 1.0 {
        format!("{value:.4}")
    } else {
        format!("{value:.8}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_read_cannot_claim_live_or_no_opportunities() {
        let snapshot = GateCrossExModeSnapshot {
            runtime_state: GateCrossExRuntimeState::Live,
            ..Default::default()
        };
        let stale = LoadState::Stale {
            value: snapshot,
            problem: shared_types::ApiProblem::new("OFFLINE", "offline"),
        };
        assert_eq!(status_label(&stale), "上次快照");
        assert!(empty_message(&stale).contains("读取失败"));
    }
    #[test]
    fn disabled_and_warming_empty_states_are_distinct() {
        let mut snapshot = GateCrossExModeSnapshot::default();
        assert_eq!(
            empty_message(&LoadState::Ready(snapshot.clone())),
            "监控已关闭"
        );
        snapshot.runtime_state = GateCrossExRuntimeState::Warming;
        assert_eq!(
            status_label(&LoadState::Ready(snapshot.clone())),
            "待选择路由"
        );
        snapshot.selected_count = 2;
        assert!(empty_message(&LoadState::Ready(snapshot)).contains("等待"));
    }
    #[test]
    fn unavailable_price_is_not_displayed_as_zero() {
        for value in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(price(value), "--");
        }
        assert_eq!(price(0.00001234), "0.00001234");
    }
}
