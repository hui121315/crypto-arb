use leptos::prelude::*;
use shared_types::{
    OnchainCexComparison, OnchainComparisonDirection, OnchainComparisonQuality,
    OnchainComparisonSnapshot,
};

use super::components::{
    batch_watchlist, command_rail, decision_board, evidence_ledger, market_sidebar, market_tape,
    source_telemetry, OnchainConfigTask,
};
use super::data::{use_onchain_data, use_onchain_form_data, OnchainRuntime, PreviewContext};
use super::draft::OnchainConfigDraft;

pub(in crate::panels) fn onchain_module(runtime: OnchainRuntime) -> impl IntoView {
    let data = use_onchain_data(runtime);
    let draft = runtime.draft;
    let active_task = RwSignal::new(OnchainConfigTask::Market);
    let active_runtime = RwSignal::new(OnchainRuntimeTask::Sources);
    let runtime_expanded = RwSignal::new(false);
    let active_direction = data.execution.preview;
    sync_direction_to_market(data.state, active_direction);
    let mobile_pane = RwSignal::new(OnchainMobilePane::Analysis);
    let market_rail_open = RwSignal::new(false);
    let open_markets = Callback::new(move |()| {
        market_rail_open.set(true);
        mobile_pane.set(OnchainMobilePane::Markets);
    });
    let return_to_analysis = Callback::new(move |()| {
        market_rail_open.set(false);
        mobile_pane.set(OnchainMobilePane::Analysis);
    });
    let show_execution_result = Callback::new(move |()| {
        mobile_pane.set(OnchainMobilePane::Analysis);
    });
    let open_execution_setup = Callback::new(move |()| {
        active_task.set(OnchainConfigTask::Connectivity);
        mobile_pane.set(OnchainMobilePane::Controls);
    });
    use_onchain_form_data(draft, data);
    view! {
        <section class="module-page onchain-page">
            {crate::panels::shared::operation_journal::settings_recovery_panel(data.configuration.journal, data.configuration.recheck)}
            <Show when=move || data.configuration.needs_current.get()>
                <div class="provider-credentials-feedback provider-credentials-recovery has-action" role="alert" aria-label="链上当前配置待同步">
                    <span>"原操作已核对；当前配置尚待确认，暂不能修改或构建。"</span>
                    <button type="button" class="row-action" on:click=move |_| data.configuration.read_current.run(())>"读取当前配置"</button>
                </div>
            </Show>
            <Show when=move || data.configuration.journal.connection.get() != 0>
                <p role="alert">"后端连接已更换，请刷新页面后读取当前配置。原请求的恢复记录仍保留在对应连接下。"</p>
            </Show>
            {mobile_workspace_tabs(mobile_pane, market_rail_open)}
            <div class=move || mobile_workbench_class(mobile_pane.get())>
                {market_sidebar(draft, data, market_rail_open, return_to_analysis)}
                <button
                    type="button"
                    class="onchain-market-drawer-scrim"
                    aria-label="关闭市场列表"
                    hidden=move || !market_rail_open.get()
                    on:click=move |_| market_rail_open.set(false)
                ></button>
                <div class="onchain-terminal-main">
                    {market_tape(draft, data, market_rail_open, open_markets)}
                    <div class="onchain-main-column">
                        <div class="onchain-primary-market">
                            {decision_board(
                                data,
                                open_execution_setup,
                                active_direction,
                                show_execution_result,
                            )}
                        </div>
                        {runtime_dock(draft, data, active_runtime, runtime_expanded)}
                    </div>
                </div>
                {command_rail(draft, data, active_task)}
            </div>
        </section>
    }
}

fn sync_direction_to_market(
    state: RwSignal<crate::state::load_state::LoadState<OnchainComparisonSnapshot>>,
    active: PreviewContext,
) {
    let last_market = StoredValue::new(None::<String>);
    Effect::new(move |_| {
        let next = state.with(|state| {
            let snapshot = state.value()?;
            let preferred = preferred_direction(snapshot)?;
            let market = market_identity(snapshot);
            let current_available = snapshot
                .comparisons
                .iter()
                .any(|row| row.direction == active.get_untracked());
            Some((market, preferred, current_available))
        });
        let Some((market, preferred, current_available)) = next else {
            return;
        };
        let market_changed = last_market.with_value(|last| last.as_deref() != Some(&market));
        if market_changed || !current_available {
            active.set(preferred);
        }
        last_market.set_value(Some(market));
    });
}

fn market_identity(snapshot: &OnchainComparisonSnapshot) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}",
        snapshot.config.chain,
        snapshot.config.provider,
        snapshot.config.base_mint,
        snapshot.config.quote_mint,
        snapshot.config.cex_venue,
        snapshot.config.cex_symbol,
    )
}

fn preferred_direction(snapshot: &OnchainComparisonSnapshot) -> Option<OnchainComparisonDirection> {
    snapshot
        .comparisons
        .iter()
        .max_by(|left, right| {
            direction_edge(snapshot.quality, left)
                .total_cmp(&direction_edge(snapshot.quality, right))
        })
        .map(|row| row.direction)
}

fn direction_edge(quality: OnchainComparisonQuality, row: &OnchainCexComparison) -> f64 {
    if matches!(
        quality,
        OnchainComparisonQuality::RawCrossQuote | OnchainComparisonQuality::RawCustomPair
    ) {
        row.gross_spread_bps
    } else {
        row.net_spread_bps
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OnchainRuntimeTask {
    Sources,
    Watchlist,
    Evidence,
}

fn runtime_dock(
    draft: OnchainConfigDraft,
    data: super::data::OnchainData,
    active: RwSignal<OnchainRuntimeTask>,
    expanded: RwSignal<bool>,
) -> impl IntoView {
    view! {
        <section
            class=move || if expanded.get() {
                "onchain-runtime-dock is-expanded"
            } else {
                "onchain-runtime-dock is-collapsed"
            }
            aria-label="链上套利运行台"
        >
            {runtime_tabs(active, expanded)}
            <div class="onchain-runtime-workspace" hidden=move || !expanded.get()>
                <div
                    id="onchain-runtime-panel-sources"
                    class="onchain-runtime-panel"
                    role="tabpanel"
                    aria-labelledby="onchain-runtime-tab-sources"
                    hidden=move || active.get() != OnchainRuntimeTask::Sources
                >
                    {source_telemetry(data.state, data.transport)}
                </div>
                <div
                    id="onchain-runtime-panel-watchlist"
                    class="onchain-runtime-panel"
                    role="tabpanel"
                    aria-labelledby="onchain-runtime-tab-watchlist"
                    hidden=move || active.get() != OnchainRuntimeTask::Watchlist
                >
                    {batch_watchlist(draft, data)}
                </div>
                <div
                    id="onchain-runtime-panel-evidence"
                    class="onchain-runtime-panel"
                    role="tabpanel"
                    aria-labelledby="onchain-runtime-tab-evidence"
                    hidden=move || active.get() != OnchainRuntimeTask::Evidence
                >
                    {evidence_ledger(data.state)}
                </div>
            </div>
        </section>
    }
}

fn runtime_tabs(active: RwSignal<OnchainRuntimeTask>, expanded: RwSignal<bool>) -> impl IntoView {
    let sources_ref = NodeRef::<leptos::html::Button>::new();
    let watchlist_ref = NodeRef::<leptos::html::Button>::new();
    let evidence_ref = NodeRef::<leptos::html::Button>::new();
    view! {
        <nav
            class="onchain-runtime-tabs"
            role="tablist"
            aria-label="链上套利运行结果"
            on:keydown=move |event| {
                let next = runtime_tab_target(active.get(), &event.key());
                let Some(next) = next else { return };
                event.prevent_default();
                active.set(next);
                expanded.set(true);
                let target = match next {
                    OnchainRuntimeTask::Sources => sources_ref,
                    OnchainRuntimeTask::Watchlist => watchlist_ref,
                    OnchainRuntimeTask::Evidence => evidence_ref,
                };
                if let Some(button) = target.get() {
                    let _ = button.focus();
                }
            }
        >
            <button
                id="onchain-runtime-tab-sources"
                type="button"
                role="tab"
                node_ref=sources_ref
                aria-controls="onchain-runtime-panel-sources"
                aria-selected=move || runtime_selected(active, OnchainRuntimeTask::Sources)
                tabindex=move || runtime_tab_index(active, OnchainRuntimeTask::Sources)
                class:active=move || active.get() == OnchainRuntimeTask::Sources
                on:click=move |_| select_runtime_task(active, expanded, OnchainRuntimeTask::Sources)
            >"来源"</button>
            <button
                id="onchain-runtime-tab-watchlist"
                type="button"
                role="tab"
                node_ref=watchlist_ref
                aria-controls="onchain-runtime-panel-watchlist"
                aria-selected=move || runtime_selected(active, OnchainRuntimeTask::Watchlist)
                tabindex=move || runtime_tab_index(active, OnchainRuntimeTask::Watchlist)
                class:active=move || active.get() == OnchainRuntimeTask::Watchlist
                on:click=move |_| select_runtime_task(active, expanded, OnchainRuntimeTask::Watchlist)
            >"监控"</button>
            <button
                id="onchain-runtime-tab-evidence"
                type="button"
                role="tab"
                node_ref=evidence_ref
                aria-controls="onchain-runtime-panel-evidence"
                aria-selected=move || runtime_selected(active, OnchainRuntimeTask::Evidence)
                tabindex=move || runtime_tab_index(active, OnchainRuntimeTask::Evidence)
                class:active=move || active.get() == OnchainRuntimeTask::Evidence
                on:click=move |_| select_runtime_task(active, expanded, OnchainRuntimeTask::Evidence)
            >"数据依据"</button>
            <button
                type="button"
                class="onchain-runtime-toggle"
                aria-label=move || if expanded.get() { "收起运行台" } else { "展开运行台" }
                aria-expanded=move || expanded.get().to_string()
                on:click=move |_| expanded.update(|expanded| *expanded = !*expanded)
            >{move || if expanded.get() { "⌄" } else { "⌃" }}</button>
        </nav>
    }
}

fn select_runtime_task(
    active: RwSignal<OnchainRuntimeTask>,
    expanded: RwSignal<bool>,
    task: OnchainRuntimeTask,
) {
    active.set(task);
    expanded.set(true);
}

fn runtime_tab_target(current: OnchainRuntimeTask, key: &str) -> Option<OnchainRuntimeTask> {
    let index = runtime_tab_index_value(current);
    let target = match key {
        "ArrowRight" => (index + 1) % 3,
        "ArrowLeft" => (index + 2) % 3,
        "Home" => 0,
        "End" => 2,
        _ => return None,
    };
    Some(runtime_task_at(target))
}

const fn runtime_tab_index_value(task: OnchainRuntimeTask) -> usize {
    match task {
        OnchainRuntimeTask::Sources => 0,
        OnchainRuntimeTask::Watchlist => 1,
        OnchainRuntimeTask::Evidence => 2,
    }
}

const fn runtime_task_at(index: usize) -> OnchainRuntimeTask {
    match index {
        0 => OnchainRuntimeTask::Sources,
        1 => OnchainRuntimeTask::Watchlist,
        _ => OnchainRuntimeTask::Evidence,
    }
}

fn runtime_selected(
    active: RwSignal<OnchainRuntimeTask>,
    task: OnchainRuntimeTask,
) -> &'static str {
    if active.get() == task {
        "true"
    } else {
        "false"
    }
}

fn runtime_tab_index(active: RwSignal<OnchainRuntimeTask>, task: OnchainRuntimeTask) -> i32 {
    if active.get() == task {
        0
    } else {
        -1
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OnchainMobilePane {
    Markets,
    Analysis,
    Controls,
}

fn mobile_workspace_tabs(
    active: RwSignal<OnchainMobilePane>,
    market_rail_open: RwSignal<bool>,
) -> impl IntoView {
    view! {
        <nav class="onchain-mobile-workspace-tabs" aria-label="链上套利工作区">
            <button
                type="button"
                aria-selected=move || mobile_selected(active, OnchainMobilePane::Markets)
                on:click=move |_| select_mobile_pane(
                    active,
                    market_rail_open,
                    OnchainMobilePane::Markets,
                )
            >"市场"</button>
            <button
                type="button"
                aria-selected=move || mobile_selected(active, OnchainMobilePane::Analysis)
                on:click=move |_| select_mobile_pane(
                    active,
                    market_rail_open,
                    OnchainMobilePane::Analysis,
                )
            >"套利"</button>
            <button
                type="button"
                aria-selected=move || mobile_selected(active, OnchainMobilePane::Controls)
                on:click=move |_| select_mobile_pane(
                    active,
                    market_rail_open,
                    OnchainMobilePane::Controls,
                )
            >"接入"</button>
        </nav>
    }
}

fn select_mobile_pane(
    active: RwSignal<OnchainMobilePane>,
    market_rail_open: RwSignal<bool>,
    pane: OnchainMobilePane,
) {
    market_rail_open.set(false);
    active.set(pane);
    if let Some(window) = web_sys::window() {
        window.scroll_to_with_x_and_y(0.0, 0.0);
    }
}

const fn mobile_workbench_class(active: OnchainMobilePane) -> &'static str {
    match active {
        OnchainMobilePane::Markets => "onchain-workbench mobile-markets",
        OnchainMobilePane::Analysis => "onchain-workbench mobile-analysis",
        OnchainMobilePane::Controls => "onchain-workbench mobile-controls",
    }
}

fn mobile_selected(active: RwSignal<OnchainMobilePane>, pane: OnchainMobilePane) -> &'static str {
    if active.get() == pane {
        "true"
    } else {
        "false"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_tabs_wrap_without_hiding_the_market_canvas() {
        assert_eq!(
            runtime_tab_target(OnchainRuntimeTask::Sources, "ArrowLeft"),
            Some(OnchainRuntimeTask::Evidence)
        );
        assert_eq!(
            runtime_tab_target(OnchainRuntimeTask::Evidence, "ArrowRight"),
            Some(OnchainRuntimeTask::Sources)
        );
    }

    #[test]
    fn verified_market_opens_on_the_best_net_direction() {
        let mut snapshot = OnchainComparisonSnapshot::default();
        snapshot.quality = OnchainComparisonQuality::Fresh;
        snapshot.comparisons = vec![
            comparison(OnchainComparisonDirection::BuyOnchainSellCex, 90.0, 40.0),
            comparison(OnchainComparisonDirection::BuyCexSellOnchain, 80.0, 55.0),
        ];

        assert_eq!(
            preferred_direction(&snapshot),
            Some(OnchainComparisonDirection::BuyCexSellOnchain)
        );
    }

    #[test]
    fn raw_market_opens_on_the_best_observed_direction() {
        let mut snapshot = OnchainComparisonSnapshot::default();
        snapshot.quality = OnchainComparisonQuality::RawCustomPair;
        snapshot.comparisons = vec![
            comparison(OnchainComparisonDirection::BuyOnchainSellCex, 90.0, 30.0),
            comparison(OnchainComparisonDirection::BuyCexSellOnchain, 80.0, 60.0),
        ];

        assert_eq!(
            preferred_direction(&snapshot),
            Some(OnchainComparisonDirection::BuyOnchainSellCex)
        );
    }

    fn comparison(
        direction: OnchainComparisonDirection,
        gross_spread_bps: f64,
        net_spread_bps: f64,
    ) -> OnchainCexComparison {
        OnchainCexComparison {
            direction,
            onchain_price: 1.0,
            cex_price: 1.0,
            gross_spread_bps,
            cex_fee_bps: 0.0,
            quote_conversion_fee_bps: 0.0,
            slippage_bps: 0.0,
            gas_usd: 0.0,
            gas_bps: 0.0,
            total_cost_bps: gross_spread_bps - net_spread_bps,
            net_spread_bps,
            observable_notional_usd: 100.0,
            executable: false,
        }
    }
}
