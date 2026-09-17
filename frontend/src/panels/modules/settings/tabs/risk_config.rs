use crate::state::action_state::ActionState;
use leptos::prelude::*;

use super::super::data::{
    settings_state, use_action_runs, use_kill_switch_action, use_risk_config_save_action,
    use_trading_status,
};

#[path = "risk_config/fields.rs"]
mod fields;
#[path = "risk_config/form.rs"]
mod form;
use fields::{
    auto_pair_exit_fields, initialize_form, risk_threshold_fields, AutoProfitCloseSignals,
    RiskFormSignals, RiskThresholdSignals,
};
use form::{
    kill_switch_request, risk_action_message, risk_patch_from_inputs, status_strip, status_value,
    AutoProfitCloseInputs, RiskThresholdInputs,
};

pub(in crate::panels::modules::settings) fn risk_config_tab() -> impl IntoView {
    let refresh_nonce = RwSignal::new(0_u64);
    let status = use_trading_status(refresh_nonce);
    let action_runs = use_action_runs(refresh_nonce);
    let save_action = use_risk_config_save_action(refresh_nonce, action_runs);
    let kill_action = use_kill_switch_action(refresh_nonce, action_runs);
    let initialized = RwSignal::new(false);
    let max_order = RwSignal::new(String::new());
    let max_open = RwSignal::new(String::new());
    let imbalance_pct = RwSignal::new(String::new());
    let allowed_exchanges = RwSignal::new(String::new());
    let allowed_symbols = RwSignal::new(String::new());
    let auto_close = AutoProfitCloseSignals::new();
    let message = RwSignal::new("风控参数写入后端 RiskConfig，立即影响新订单。".to_string());

    let threshold_signals = RiskThresholdSignals {
        max_order,
        max_open,
        imbalance_pct,
        allowed_exchanges,
        allowed_symbols,
    };
    initialize_form(
        status,
        RiskFormSignals {
            initialized,
            thresholds: threshold_signals,
            auto_close,
        },
    );

    Effect::new(move |_| {
        if matches!(save_action.state.get(), ActionState::Succeeded { .. }) {
            initialized.set(false);
        }
    });

    let save = move |_| {
        if save_action.state.get_untracked().is_pending() {
            return;
        }
        let risk_inputs = RiskThresholdInputs {
            max_order: max_order.get_untracked(),
            max_open: max_open.get_untracked(),
            imbalance_pct: imbalance_pct.get_untracked(),
            allowed_exchanges: allowed_exchanges.get_untracked(),
            allowed_symbols: allowed_symbols.get_untracked(),
        };
        let auto_close_inputs = AutoProfitCloseInputs {
            enabled: auto_close.enabled.get_untracked(),
            min_net_profit_usd: auto_close.min_net_profit_usd.get_untracked(),
            min_roi_pct: auto_close.min_roi_pct.get_untracked(),
            exit_buffer_pct: auto_close.exit_buffer_pct.get_untracked(),
            stop_loss_enabled: auto_close.stop_loss_enabled.get_untracked(),
            max_net_loss_usd: auto_close.max_net_loss_usd.get_untracked(),
            max_loss_roi_pct: auto_close.max_loss_roi_pct.get_untracked(),
            liquidation_guard_enabled: auto_close.liquidation_guard_enabled.get_untracked(),
            liquidation_exit_distance_pct: auto_close.liquidation_exit_distance_pct.get_untracked(),
            confirmation_samples: auto_close.confirmation_samples.get_untracked(),
            cooldown_secs: auto_close.cooldown_secs.get_untracked(),
        };
        let patch = risk_patch_from_inputs(&risk_inputs, &auto_close_inputs);
        let patch = match patch {
            Ok(patch) => patch,
            Err(error) => {
                save_action.state.set(ActionState::Idle);
                kill_action.state.set(ActionState::Idle);
                message.set(error);
                return;
            }
        };
        kill_action.state.set(ActionState::Idle);
        save_action.submit.run(patch);
    };

    let toggle_kill = move |_| {
        if kill_action.state.get_untracked().is_pending() {
            return;
        }
        let Some(current) = status_value(status) else {
            message.set("风控状态仍在加载，不能切换 Kill Switch。".to_owned());
            return;
        };
        let active = !current.risk.kill_switch_active;
        save_action.state.set(ActionState::Idle);
        kill_action
            .submit
            .run(kill_switch_request(&current, active));
    };

    view! {
        <div class="settings-stack">
            <div class="settings-risk-scope" data-settings-risk-scope="runtime-readonly">
                <div class="settings-scope-head">
                    <strong>"运行态事实"</strong>
                    <span>"只读"</span>
                </div>
                {move || status_strip(settings_state(status))}
            </div>
            <div class="settings-risk-scope" data-settings-risk-scope="editable-thresholds">
                <div class="settings-scope-head">
                    <strong>"订单约束"</strong>
                    <span>"可编辑"</span>
                </div>
                {risk_threshold_fields(threshold_signals)}
                <div class="settings-scope-head">
                    <strong>"自动双边退出"</strong>
                    <span>"默认停用"</span>
                </div>
                {auto_pair_exit_fields(auto_close)}
                <div class="settings-actions">
                    <button
                        class="primary-blue"
                        disabled=move || save_action.state.get().is_pending()
                        on:click=save
                    >
                        {move || if save_action.state.get().is_pending() { "保存中" } else { "保存风控" }}
                    </button>
                    <em class="settings-message">{move || risk_action_message(
                        &message.get(),
                        &save_action.state.get(),
                        &ActionState::Idle,
                    )}</em>
                </div>
            </div>
            <div class="settings-risk-scope" data-settings-risk-scope="kill-switch-action">
                <div class="settings-scope-head">
                    <strong>"总闸动作"</strong>
                    <span>"独立提交"</span>
                </div>
                <div class="settings-actions">
                    <button
                        class="row-action"
                        disabled=move || kill_action.state.get().is_pending()
                        on:click=toggle_kill
                    >
                        {move || if kill_action.state.get().is_pending() { "更新中" } else { "切换 Kill Switch" }}
                    </button>
                    <em class="settings-message">{move || risk_action_message(
                        "等待总闸动作",
                        &ActionState::Idle,
                        &kill_action.state.get(),
                    )}</em>
                </div>
            </div>
        </div>
    }
}
