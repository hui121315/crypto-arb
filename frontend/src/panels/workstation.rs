//! CROSSLINE Omni workstation shell.

use crate::panels::modules::{
    automation_module, create_automation_runtime, create_execution_runtime, create_futures_runtime,
    create_gate_crossex_runtime, create_onchain_runtime, create_opportunities_runtime,
    create_positions_runtime, create_review_runtime, execution_module, futures_module,
    gate_crossex_module, onchain_module, opportunities_module, positions_module, review_module,
    select_credentials_tab, select_risk_tab, settings_module, AutomationRuntime, ExecutionRuntime,
    FuturesRuntime, GateCrossExRuntime, OnchainRuntime, OpportunitiesRuntime, PositionsRuntime,
    ReviewRuntime,
};
use crate::panels::routing::{
    bind_workspace_route_listener, initial_workspace_route, sync_module_hash, WorkspaceRoute,
};
use crate::panels::modules::{create_stocks_runtime, stocks_module, StocksRuntime};
use crate::panels::status_bar::data::use_trading_status_state;
use crate::panels::status_bar::view::TopStatusBar;
use crate::state::load_state::LoadState;
use crate::state::module_runtime::{ModuleRuntimeState, ModuleRuntimeStatus};
use crate::state::{push_toast_to, use_toasts, ToastLevel};
use leptos::prelude::*;
use shared_types::{ApiProblem, TradingStatusResponse};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleId {
    Positions,
    Futures,
    Opportunities,
    GateCrossEx,
    Onchain,
    Stocks,
    Automation,
    Execution,
    Review,
    Settings,
}

#[derive(Clone, Copy)]
pub struct WorkspaceRuntime {
    active: RwSignal<ModuleId>,
    futures_runtime: FuturesRuntime,
    opportunities_runtime: OpportunitiesRuntime,
    gate_crossex_runtime: GateCrossExRuntime,
    onchain_runtime: OnchainRuntime,
    stocks_runtime: StocksRuntime,
    automation_runtime: AutomationRuntime,
    positions_runtime: PositionsRuntime,
    review_runtime: ReviewRuntime,
    execution_runtime: ExecutionRuntime,
    trading_status: RwSignal<LoadState<TradingStatusResponse>>,
}

impl WorkspaceRuntime {
    fn new(
        route: &WorkspaceRoute,
        trading_status: RwSignal<LoadState<TradingStatusResponse>>,
    ) -> Self {
        let opportunities_runtime = create_opportunities_runtime();
        let runtime = Self {
            active: RwSignal::new(route.module),
            futures_runtime: create_futures_runtime(opportunities_runtime),
            opportunities_runtime,
            gate_crossex_runtime: create_gate_crossex_runtime(),
            onchain_runtime: create_onchain_runtime(),
            stocks_runtime: create_stocks_runtime(),
            automation_runtime: create_automation_runtime(),
            positions_runtime: create_positions_runtime(),
            review_runtime: create_review_runtime(),
            execution_runtime: create_execution_runtime(),
            trading_status,
        };
        runtime.apply_workspace_route(route);
        runtime
    }

    pub(in crate::panels) const fn active(self) -> RwSignal<ModuleId> {
        self.active
    }

    pub(in crate::panels) const fn trading_status(
        self,
    ) -> RwSignal<LoadState<TradingStatusResponse>> {
        self.trading_status
    }

    pub(in crate::panels) fn set_active(self, module: ModuleId) {
        self.active.set(module);
    }

    fn apply_workspace_route(self, route: &WorkspaceRoute) {
        if self.active.get_untracked() != route.module {
            self.active.set(route.module);
        }
        self.opportunities_runtime.apply_workspace_route(route);
        self.futures_runtime.apply_workspace_route(route);
        self.execution_runtime.apply_workspace_route(route);
    }

    pub(in crate::panels) fn module_runtime_state(self, module: ModuleId) -> ModuleRuntimeState {
        match module {
            ModuleId::Positions => self.positions_runtime.module_runtime_state(),
            ModuleId::Futures => self.futures_runtime.module_runtime_state(),
            ModuleId::Opportunities => self.opportunities_runtime.module_runtime_state(),
            ModuleId::GateCrossEx => self.gate_crossex_runtime.module_runtime_state(),
            ModuleId::Onchain => self.onchain_runtime.module_runtime_state(),
            ModuleId::Stocks => self.stocks_runtime.module_runtime_state(),
            ModuleId::Automation => self.automation_runtime.module_runtime_state(),
            ModuleId::Execution => self.execution_runtime.module_runtime_state(),
            ModuleId::Review => self.review_runtime.module_runtime_state(),
            ModuleId::Settings => ModuleRuntimeState::ready(),
        }
    }
}

#[component]
pub fn OmniWorkstation() -> impl IntoView {
    let route = initial_workspace_route();
    let trading_status = use_trading_status_state();
    let runtime = WorkspaceRuntime::new(&route, trading_status);
    let active_module = runtime.active();

    bind_workspace_route_listener(active_module, move |route| {
        runtime.apply_workspace_route(&route)
    });
    Effect::new(move |previous: Option<ModuleId>| {
        let module = active_module.get();
        sync_module_hash(module);
        if previous.is_some_and(|previous| previous != module) {
            scroll_workspace_to_top();
        }
        module
    });
    bind_runtime_problem_toasts(runtime);

    view! {
        <div class="mod-shell">
            <TopStatusBar runtime=runtime/>
            <div class="mod-layout trade-layout">
                <main class="mod-content">
                    {move || render_module(active_module.get(), runtime)}
                </main>
            </div>
        </div>
    }
}

fn scroll_workspace_to_top() {
    if let Some(window) = web_sys::window() {
        window.scroll_to_with_x_and_y(0.0, 0.0);
    }
}

fn render_module(module: ModuleId, runtime: WorkspaceRuntime) -> AnyView {
    match module {
        ModuleId::Positions => positions_module(
            runtime.positions_runtime,
            runtime.trading_status,
            Callback::new(move |_| {
                select_credentials_tab();
                runtime.set_active(ModuleId::Settings);
            }),
            Callback::new(move |_| {
                select_risk_tab();
                runtime.set_active(ModuleId::Settings);
            }),
        )
        .into_any(),
        ModuleId::Opportunities => opportunities_module(
            runtime.opportunities_runtime,
            runtime.execution_runtime,
            runtime.active,
        )
        .into_any(),
        ModuleId::GateCrossEx => gate_crossex_module(runtime.gate_crossex_runtime).into_any(),
        ModuleId::Onchain => onchain_module(runtime.onchain_runtime).into_any(),
        ModuleId::Stocks => stocks_module(runtime.stocks_runtime).into_any(),
        ModuleId::Automation => automation_module(runtime.automation_runtime).into_any(),
        ModuleId::Futures => futures_module(
            runtime.futures_runtime,
            runtime.execution_runtime,
            runtime.active,
        )
        .into_any(),
        ModuleId::Execution => execution_module(runtime.execution_runtime).into_any(),
        ModuleId::Review => review_module(runtime.review_runtime).into_any(),
        ModuleId::Settings => settings_module(runtime.execution_runtime).into_any(),
    }
}

fn bind_runtime_problem_toasts(runtime: WorkspaceRuntime) {
    let toasts = use_toasts();
    let previous = RwSignal::new(None::<(ModuleId, ModuleRuntimeStatus, String)>);
    Effect::new(move |_| {
        let module = runtime.active.get();
        let state = runtime.module_runtime_state(module);
        let next = state
            .problem
            .as_ref()
            .map(|problem| (module, state.status, problem_fingerprint(problem)));
        if next == previous.get_untracked() {
            return;
        }
        previous.set(next);
        if state.status != ModuleRuntimeStatus::Error {
            return;
        }
        if let Some(problem) = state.problem {
            push_toast_to(
                toasts,
                ToastLevel::Error,
                module_problem_message(module, &problem),
            );
        }
    });
}

fn problem_fingerprint(problem: &ApiProblem) -> String {
    format!(
        "{}:{}:{}",
        problem.code,
        problem.source.as_deref().unwrap_or_default(),
        problem.status.unwrap_or_default()
    )
}

fn module_problem_message(module: ModuleId, problem: &ApiProblem) -> String {
    let mut evidence = vec![format!("code={}", problem.code)];
    if let Some(status) = problem.status {
        evidence.push(format!("status={status}"));
    }
    if let Some(source) = problem.source.as_deref() {
        evidence.push(format!("source={source}"));
    }
    let message = match (module, problem.code.as_str()) {
        (
            ModuleId::Futures,
            shared_types::problem::codes::OPPORTUNITY_SNAPSHOT_STALE | "OPPORTUNITY_ENVELOPE_STALE",
        ) => "候选快照已过期，正在等待下一轮扫描",
        _ => problem.message.as_str(),
    };
    format!(
        "{}：{}（{}）",
        module.title(),
        message,
        evidence.join(" · ")
    )
}
