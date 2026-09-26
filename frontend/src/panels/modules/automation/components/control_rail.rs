use leptos::prelude::*;
use shared_types::{
    AutomatedArbitrageConfigPatch, AutomationControlAction, AutomationControlRequest,
    ExecutionEnvironment, StrategyKind, MIN_AUTOMATION_ENTRY_COOLDOWN_SECS,
    P0_EXECUTABLE_STRATEGY_KINDS,
};

use super::super::data::AutomationData;
use super::super::draft::{AutomationConfigDraft, AutomationProtectionDraft};
use super::protection_controls::{protection_controls, protection_ready};
use crate::panels::modules::strategy_kinds::strategy_option_target_index;
use crate::state::load_state::LoadState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutomationSwitchState {
    Unavailable,
    Disabled,
    Paused,
    Running,
}

pub(in crate::panels::modules::automation) fn control_rail(
    draft: AutomationConfigDraft,
    protection_draft: AutomationProtectionDraft,
    data: AutomationData,
) -> impl IntoView {
    let paper_ref = NodeRef::<leptos::html::Button>::new();
    let live_ref = NodeRef::<leptos::html::Button>::new();
    view! {
        <aside
            class=move || format!("automation-control-rail {}", command_state_class(data))
            aria-label="自动化套利控制"
        >
            <header class="workbench-rail-header">
                <div>
                    <strong>"策略控制"</strong>
                    <span>"自动入场 · 共享执行安全门"</span>
                </div>
            </header>

            <div class="automation-enable-block">
                <div class="automation-command-status">
                    <span>"当前状态"</span>
                    <strong class=move || command_state_class(data)>{move || command_state_label(data)}</strong>
                </div>
                <button
                    class=move || primary_action_class(data)
                    type="button"
                    prop:disabled=move || enable_blocked(data)
                    title=move || primary_action_title(data)
                    on:click=move |_| run_primary_action(data)
                >
                    {move || primary_action_label(data)}
                </button>
                <small>{move || command_state_detail(data)}</small>
            </div>
            {move || data.notice.get().map(|notice| view! { <p class="automation-action-notice" role="status">{notice}</p> })}

            <div class="automation-environment-block">
                <span>"执行环境"</span>
                <div
                    class="automation-segmented"
                    role="radiogroup"
                    aria-label="执行环境"
                    aria-orientation="horizontal"
                    on:keydown=move |event| {
                        let next = match event.key().as_str() {
                            "ArrowRight" | "ArrowDown" | "End" => Some(ExecutionEnvironment::Live),
                            "ArrowLeft" | "ArrowUp" | "Home" => Some(ExecutionEnvironment::Paper),
                            _ => None,
                        };
                        let Some(next) = next else { return };
                        event.prevent_default();
                        set_environment(data, next);
                        let target = if next == ExecutionEnvironment::Paper { paper_ref } else { live_ref };
                        if let Some(button) = target.get() {
                            let _ = button.focus();
                        }
                    }
                >
                    <button
                        node_ref=paper_ref
                        type="button"
                        role="radio"
                        aria-checked=move || {
                            environment_selected(data, ExecutionEnvironment::Paper).to_string()
                        }
                        tabindex=move || if environment_selected(data, ExecutionEnvironment::Paper) { 0 } else { -1 }
                        prop:disabled=move || data.busy.get() || !status_confirmed(data)
                        class=move || environment_class(data, ExecutionEnvironment::Paper)
                        on:click=move |_| set_environment(data, ExecutionEnvironment::Paper)
                    >"模拟盘"</button>
                    <button
                        node_ref=live_ref
                        type="button"
                        role="radio"
                        aria-checked=move || {
                            environment_selected(data, ExecutionEnvironment::Live).to_string()
                        }
                        tabindex=move || if environment_selected(data, ExecutionEnvironment::Live) { 0 } else { -1 }
                        prop:disabled=move || data.busy.get() || !status_confirmed(data)
                        class=move || environment_class(data, ExecutionEnvironment::Live)
                        on:click=move |_| set_environment(data, ExecutionEnvironment::Live)
                    >"实盘"</button>
                </div>
            </div>

            <details class="automation-entry-config">
                <summary>
                    <span>"入场规则"</span>
                    <strong>{move || entry_summary(data)}</strong>
                </summary>
                <fieldset class="automation-entry-config-body" disabled=move || data.busy.get() || !status_confirmed(data)
                    on:input=move |_| draft.dirty.set(true)>
                    {strategy_scope(draft)}
                    <div class="automation-rail-fields">
                        <label class="workbench-field">
                            <span>"规范币种（留空为全部）"</span>
                            <input type="text" autocomplete="off" spellcheck="false" placeholder="BTC, COTI" bind:value=draft.canonical_symbols />
                        </label>
                        <div class="automation-field-group">
                            <label class="workbench-field"><span>"资金 (USD)"</span><input type="number" min="1" step="any" bind:value=draft.capital /></label>
                            <label class="workbench-field"><span>"杠杆"</span><input type="number" min="1" max="20" step="0.5" bind:value=draft.leverage /></label>
                        </div>
                        <label class="workbench-field"><span>"最低费后净利 (%)"</span><input type="number" min="0.0001" max="100" step="0.0001" bind:value=draft.min_net /></label>
                        <label class="workbench-field"><span>"最低双腿深度 (USD)"</span><input type="number" min="1" step="100" bind:value=draft.min_depth /></label>
                        <div class="automation-field-group">
                            <label class="workbench-field"><span>"最大并发"</span><input type="number" min="1" max="8" step="1" bind:value=draft.concurrency /></label>
                            <label class="workbench-field"><span>"入场冷却 (秒)"</span><input type="number" min=MIN_AUTOMATION_ENTRY_COOLDOWN_SECS max="86400" step="1" bind:value=draft.cooldown /></label>
                        </div>
                        <button class="workbench-save" type="button" on:click=move |_| match draft.patch() {
                            Ok(patch) => data.update.run(patch), Err(problem) => data.notice.set(Some(problem)),
                        }>"保存门槛"</button>
                        <small class="automation-draft-state">{move || if draft.dirty.get() { "有未保存的入场规则；运行仍使用已保存配置" } else { "入场规则与已保存配置一致" }}</small>
                    </div>
                </fieldset>
            </details>

            {protection_controls(protection_draft, data)}

            {emergency_action(data)}

            <div class="workbench-boundary-note">
                <strong>"安全边界"</strong>
                <span>"暂停或急停只阻止新的自动入场，不等同于平仓；已有仓位仍需在持仓/风控核对退出保护与最终结果。"</span>
            </div>
        </aside>
    }
}

fn emergency_action(data: AutomationData) -> impl IntoView {
    view! {
        {move || (automation_switch_state(data) == AutomationSwitchState::Paused
            || automation_switch_state(data) == AutomationSwitchState::Running)
            .then(|| view! {
                <div class="automation-emergency-action">
                    <span><strong>"策略急停"</strong><small>"关闭后续自动入场，不代表平仓"</small></span>
                    <button
                        class="workbench-save is-danger"
                        type="button"
                        disabled=move || data.busy.get()
                        on:click=move |_| control(data, AutomationControlAction::EmergencyStop)
                    >"立即急停"</button>
                </div>
            })}
    }
}

fn strategy_scope(draft: AutomationConfigDraft) -> impl IntoView {
    let option_refs = P0_EXECUTABLE_STRATEGY_KINDS
        .iter()
        .map(|_| NodeRef::<leptos::html::Button>::new())
        .collect::<Vec<_>>();
    let option_refs_for_view = option_refs.clone();
    let option_refs = StoredValue::new(option_refs);
    view! {
        <div class="automation-strategy-scope">
            <header>
                <span>"执行策略"</span>
                <small>{move || draft.strategy_kind.get().label_zh()}</small>
            </header>
            <div
                class="automation-segmented automation-strategy-options"
                role="radiogroup"
                aria-label="自动化执行策略"
                aria-orientation="horizontal"
                on:keydown=move |event| {
                    let current = P0_EXECUTABLE_STRATEGY_KINDS
                        .iter()
                        .position(|kind| *kind == draft.strategy_kind.get())
                        .unwrap_or(0);
                    let Some(next_index) = strategy_option_target_index(
                        &event.key(),
                        current,
                        P0_EXECUTABLE_STRATEGY_KINDS.len(),
                    ) else { return };
                    event.prevent_default();
                    let Some(next) = P0_EXECUTABLE_STRATEGY_KINDS.get(next_index).copied() else { return };
                    draft.strategy_kind.set(next);
                    draft.dirty.set(true);
                    let next_ref = option_refs.with_value(|refs| refs.get(next_index).cloned());
                    if let Some(button) = next_ref.and_then(|node_ref| node_ref.get()) {
                        let _ = button.focus();
                    }
                }
            >
                {P0_EXECUTABLE_STRATEGY_KINDS.into_iter().zip(option_refs_for_view).map(|(kind, node_ref)| {
                    view! {
                        <button
                            node_ref=node_ref
                            type="button"
                            role="radio"
                            aria-checked=move || (draft.strategy_kind.get() == kind).to_string()
                            tabindex=move || if draft.strategy_kind.get() == kind { 0 } else { -1 }
                            class=move || strategy_class(draft, kind)
                            title=kind.description()
                            on:click=move |_| { draft.strategy_kind.set(kind); draft.dirty.set(true); }
                        >{kind.label_zh()}</button>
                    }
                }).collect_view()}
            </div>
        </div>
    }
}

fn strategy_class(draft: AutomationConfigDraft, kind: StrategyKind) -> &'static str {
    if draft.strategy_kind.get() == kind {
        "is-active"
    } else {
        ""
    }
}

fn run_primary_action(data: AutomationData) {
    match automation_switch_state(data) {
        AutomationSwitchState::Running => control(data, AutomationControlAction::Pause),
        AutomationSwitchState::Paused => control(data, AutomationControlAction::Resume),
        AutomationSwitchState::Disabled => data.update.run(AutomatedArbitrageConfigPatch {
            enabled: Some(true),
            paused: Some(false),
            ..AutomatedArbitrageConfigPatch::default()
        }),
        AutomationSwitchState::Unavailable => {}
    }
}

fn set_environment(data: AutomationData, environment: ExecutionEnvironment) {
    data.update.run(AutomatedArbitrageConfigPatch {
        environment: Some(environment),
        ..AutomatedArbitrageConfigPatch::default()
    });
}

fn control(data: AutomationData, action: AutomationControlAction) {
    data.control.run(AutomationControlRequest { action });
}

fn primary_action_label(data: AutomationData) -> &'static str {
    if data.busy.get() {
        return if data.mutations.get_value().journal.busy.get() || data.risk.get_value().pending() { "正在更新…" } else { "等待操作核对" };
    }
    if !status_confirmed(data) {
        return if automation_switch_state(data) == AutomationSwitchState::Running {
            "暂停新入场（状态待确认）"
        } else {
            "等待状态确认"
        };
    }
    if enable_blocked(data) && automation_switch_state(data) != AutomationSwitchState::Unavailable {
        return "先配置退出保护";
    }
    match automation_switch_state(data) {
        AutomationSwitchState::Unavailable => "等待运行状态",
        AutomationSwitchState::Disabled if live_environment(data) => "启动实盘自动化",
        AutomationSwitchState::Disabled => "启动模拟自动化",
        AutomationSwitchState::Paused if live_environment(data) => "恢复实盘自动提交",
        AutomationSwitchState::Paused => "恢复模拟自动提交",
        AutomationSwitchState::Running if live_environment(data) => "暂停实盘新入场",
        AutomationSwitchState::Running => "暂停模拟新入场",
    }
}

fn enable_blocked(data: AutomationData) -> bool {
    if data.busy.get() {
        return true;
    }
    if !status_confirmed(data) && automation_switch_state(data) != AutomationSwitchState::Running {
        return true;
    }
    match automation_switch_state(data) {
        AutomationSwitchState::Unavailable => true,
        AutomationSwitchState::Disabled | AutomationSwitchState::Paused => !protection_ready(data),
        AutomationSwitchState::Running => false,
    }
}

fn primary_action_title(data: AutomationData) -> &'static str {
    if data.busy.get() {
        "等待当前控制请求完成"
    } else if !status_confirmed(data) {
        if automation_switch_state(data) == AutomationSwitchState::Running {
            "状态未确认，仍可请求暂停新入场，不会平掉已有仓位"
        } else {
            "等待自动化状态与后台任务健康确认"
        }
    } else if enable_blocked(data) {
        "至少保存一项止盈、止损或单腿强平保护"
    } else {
        match automation_switch_state(data) {
            AutomationSwitchState::Disabled if live_environment(data) => {
                "开始监控合格机会并自动提交实盘双腿"
            }
            AutomationSwitchState::Disabled => "开始监控合格机会并生成模拟双腿结果",
            AutomationSwitchState::Paused if live_environment(data) => "恢复监控与实盘自动提交",
            AutomationSwitchState::Paused => "恢复监控与模拟自动提交",
            AutomationSwitchState::Running => "暂停新的自动入场，不会平掉已有仓位",
            AutomationSwitchState::Unavailable => "等待后端自动化运行状态",
        }
    }
}

fn primary_action_class(data: AutomationData) -> &'static str {
    if automation_switch_state(data) == AutomationSwitchState::Running {
        "workbench-save automation-primary-action is-pause"
    } else if live_environment(data) {
        "workbench-primary automation-primary-action is-live-start"
    } else {
        "workbench-primary automation-primary-action"
    }
}

fn command_state_label(data: AutomationData) -> &'static str {
    if data.busy.get() { return "操作待确认"; }
    if !status_confirmed(data) {
        return "状态待确认";
    }
    match automation_switch_state(data) {
        AutomationSwitchState::Unavailable => "运行状态不可用",
        AutomationSwitchState::Disabled => "已关闭",
        AutomationSwitchState::Paused => "已暂停",
        AutomationSwitchState::Running => "运行中",
    }
}

fn command_state_detail(data: AutomationData) -> &'static str {
    if !status_confirmed(data) {
        return "保留上次状态；新增入场需等待后台确认";
    }
    match automation_switch_state(data) {
        AutomationSwitchState::Unavailable => "后端运行状态可用后才能操作",
        AutomationSwitchState::Disabled if live_environment(data) => {
            "启动后会监控并自动提交实盘双腿"
        }
        AutomationSwitchState::Disabled => "启动后会监控并生成模拟双腿结果",
        AutomationSwitchState::Paused => "不新增运行单；已有仓位不等于已平仓",
        AutomationSwitchState::Running => "合格执行信息通过全部安全门后自动执行",
    }
}

fn command_state_class(data: AutomationData) -> &'static str {
    if !status_confirmed(data) {
        return "is-unavailable";
    }
    match automation_switch_state(data) {
        AutomationSwitchState::Unavailable => "is-unavailable",
        AutomationSwitchState::Disabled => "is-off",
        AutomationSwitchState::Paused => "is-paused",
        AutomationSwitchState::Running => "is-running",
    }
}

fn status_confirmed(data: AutomationData) -> bool {
    data.status
        .with(|state| matches!(state, LoadState::Ready(_)))
}

fn automation_switch_state(data: AutomationData) -> AutomationSwitchState {
    data.status.with(|state| {
        state
            .value()
            .map_or(AutomationSwitchState::Unavailable, |status| {
                switch_state(status.config.enabled, status.config.paused)
            })
    })
}

const fn switch_state(enabled: bool, paused: bool) -> AutomationSwitchState {
    if !enabled {
        AutomationSwitchState::Disabled
    } else if paused {
        AutomationSwitchState::Paused
    } else {
        AutomationSwitchState::Running
    }
}

fn entry_summary(data: AutomationData) -> String {
    data.status.with(|state| {
        state.value().map_or_else(
            || "读取中".to_owned(),
            |status| {
                format!(
                    "净利 ≥ {:.2}% · 深度 ≥ ${:.0}",
                    status.config.min_one_cycle_net_bps / 100.0,
                    status.config.min_depth_usd
                )
            },
        )
    })
}

fn environment_class(data: AutomationData, expected: ExecutionEnvironment) -> &'static str {
    if environment_selected(data, expected) {
        "is-active"
    } else {
        ""
    }
}

fn environment_selected(data: AutomationData, expected: ExecutionEnvironment) -> bool {
    automation_environment(data) == Some(expected)
}

fn live_environment(data: AutomationData) -> bool {
    environment_selected(data, ExecutionEnvironment::Live)
}

fn automation_environment(data: AutomationData) -> Option<ExecutionEnvironment> {
    data.status
        .with(|state| state.value().map(|status| status.config.environment))
}

#[cfg(test)]
mod tests {
    use super::{switch_state, AutomationSwitchState};

    #[test]
    fn automation_control_display_uses_one_contextual_runtime_action() {
        assert_eq!(switch_state(false, true), AutomationSwitchState::Disabled);
        assert_eq!(switch_state(true, true), AutomationSwitchState::Paused);
        assert_eq!(switch_state(true, false), AutomationSwitchState::Running);
    }
}
