use crate::api::ws::WsChannelState;
use crate::panels::nav::view::ModuleNav;
use crate::panels::status_bar::data::{
    use_scan_status_state, use_system_health_state, use_venue_operation_health_state,
};
use crate::panels::status_bar::problem_ledger::problem_evidence_ledger;
use crate::panels::status_bar::slots::{
    api_status_slot, scan_status_slot, AppWsStatusSlot, ExecutionModeSlot, MarketDataStatusSlot,
    NetDeltaSlot, NextFundingSlot, OrderElapsedSlot, RiskSlot, WsStatusSlot,
};
use crate::panels::status_bar::summary::summarize_system_health;
use crate::panels::workstation::{ModuleId, WorkspaceRuntime};
use leptos::prelude::*;
use shared_types::{ApiProblem, SystemHealth, VenueOperationHealthSnapshot};

#[component]
pub(in crate::panels) fn TopStatusBar(runtime: WorkspaceRuntime) -> impl IntoView {
    let health_state = use_system_health_state();
    let operation_health_state = use_venue_operation_health_state();
    let trading_state = runtime.trading_status();
    let scan_state = use_scan_status_state();
    let health = Memo::new(move |_| health_state.state.get().value().cloned());
    let operation_health = Memo::new(move |_| operation_health_state.state.get().value().cloned());
    let trading = Memo::new(move |_| trading_state.get().value().cloned());
    let environment = Memo::new(move |_| trading.get().map(|status| status.environment));
    let order_elapsed_ms =
        Memo::new(move |_| current_health(health).and_then(|h| h.order_elapsed_ms));
    let risk = Memo::new(move |_| current_health(health).map(|h| h.risk));
    let scalar_problem = Memo::new(move |_| {
        current_health(health)
            .is_none()
            .then(|| health_state.state.get().problem().cloned())
            .flatten()
    });
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
    let next = Memo::new(move |_| current_health(health).and_then(|h| h.next_funding));
    let health_summary = Memo::new(move |_| {
        let snapshot = current_health(health);
        summarize_system_health(snapshot.as_ref())
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
                                <span class="status-summary-action-closed">"查看证据"</span>
                                <span class="status-summary-action-open">"收起证据"</span>
                            </span>
                        </summary>
                        <div class="top-slots status-details-panel" role="group" aria-label="系统状态详情">
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
