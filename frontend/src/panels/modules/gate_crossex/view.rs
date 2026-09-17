use crate::panels::shared::ModuleHeader;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{
    GateCrossExMode, GateCrossExModeSnapshot, GateCrossExProduct, GateCrossExRouteCatalogRow,
    GateCrossExRuntimeState,
};

use super::data::{use_gate_crossex_data, GateCrossExData, GateCrossExRuntime};

pub(in crate::panels) fn gate_crossex_module(runtime: GateCrossExRuntime) -> impl IntoView {
    let data = use_gate_crossex_data(runtime);
    view! {
        <section class="module-page gate-crossex-page">
            <ModuleHeader title="CrossEx"/>
            <div class="gate-crossex-workbench">
                {control_rail(data)}
                <div class="gate-crossex-main">
                    {runtime_strip(data.status)}
                    {candidate_table(data.status)}
                    {route_table(data.status)}
                </div>
            </div>
        </section>
    }
}

fn control_rail(data: GateCrossExData) -> impl IntoView {
    view! {
        <aside class="gate-crossex-control" aria-label="Gate CrossEx 模式与路由">
            <header class="workbench-rail-header">
                <div><strong>"Gate CrossEx"</strong><span>"官方 native route"</span></div>
                <span class="read-only-flag">"监控"</span>
            </header>
            <div class="gate-crossex-segmented" role="group" aria-label="CrossEx 模式">
                <button
                    type="button"
                    class=move || mode_class(data.status, GateCrossExMode::Disabled)
                    on:click=move |_| data.set_mode.run(GateCrossExMode::Disabled)
                >"关闭"</button>
                <button
                    type="button"
                    class=move || mode_class(data.status, GateCrossExMode::Monitor)
                    on:click=move |_| data.set_mode.run(GateCrossExMode::Monitor)
                >"监控"</button>
            </div>
            <label class="workbench-field">
                <span>"最小毛价差 (%)"</span>
                <input
                    type="number"
                    min="0"
                    max="100"
                    step="0.01"
                    prop:value=move || minimum_value(data.status)
                    on:change=move |event| {
                        if let Ok(value) = event_target_value(&event).parse::<f64>() {
                            data.set_minimum.run(value);
                        }
                    }
                />
            </label>
            <label class="workbench-field">
                <span>"选择 native route"</span>
                <input
                    type="search"
                    placeholder="BTC / Kraken / USDT"
                    prop:value=move || data.search.get()
                    on:input=move |event| data.search.set(event_target_value(&event))
                />
            </label>
            {route_catalog(data)}
            <div class="workbench-boundary-note">
                <strong>"只显示毛价差"</strong>
                <span>"手续费、深度、账户和订单终态全部通过后，才会进入执行闭环。"</span>
            </div>
            <output class="gate-crossex-notice">{move || data.notice.get().unwrap_or_default()}</output>
        </aside>
    }
}

fn route_catalog(data: GateCrossExData) -> impl IntoView {
    move || match data.catalog.get() {
        LoadState::Loading => {
            view! { <div class="gate-crossex-route-state">"正在读取官方路由…"</div> }.into_any()
        }
        LoadState::Error(problem) => {
            view! { <div class="gate-crossex-route-state is-danger">{problem.message}</div> }
                .into_any()
        }
        LoadState::Ready(catalog) | LoadState::Stale { value: catalog, .. } => {
            let selected = selected_routes(data.status);
            view! {
                <div class="gate-crossex-route-picker">
                    <div class="gate-crossex-route-count">{format!("显示 {} / {}", catalog.routes.len(), catalog.total)}</div>
                    {catalog.routes.into_iter().map(|row| route_option(data, &selected, row)).collect_view()}
                </div>
            }
            .into_any()
        }
    }
}

fn route_option(
    data: GateCrossExData,
    selected: &[String],
    row: GateCrossExRouteCatalogRow,
) -> impl IntoView {
    let native_symbol = row.native_symbol.clone();
    let checked = selected.iter().any(|route| route == &native_symbol);
    view! {
        <label class="gate-crossex-route-option">
            <input
                type="checkbox"
                prop:checked=checked
                on:change=move |_| data.toggle_route.run(native_symbol.clone())
            />
            <span><strong>{row.base_asset}</strong><small>{format!("{} · {}", row.underlying_venue.to_ascii_uppercase(), product_label(row.product))}</small></span>
            <code>{row.quote_asset}</code>
        </label>
    }
}

fn runtime_strip(status: RwSignal<LoadState<GateCrossExModeSnapshot>>) -> impl IntoView {
    move || match status.get() {
        LoadState::Loading => {
            view! { <div class="gate-crossex-runtime is-warming">"正在读取 CrossEx 运行态…"</div> }
                .into_any()
        }
        LoadState::Error(problem) => {
            view! { <div class="gate-crossex-runtime is-danger">{problem.message}</div> }.into_any()
        }
        LoadState::Ready(snapshot)
        | LoadState::Stale {
            value: snapshot, ..
        } => {
            let tone = runtime_tone(snapshot.runtime_state);
            let problem = snapshot
                .problem
                .as_ref()
                .map(|problem| problem.message.clone());
            view! {
                <section class=format!("gate-crossex-runtime {tone}")>
                    <div><span>"状态"</span><strong>{runtime_label(snapshot.runtime_state)}</strong></div>
                    <div><span>"官方路由"</span><strong class="num">{snapshot.catalog_count}</strong></div>
                    <div><span>"已选"</span><strong class="num">{snapshot.selected_count}</strong></div>
                    <div><span>"WS 新鲜"</span><strong class="num">{snapshot.live_count}</strong></div>
                    <div><span>"毛价差候选"</span><strong class="num">{snapshot.candidates.len()}</strong></div>
                    {problem.map(|problem| view! { <p>{problem}</p> })}
                </section>
            }
            .into_any()
        }
    }
}

fn candidate_table(status: RwSignal<LoadState<GateCrossExModeSnapshot>>) -> impl IntoView {
    move || {
        let Some(snapshot) = status.get().value().cloned() else {
            return view! { <section class="gate-crossex-panel"><header><strong>"跨路由毛价差"</strong></header><div class="workbench-table-empty">"等待行情"</div></section> }.into_any();
        };
        let empty = snapshot.candidates.is_empty();
        view! {
            <section class="gate-crossex-panel">
                <header><strong>"跨路由毛价差"</strong><span>"Ask 买入 → Bid 卖出"</span></header>
                <div class="workbench-table-wrap">
                    <table class="workbench-table gate-crossex-candidate-table" data-table-budget="bounded-small">
                        <thead><tr><th>"市场"</th><th>"买入路由"</th><th>"卖出路由"</th><th class="num">"买入价"</th><th class="num">"卖出价"</th><th class="num">"毛价差"</th><th>"状态"</th></tr></thead>
                        <tbody>
                            {snapshot.candidates.into_iter().map(|row| view! {
                                <tr>
                                    <td><strong>{format!("{}/{}", row.base_asset, row.quote_asset)}</strong><small>{product_label(row.product)}</small></td>
                                    <td><code>{row.long_route}</code></td>
                                    <td><code>{row.short_route}</code></td>
                                    <td class="num">{price(row.long_ask)}</td>
                                    <td class="num">{price(row.short_bid)}</td>
                                    <td class="num is-positive">{format!("+{:.3}%", row.gross_spread_pct)}</td>
                                    <td><span class="gate-crossex-observe">"待净收益核验"</span></td>
                                </tr>
                            }).collect_view()}
                        </tbody>
                    </table>
                    {empty.then(|| view! { <div class="workbench-table-empty">"当前没有达到阈值的同币种、同报价币跨路由毛价差"</div> })}
                </div>
            </section>
        }
        .into_any()
    }
}

fn route_table(status: RwSignal<LoadState<GateCrossExModeSnapshot>>) -> impl IntoView {
    move || {
        let Some(snapshot) = status.get().value().cloned() else {
            return view! { <section class="gate-crossex-panel"><header><strong>"Native route 行情"</strong></header><div class="workbench-table-empty">"等待行情"</div></section> }.into_any();
        };
        let empty = snapshot.routes.is_empty();
        view! {
            <section class="gate-crossex-panel">
                <header><strong>"Native route 行情"</strong><span>"Gate CrossEx 公共 WS"</span></header>
                <div class="workbench-table-wrap">
                    <table class="workbench-table gate-crossex-route-table" data-table-budget="bounded-small">
                        <thead><tr><th>"Native symbol"</th><th>"底层场所"</th><th>"产品"</th><th class="num">"Bid"</th><th class="num">"Ask"</th><th class="num">"Last"</th></tr></thead>
                        <tbody>{snapshot.routes.into_iter().map(|row| view! {
                            <tr><td><code>{row.native_symbol}</code></td><td>{row.underlying_venue.to_ascii_uppercase()}</td><td>{product_label(row.product)}</td><td class="num">{price(row.bid)}</td><td class="num">{price(row.ask)}</td><td class="num">{price(row.last)}</td></tr>
                        }).collect_view()}</tbody>
                    </table>
                    {empty.then(|| view! { <div class="workbench-table-empty">"启用监控并选择路由后开始订阅"</div> })}
                </div>
            </section>
        }
        .into_any()
    }
}

fn selected_routes(status: RwSignal<LoadState<GateCrossExModeSnapshot>>) -> Vec<String> {
    status
        .get()
        .value()
        .map(|snapshot| snapshot.config.selected_routes.clone())
        .unwrap_or_default()
}

fn mode_class(
    status: RwSignal<LoadState<GateCrossExModeSnapshot>>,
    mode: GateCrossExMode,
) -> &'static str {
    if status
        .get()
        .value()
        .is_some_and(|snapshot| snapshot.config.mode == mode)
    {
        "is-active"
    } else {
        ""
    }
}

fn minimum_value(status: RwSignal<LoadState<GateCrossExModeSnapshot>>) -> String {
    status
        .get()
        .value()
        .map(|snapshot| format!("{:.2}", snapshot.config.min_gross_spread_pct))
        .unwrap_or_else(|| "0.10".to_owned())
}

const fn product_label(product: GateCrossExProduct) -> &'static str {
    match product {
        GateCrossExProduct::Spot => "现货",
        GateCrossExProduct::Future => "永续",
    }
}

const fn runtime_label(state: GateCrossExRuntimeState) -> &'static str {
    match state {
        GateCrossExRuntimeState::Disabled => "已关闭",
        GateCrossExRuntimeState::WaitingForRegistry => "等待路由",
        GateCrossExRuntimeState::Warming => "预热中",
        GateCrossExRuntimeState::Live => "WS 实时",
        GateCrossExRuntimeState::Degraded => "数据降级",
    }
}

const fn runtime_tone(state: GateCrossExRuntimeState) -> &'static str {
    match state {
        GateCrossExRuntimeState::Live => "is-live",
        GateCrossExRuntimeState::Degraded => "is-danger",
        GateCrossExRuntimeState::Disabled => "is-disabled",
        GateCrossExRuntimeState::WaitingForRegistry | GateCrossExRuntimeState::Warming => {
            "is-warming"
        }
    }
}

fn price(value: f64) -> String {
    if value >= 1_000.0 {
        format!("{value:.2}")
    } else if value >= 1.0 {
        format!("{value:.4}")
    } else {
        format!("{value:.8}")
    }
}
