use crate::state::action_state::ActionState;
use crate::state::module_runtime::ModuleRuntimeState;
use super::super::runtime::{action_health, PaneState};
use crate::state::{load_state::LoadState, trading_status::TradingStatusState};
use leptos::prelude::*;

use super::super::data::{settings_recovery_panel, settings_state};

#[path = "risk_config/fields.rs"]
mod fields;
#[path = "risk_config/form.rs"]
mod form;
mod runtime;
use fields::{auto_pair_exit_fields, risk_threshold_fields};
use form::{kill_switch_request, risk_action_message, status_strip, status_value};
pub(in crate::panels) use runtime::{create_risk_config_runtime, RiskConfigRuntime};

pub(in crate::panels::modules::settings) fn risk_config_tab(
    runtime: RiskConfigRuntime,
    pane: PaneState,
) -> impl IntoView {
    let status = expect_context::<TradingStatusState>().state;
    let save_action = runtime.save;
    let kill_action = runtime.kill;
    pane.track(move || ModuleRuntimeState::combine([
        ModuleRuntimeState::from_load_state(&status.get()),
        action_health(save_action.journal, &save_action.state.get()),
        action_health(kill_action.journal, &kill_action.state.get()),
    ]));
    let message = runtime.message;

    let blocked = Memo::new(move |_| {
        !runtime.form.initialized.get()
            || !matches!(status.get(), LoadState::Ready(_))
            || save_action.state.get().is_pending()
            || kill_action.state.get().is_pending()
    });

    let save = move |_| {
        if blocked.get_untracked() || runtime.unresolved() {
            return;
        }
        let Some(current) = status_value(status) else {
            return;
        };
        let patch = match runtime.patch(&current.risk) {
            Ok(Some(patch)) => patch,
            Ok(None) => {
                runtime.apply(&current.risk);
                save_action.state.set(ActionState::Idle);
                message.set("当前配置一致，无需重复保存。".into());
                return;
            }
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
        if blocked.get_untracked() || runtime.unresolved() {
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
            {settings_recovery_panel(save_action.journal, runtime.recheck)}
            <div class="settings-actions"><button class="row-action" disabled=move || runtime.pending()
                on:click=move |_| runtime.refresh.run(())>"刷新风控状态"</button></div>
            <div class="settings-risk-scope" data-settings-risk-scope="runtime-readonly">
                <div class="settings-scope-head">
                    <strong>"运行状态事实"</strong>
                    <span>"只读"</span>
                </div>
                {move || status_strip(settings_state(status))}
            </div>
            <div class="settings-risk-scope" data-settings-risk-scope="editable-thresholds">
            <fieldset class="settings-risk-editor" disabled=move || blocked.get() || runtime.unresolved()
                on:input=move |_| runtime.edit() on:change=move |_| runtime.edit()>
                <div class="settings-scope-head">
                    <strong>"订单约束"</strong>
                    <span>{move || if runtime.pending() { "提交中" }
                        else if runtime.unresolved() { "处理结果待确认" }
                        else if runtime.dirty.get() { "未保存" }
                        else if matches!(status.get(), LoadState::Ready(_)) { "与后台一致" }
                        else { "状态待确认" }}</span>
                </div>
                {risk_threshold_fields(runtime.form.thresholds)}
                <div class="settings-scope-head">
                    <strong>"自动双边退出"</strong>
                    <span>"保存后生效"</span>
                </div>
                {auto_pair_exit_fields(runtime.form.auto_close)}
            </fieldset>
                <div class="settings-actions settings-risk-save-actions">
                    <button
                        class="primary-blue"
                        disabled=move || blocked.get() || runtime.unresolved()
                        on:click=save
                    >
                        {move || if save_action.state.get().is_pending() { "保存中" } else { "保存风控" }}
                    </button>
                    <button class="row-action" disabled=move || blocked.get() || runtime.unresolved() on:click=move |_| {
                        if blocked.get_untracked() || runtime.unresolved() { return; }
                        if let Some(current) = status_value(status) {
                            runtime.apply(&current.risk);
                            save_action.state.set(ActionState::Idle);
                            message.set("已载入当前后端配置。".into());
                        }
                    }>"载入最新配置"</button>
                    <em class="settings-message">{move || risk_action_message(
                        &message.get(),
                        &save_action.state.get(),
                        &ActionState::Idle,
                    )}</em>
                </div>
                <Show when=move || runtime.save_unresolved()>
                    <em class="settings-message is-error">"原保存结果尚未确认，请核对原处理结果；不会重新保存。"</em>
                </Show>
            </div>
            <div class="settings-risk-scope" data-settings-risk-scope="kill-switch-action">
                <div class="settings-scope-head">
                    <strong>"总闸动作"</strong>
                    <span>"独立提交"</span>
                </div>
                <div class="settings-actions">
                    <button
                        class="row-action"
                        disabled=move || blocked.get() || runtime.unresolved()
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
                <Show when=move || runtime.kill_unresolved()>
                    <em class="settings-message is-error">"原总闸动作尚未确认，请核对原处理结果；不会反向切换。"</em>
                </Show>
            </div>
        </div>
    }
}
