use crate::panels::shared::Surface;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::TradingStatusResponse;

mod derive;
#[path = "view/model.rs"]
mod model;
#[path = "view/snapshot.rs"]
mod snapshot;
#[path = "view/workbench.rs"]
mod workbench;
use super::components::{
    account_setup_prompt, balance_panel, close_history_panel, close_runs_panel, kill_switch_bar,
    nav_breakdown_panel, nav_history_panel, pair_protection_bar, positions_table, risk_panel,
    risk_summary_panel, runtime_problems_banner, summary_cards, PositionTableEvidence,
    PositionTableRuntime,
};
use super::data::PositionsRuntime;
#[cfg(test)]
use derive::{actionable_snapshot_problem, snapshot_section};
use derive::{
    completed_previous_close_summary, position_action_evidence, position_action_message,
    should_render_close_runs_surface,
};
use model::create_positions_view_model;
#[cfg(test)]
use snapshot::position_envelope_section;
#[cfg(test)]
use snapshot::{balance_snapshot_section, position_snapshot_section, position_surface_evidence};
use workbench::{positions_command_header, positions_detail_tabs, PositionsDetailTab};

pub(in crate::panels) fn positions_module(
    runtime: PositionsRuntime,
    trading_status: RwSignal<LoadState<TradingStatusResponse>>,
    open_settings: Callback<()>,
    open_risk_settings: Callback<()>,
) -> impl IntoView {
    let model =
        create_positions_view_model(runtime, trading_status, open_settings, open_risk_settings);

    view! {
        <section class="module-page positions-layout">
            {positions_command_header(
                model.snapshot_transport,
                model.account_access,
            )}
            {runtime_problems_banner(
                model.problems,
                model.operation_health,
                model.load_problem,
                model.account_access,
            )}
            {summary_cards(model.summary, model.account_access, model.position_values_known)}
            <div class="positions-risk-command" aria-label="当前风险边界">
                <Surface title="风险边界" meta="最坏项优先" class_name="positions-risk-summary">
                    {risk_summary_panel(
                        model.risk,
                        model.positions,
                        model.position_values_known,
                        model.nav_evidence_status,
                        model.account_access,
                        model.open_risk_details,
                        model.open_risk_controls,
                    )}
                </Surface>
            </div>
            <section class="positions-detail-workspace" aria-label="持仓与账户风险任务">
                {positions_detail_tabs(model.detail_tab)}
                <div
                    id="positions-detail-positions"
                    class="positions-detail-panel positions-positions-panel"
                    role="tabpanel"
                    aria-labelledby="positions-tab-positions"
                    hidden=move || model.detail_tab.get() != PositionsDetailTab::Positions
                >
                    <div class="positions-primary-workspace">
                        <Show when=move || should_render_close_runs_surface(&model.close_runs.get())>
                            <div class="positions-incident-notice" role="status">
                                <span>{move || format!("{} 笔平仓待处理", model.close_runs.get().value.len())}</span>
                                <button type="button" class="row-action" on:click=move |_| model.detail_tab.set(PositionsDetailTab::Activity)>"查看处理"</button>
                            </div>
                        </Show>
                        <Surface title="持仓" meta="按强平距离升序" class_name="positions-main">
                            {position_action_status(model.close_action, model.can_manage_positions)}
                            {positions_table(
                                model.positions,
                                PositionTableEvidence::new(
                                    model.account_field_quality,
                                    model.position_row_health,
                                    model.position_account_evidence,
                                ),
                                PositionTableRuntime::new(
                                    model.close_action,
                                    model.account_access,
                                    model.ledger_flow_active,
                                    model.live_account_close_ready,
                                ),
                            )}
                        </Surface>
                    </div>
                </div>
                <div
                    id="positions-detail-accounts"
                    class="positions-detail-panel positions-account-panel"
                    role="tabpanel"
                    aria-labelledby="positions-tab-accounts"
                    hidden=move || model.detail_tab.get() != PositionsDetailTab::Accounts
                >
                    {account_setup_prompt(
                        model.account_access,
                        model.ledger_flow_active,
                        model.open_settings,
                    )}
                </div>
                <div
                    id="positions-detail-balances"
                    class="positions-detail-panel positions-balance-panel"
                    role="tabpanel"
                    aria-labelledby="positions-tab-balances"
                    hidden=move || model.detail_tab.get() != PositionsDetailTab::Balances
                >
                    {nav_breakdown_panel(model.summary, model.account_access)}
                    <div class="positions-balance-nav-grid">
                        <section class="positions-detail-region positions-balance-region">
                            <header><h3>"账户资产"</h3><span>"余额、可用与占用"</span></header>
                            {balance_panel(model.balance_input)}
                        </section>
                        <section class="positions-detail-region positions-nav-region">
                            <header>
                                <h3>"NAV 历史"</h3>
                                <button
                                    type="button" class="icon-button"
                                    title="刷新净值历史" aria-label="刷新净值历史"
                                    disabled=move || model.nav_history_state.refreshing.get()
                                    on:click=move |_| model.nav_history_state.refresh.run(())
                                >"↻"</button>
                            </header>
                            {nav_history_panel(model.nav_history_state.state, model.account_access)}
                        </section>
                    </div>
                </div>
                <div
                    id="positions-detail-risk"
                    class="positions-detail-panel positions-risk-detail"
                    role="tabpanel"
                    aria-labelledby="positions-tab-risk"
                    tabindex="-1"
                    hidden=move || model.detail_tab.get() != PositionsDetailTab::Risk
                >
                    {risk_panel(
                        model.risk,
                        model.account_field_quality,
                        model.nav_evidence_status,
                        model.account_access,
                            model.position_values_known,
                            model.positions,
                        )}
                </div>
                <div
                    id="positions-detail-activity"
                    class="positions-detail-panel positions-activity-panel"
                    role="tabpanel"
                    aria-labelledby="positions-tab-activity"
                    hidden=move || model.detail_tab.get() != PositionsDetailTab::Activity
                >
                    <Show when=move || should_render_close_runs_surface(&model.close_runs.get())>
                        {close_runs_panel(model.close_runs, model.compensation_action)}
                    </Show>
                    {close_history_panel(model.close_history)}
                </div>
                <div
                    id="positions-detail-controls"
                    class="positions-detail-panel positions-controls-panel"
                    role="tabpanel"
                    aria-labelledby="positions-tab-controls"
                    tabindex="-1"
                    hidden=move || model.detail_tab.get() != PositionsDetailTab::Controls
                >
                    <div class="positions-advanced-grid">
                        {kill_switch_bar(
                            model.risk,
                            model.position_count,
                            model.positions,
                            model.kill_switch_action,
                            model.close_all_action,
                            model.can_use_portfolio_controls,
                        )}
                        {pair_protection_bar(
                            model.auto_exit_config,
                            model.positions,
                            model.open_risk_settings,
                        )}
                    </div>
                </div>
            </section>
        </section>
    }
}

fn position_action_status(
    action: super::data::PositionCloseAction,
    can_manage_positions: Memo<bool>,
) -> impl IntoView {
    move || {
        if !can_manage_positions.get() {
            return ().into_any();
        }
        let state = action.state.get();
        if completed_previous_close_summary(&state).is_some() {
            return ().into_any();
        }
        let message = position_action_message(&state);
        if message.is_empty() {
            return ().into_any();
        }
        let evidence = position_action_evidence(&state);
        view! {
            <div class="positions-action-status" role="status">
                <em class="positions-action-message table-message">
                    {message}
                </em>
                {evidence.map(|evidence| view! {
                    <details class="positions-action-evidence">
                        <summary>"查看证据"</summary>
                        <span>{evidence}</span>
                    </details>
                })}
            </div>
        }
        .into_any()
    }
}

#[cfg(test)]
#[path = "view/tests.rs"]
mod tests;
