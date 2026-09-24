use crate::api::ws::now_ms;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{
    AccountDataHealth, AccountFieldQuality, AccountFieldQualityStatus, ApiProblem,
    AutoProfitCloseConfig, CloseRun, PortfolioSnapshot, PortfolioSummary, PositionRow,
    RiskSnapshot, RuntimeProblem, TradingStatusResponse, VenueOperationHealth,
};
use wasm_bindgen::JsCast;

use super::super::components::snapshot_transport::{
    describe_snapshot_transport, SnapshotTransport,
};
use super::super::components::{AccountSurfaceEvidence, BalancePanelInput, SectionData};
use super::super::data::{
    has_execution_ledger_context, use_close_all_positions_action,
    use_close_run_compensation_action, use_portfolio_nav_history_state,
    use_portfolio_snapshot_state, use_position_close_action, use_positions_kill_switch_action,
    CloseAllPositionsAction, CloseRunCompensationAction, PortfolioAccountAccess,
    PortfolioNavHistoryRuntime, PositionCloseAction, PositionsKillSwitchAction, PositionsRuntime,
};
use super::derive::{
    actionable_close_runs, actionable_snapshot_problem, loaded_snapshot, snapshot_account_access,
    snapshot_section,
};
use super::snapshot::{
    balance_snapshot_section, balance_surface_evidence, position_snapshot_section,
    position_surface_evidence, snapshot_values,
};
use super::workbench::PositionsDetailTab;

#[derive(Clone, Copy)]
pub(super) struct PositionsViewModel {
    pub(super) snapshot_transport: Memo<SnapshotTransport>,
    pub(super) account_access: Memo<PortfolioAccountAccess>,
    pub(super) problems: Memo<Vec<RuntimeProblem>>,
    pub(super) operation_health: Memo<Vec<VenueOperationHealth>>,
    pub(super) load_problem: Memo<Option<ApiProblem>>,
    pub(super) summary: Memo<SectionData<Option<PortfolioSummary>>>,
    pub(super) positions: Memo<SectionData<Vec<PositionRow>>>,
    pub(super) position_values_known: Memo<bool>,
    pub(super) account_field_quality: Memo<Vec<AccountFieldQuality>>,
    pub(super) position_row_health: Memo<Vec<AccountDataHealth>>,
    pub(super) position_account_evidence: Memo<Option<AccountSurfaceEvidence>>,
    pub(super) close_action: PositionCloseAction,
    pub(super) ledger_flow_active: Memo<bool>,
    pub(super) live_account_close_ready: Memo<bool>,
    pub(super) can_manage_positions: Memo<bool>,
    pub(super) close_runs: Memo<SectionData<Vec<CloseRun>>>,
    pub(super) close_history: Memo<SectionData<Vec<CloseRun>>>,
    pub(super) compensation_action: CloseRunCompensationAction,
    pub(super) risk: Memo<SectionData<Option<RiskSnapshot>>>,
    pub(super) nav_evidence_status: Memo<Option<AccountFieldQualityStatus>>,
    pub(super) detail_tab: RwSignal<PositionsDetailTab>,
    pub(super) open_risk_details: Callback<()>,
    pub(super) open_risk_controls: Callback<()>,
    pub(super) open_settings: Callback<()>,
    pub(super) nav_history_state: PortfolioNavHistoryRuntime,
    pub(super) balance_input: BalancePanelInput,
    pub(super) position_count: Memo<usize>,
    pub(super) kill_switch_action: PositionsKillSwitchAction,
    pub(super) close_all_action: CloseAllPositionsAction,
    pub(super) can_use_portfolio_controls: Memo<bool>,
    pub(super) auto_exit_config: Memo<SectionData<Option<AutoProfitCloseConfig>>>,
    pub(super) open_risk_settings: Callback<()>,
}

pub(super) fn create_positions_view_model(
    runtime: PositionsRuntime,
    trading_status: RwSignal<LoadState<TradingStatusResponse>>,
    open_settings: Callback<()>,
    open_risk_settings: Callback<()>,
) -> PositionsViewModel {
    let refresh_nonce = RwSignal::new(0_u64);
    let snapshot_runtime = use_portfolio_snapshot_state(refresh_nonce, runtime.snapshot);
    let snapshot_state = snapshot_runtime.snapshot;
    let snapshot_transport = Memo::new(move |_| {
        snapshot_state.track();
        let snapshot_observed_at_ms = snapshot_state.with(|state| {
            loaded_snapshot(state).map(|snapshot| snapshot.server_now_ms.max(0) as u64)
        });
        describe_snapshot_transport(
            &snapshot_runtime.transport.get(),
            snapshot_runtime.source.get(),
            snapshot_runtime.poll_active.get(),
            snapshot_observed_at_ms,
            now_ms(),
        )
    });
    let nav_history_state = use_portfolio_nav_history_state(refresh_nonce, runtime.nav_history);
    let close_action = use_position_close_action(snapshot_state, trading_status);
    let kill_switch_action = use_positions_kill_switch_action(refresh_nonce);
    let close_all_action = use_close_all_positions_action(snapshot_state, trading_status);
    let compensation_action = use_close_run_compensation_action(snapshot_state);
    let summary = Memo::new(move |_| {
        snapshot_state
            .with(|state| snapshot_section(state, |snapshot| Some(snapshot.summary.clone())))
    });
    let account_access = Memo::new(move |_| snapshot_state.with(snapshot_account_access));
    let nav_evidence_status = Memo::new(move |_| {
        summary
            .get()
            .value
            .map(|summary| summary.nav_evidence.status)
    });
    let positions = Memo::new(move |_| snapshot_state.with(position_snapshot_section));
    let position_values_known =
        Memo::new(move |_| snapshot_state.with(super::snapshot::position_values_known));
    let auto_exit_config = Memo::new(move |_| match trading_status.get() {
        LoadState::Loading => SectionData::loading(),
        LoadState::Ready(status) => SectionData::ready(Some(status.risk.auto_profit_close)),
        LoadState::Stale { value, problem } => {
            SectionData::stale(Some(value.risk.auto_profit_close), &problem)
        }
        LoadState::Error(problem) => SectionData::error(&problem),
    });
    let live_account_close_ready = Memo::new(move |_| {
        trading_status.get().value().is_some_and(|status| {
            status.environment == shared_types::ExecutionEnvironment::Live
                && status.risk.live_trading_enabled
        })
    });
    let position_count = Memo::new(move |_| positions.get().value.len());
    let ledger_flow_active = Memo::new(move |_| {
        snapshot_state
            .with(|state| loaded_snapshot(state).is_some_and(has_execution_ledger_context))
    });
    let can_manage_positions = Memo::new(move |_| {
        ledger_flow_active.get() || !account_access.get().account_data_unavailable()
    });
    let can_use_portfolio_controls = Memo::new(move |_| {
        position_count.get() > 0 || !account_access.get().account_data_unavailable()
    });
    let position_account_evidence =
        Memo::new(move |_| snapshot_state.with(position_surface_evidence));
    let close_runs = Memo::new(move |_| {
        snapshot_state.with(|state| snapshot_section(state, actionable_close_runs))
    });
    let close_history = Memo::new(move |_| {
        snapshot_state
            .with(|state| snapshot_section(state, |snapshot| snapshot.recent_close_runs.clone()))
    });
    let risk = Memo::new(move |_| {
        snapshot_state.with(|state| snapshot_section(state, |snapshot| Some(snapshot.risk.clone())))
    });
    let problems = Memo::new(move |_| {
        snapshot_state.with(|state| snapshot_values(state, |row| row.problems.clone()))
    });
    let operation_health = Memo::new(move |_| {
        snapshot_state.with(|state| snapshot_values(state, |row| row.operation_health.clone()))
    });
    let account_field_quality = Memo::new(move |_| {
        snapshot_state
            .with(|state| snapshot_values(state, |row| row.account_state.field_quality.clone()))
    });
    let position_row_health = Memo::new(move |_| {
        snapshot_state.with(|state| {
            snapshot_values(state, |row| row.account_state.positions.row_health.clone())
        })
    });
    let balance_input = build_balance_panel_input(
        snapshot_state,
        operation_health,
        account_field_quality,
        account_access,
    );
    let load_problem = Memo::new(move |_| snapshot_state.with(actionable_snapshot_problem));
    let detail_tab = RwSignal::new(PositionsDetailTab::Positions);
    Effect::new(move |_| {
        if runtime.run_scope.get().is_some() {
            detail_tab.set(PositionsDetailTab::Positions);
        }
    });
    let open_risk_details = detail_panel_callback(
        detail_tab,
        PositionsDetailTab::Risk,
        "positions-tab-risk",
        "positions-detail-risk",
    );
    let open_risk_controls = detail_panel_callback(
        detail_tab,
        PositionsDetailTab::Controls,
        "positions-tab-controls",
        "positions-detail-controls",
    );

    PositionsViewModel {
        snapshot_transport,
        account_access,
        problems,
        operation_health,
        load_problem,
        summary,
        positions,
        position_values_known,
        account_field_quality,
        position_row_health,
        position_account_evidence,
        close_action,
        ledger_flow_active,
        live_account_close_ready,
        can_manage_positions,
        close_runs,
        close_history,
        compensation_action,
        risk,
        nav_evidence_status,
        detail_tab,
        open_risk_details,
        open_risk_controls,
        open_settings,
        nav_history_state,
        balance_input,
        position_count,
        kill_switch_action,
        close_all_action,
        can_use_portfolio_controls,
        auto_exit_config,
        open_risk_settings,
    }
}

fn detail_panel_callback(
    active: RwSignal<PositionsDetailTab>,
    tab: PositionsDetailTab,
    tab_id: &'static str,
    panel_id: &'static str,
) -> Callback<()> {
    Callback::new(move |()| {
        active.set(tab);
        gloo_timers::callback::Timeout::new(0, move || {
            let Some(document) = web_sys::window().and_then(|window| window.document()) else {
                return;
            };
            reveal_detail_tab(&document, tab_id);
            let Some(element) = document.get_element_by_id(panel_id) else {
                return;
            };
            element.scroll_into_view_with_bool(true);
            if let Some(element) = element.dyn_ref::<web_sys::HtmlElement>() {
                let _ = element.focus();
            }
        })
        .forget();
    })
}

fn reveal_detail_tab(document: &web_sys::Document, tab_id: &str) {
    let Some(tab) = document
        .get_element_by_id(tab_id)
        .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
    else {
        return;
    };
    let Some(tablist) = tab
        .parent_element()
        .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
    else {
        return;
    };
    let start = tab.offset_left();
    let end = start + tab.offset_width();
    let visible_start = tablist.scroll_left();
    let visible_end = visible_start + tablist.client_width();
    if start < visible_start {
        tablist.set_scroll_left(start);
    } else if end > visible_end {
        tablist.set_scroll_left(end - tablist.client_width());
    }
}

fn build_balance_panel_input(
    snapshot_state: RwSignal<LoadState<PortfolioSnapshot>>,
    operation_health: Memo<Vec<VenueOperationHealth>>,
    field_quality: Memo<Vec<AccountFieldQuality>>,
    account_access: Memo<PortfolioAccountAccess>,
) -> BalancePanelInput {
    BalancePanelInput {
        balances: Memo::new(move |_| snapshot_state.with(balance_snapshot_section)),
        asset_valuations: Memo::new(move |_| {
            snapshot_state.with(|state| {
                snapshot_values(state, |row| {
                    row.account_state.balances.asset_valuations.clone()
                })
            })
        }),
        account_summaries: Memo::new(move |_| {
            snapshot_state.with(|state| {
                snapshot_values(state, |row| {
                    row.account_state.balances.account_summaries.clone()
                })
            })
        }),
        operation_health,
        field_quality,
        row_health: Memo::new(move |_| {
            snapshot_state.with(|state| {
                snapshot_values(state, |row| row.account_state.balances.row_health.clone())
            })
        }),
        account_evidence: Memo::new(move |_| snapshot_state.with(balance_surface_evidence)),
        account_access,
    }
}
