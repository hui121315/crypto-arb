use crate::api::ws::WsChannelState;
use crate::panels::nav::view::ModuleNav;
use crate::panels::status_bar::data::{
    use_scan_status_state, use_system_health_state, VenueOperationHealthState,
};
use crate::panels::status_bar::problem_ledger::problem_evidence_ledger;
use crate::panels::status_bar::slots::{
    api_status_slot, app_connection_readiness, operation_readiness, scan_status_slot,
    AppWsStatusSlot, ExecutionModeSlot, MarketDataStatusSlot, NetDeltaSlot, NextFundingSlot,
    OrderElapsedSlot, RiskSlot, WsStatusSlot,
};
use crate::panels::status_bar::summary::summarize_system_state;
use crate::panels::workstation::{ModuleId, WorkspaceRuntime};
use crate::state::module_runtime::ModuleRuntimeStatus;
use leptos::prelude::*;
use shared_types::{ApiProblem, SystemHealth, VenueOperationHealthSnapshot};

#[component]
pub(in crate::panels) fn TopStatusBar(runtime: WorkspaceRuntime) -> impl IntoView {
    let health_state = use_system_health_state();
    let operation_health_state = expect_context::<VenueOperationHealthState>();
    let trading_state = runtime.trading_status();
    let scan_state = use_scan_status_state();
    let health = Memo::new(move |_| health_state.state.get().value().cloned());
    let operation_health = Memo::new(move |_| operation_health_state.state.get().value().cloned());
    let trading = Memo::new(move |_| trading_state.get().value().cloned());
    let environment = Memo::new(move |_| trading.get().map(|status| status.environment));
    let order_elapsed_ms =
        Memo::new(move |_| current_health(health).and_then(|h| h.order_elapsed_ms));
    let risk = Memo::new(move |_| current_health(health).map(|h| h.risk));
    let scalar_problem = Memo::new(move |_| health_state.state.get().problem().cloned());
    let trading_problem = Memo::new(move |_| trading_state.get().problem().cloned());
    let problems = Memo::new(move |_| {
        current_health(health)
            .map(|h| h.problems)
            .unwrap_or_default()
    });
    let delta = Memo::new(move |_| {
        current_health(health).map(|h| (h.net_delta_usd, h.net_delta_pct_of_nav))
    });
    let operation_health_problem =
        Memo::new(move |_| operation_health_state.state.get().problem().cloned());
    let app_ws_channel = Memo::new(move |_| health_state.ws_channel.get());
    let readiness = Memo::new(move |_| {
        let mut categories =
            operation_readiness(&operation_health_state.state.get(), environment.get());
        categories.push(app_connection_readiness(
            &app_ws_channel.get(),
            operation_health.get().as_ref(),
        ));
        categories
    });
    let next = Memo::new(move |_| current_health(health).and_then(|h| h.next_funding));
    let module_health = Memo::new(move |_| {
        let module = runtime.active().get();
        (module.title(), runtime.module_runtime_state(module))
    });
    let health_summary = Memo::new(move |_| {
        summarize_system_state(&health_state.state.get())
            .with_readiness(&readiness.get())
            .with_trading_status(&trading_state.get())
            .with_module(module_health.get().0, &module_health.get().1)
    });

    view! {
        <header class="mod-topbar">
            <div class="topbar-primary">
                <div class="mod-brand">
                    <strong>"CROSSLINE"</strong>
                    <span>"Omni"</span>
                </div>
                <ModuleNav runtime=runtime/>
            </div>
            <div class="topbar-status-frame">
                <div class="status-summary-row" data-testid="top-status-bar">
                    <details class="status-disclosure">
                        <summary
                            class="status-summary"
                            data-state=move || health_summary.with(|summary| summary.state)
                        >
                            <span class="status-summary-dot" aria-hidden="true"></span>
                            <strong>{move || health_summary.with(|summary| summary.label)}</strong>
                            <span class="status-summary-detail">
                                {move || health_summary.with(|summary| summary.detail.clone())}
                            </span>
                            <span class="status-summary-action" aria-hidden="true">
                                <span class="status-summary-action-closed">"查看数据依据"</span>
                                <span class="status-summary-action-open">"收起数据依据"</span>
                            </span>
                        </summary>
                        <div class="top-slots status-details-panel" role="group" aria-label="系统状态详情">
                            <Show when=move || readiness.with(|rows| rows.iter().any(|row| row.readiness.needs_attention()))>
                                <div class="status-module-evidence" role="group" aria-label="接口与后台状态">
                                    <strong>"接口与后台状态"</strong>
                                    {move || readiness.get().into_iter().filter(|row| row.readiness.needs_attention())
                                        .map(|row| view! { <span>{format!("{} · {}：{}", row.category.label(), row.readiness.label(), row.detail)}</span> })
                                        .collect_view()}
                                </div>
                            </Show>
                            {move || scalar_problem.get().map(|problem| view! {
                                <div class="status-module-evidence" role="group" aria-label="系统快照状态">
                                    <strong>{move || if health.get().is_some() {
                                        "风险与资金数值未确认，仅供参考"
                                    } else { "尚无可用的风险与资金快照" }}</strong>
                                    <span>{format!("{} · {}", problem.code, problem.message)}</span>
                                </div>
                            })}
                            <Show when=move || module_health.with(|(_, state)| state.status != ModuleRuntimeStatus::Ready)>
                                <div class="status-module-evidence" role="group" aria-label="当前模块状态">
                                    <strong>{move || module_health.with(|(name, state)| format!("{name} · {}", state.pending_label.as_deref().unwrap_or(state.label())))}</strong>
                                    {move || module_health.with(|(_, state)| state.problem.as_ref().map(|problem| view! {
                                        <span>{format!("{} · {}", problem.code, problem.message)}</span>
                                    }))}
                                </div>
                            </Show>
                            <div class="status-cluster connectivity" role="group" aria-label="连接状态">
                                <span class="status-cluster-label">"连接"</span>
                                <RuntimeCategorySlots
                                    operation_health=operation_health
                                    operation_problem=operation_health_problem
                                    app_ws_channel=app_ws_channel
                                    environment=environment
                                />
                            </div>
                            <div class="status-cluster activity" role="group" aria-label="扫描与订单状态">
                                <span class="status-cluster-label">"活动"</span>
                                {scan_status_slot(scan_state.state, problems)}
                                <OrderElapsedSlot elapsed_ms=order_elapsed_ms problem=scalar_problem environment=environment/>
                            </div>
                            <div class="status-cluster exposure" role="group" aria-label="风险与资金状态">
                                <span class="status-cluster-label">"风险"</span>
                                <RiskSlot status=risk problem=scalar_problem on_click=Callback::new(move |_| runtime.set_active(ModuleId::Positions))/>
                                <NetDeltaSlot delta=delta problem=scalar_problem/>
                                <NextFundingSlot next=next problem=scalar_problem environment=environment/>
                            </div>
                            {problem_evidence_ledger(problems)}
                        </div>
                    </details>
                    <div class="status-summary-mode" role="group" aria-label="执行环境">
                        <ExecutionModeSlot status=trading problem=trading_problem/>
                    </div>
                </div>
            </div>
        </header>
    }
}

#[component]
fn RuntimeCategorySlots(
    operation_health: Memo<Option<VenueOperationHealthSnapshot>>,
    operation_problem: Memo<Option<ApiProblem>>,
    app_ws_channel: Memo<WsChannelState>,
    environment: Memo<Option<shared_types::ExecutionEnvironment>>,
) -> impl IntoView {
    view! {
        <>
            <MarketDataStatusSlot
                operation_health=operation_health
                operation_problem=operation_problem
            />
            {api_status_slot(operation_health, operation_problem, environment)}
            <WsStatusSlot
                operation_health=operation_health
                operation_problem=operation_problem
                environment=environment
            />
            <AppWsStatusSlot
                channel=app_ws_channel
                operation_health=operation_health
            />
        </>
    }
}

fn current_health(health: Memo<Option<SystemHealth>>) -> Option<SystemHealth> {
    health.get()
}
