use crate::state::load_state::LoadState;
use leptos::prelude::*;

use super::super::data::AutomationData;
use super::super::draft::AutomationProtectionDraft;
use super::super::protection_calibration::protection_capital_ready;

pub(in crate::panels::modules::automation) fn protection_controls(
    draft: AutomationProtectionDraft,
    data: AutomationData,
) -> impl IntoView {
    view! {
        <details class="automation-protection" open=move || !protection_ready(data)>
            <summary>
                <span>"退出保护"</span>
                <strong>{move || protection_label(data)}</strong>
            </summary>
            <div class="automation-protection-body">
                {capital_calibration(draft, data)}
                {guard_fields(
                    "止盈",
                    "净利润与收益率同时达标后平双边",
                    draft.take_profit,
                    ("最低净利润 USD", draft.min_profit_usd, "0.01"),
                    ("最低收益 (%)", draft.min_profit_bps, "0.0001"),
                )}
                {guard_fields(
                    "止损",
                    "净亏损或亏损率任一达标后平双边",
                    draft.stop_loss,
                    ("最大净亏损 USD", draft.max_loss_usd, "0.01"),
                    ("最大亏损 (%)", draft.max_loss_bps, "0.0001"),
                )}
                <section class="automation-protection-group">
                    <label class="automation-protection-toggle">
                        <span><strong>"单腿强平保护"</strong><small>"任一腿接近交易所强平价时平双边"</small></span>
                        <input type="checkbox" bind:checked=draft.liquidation_guard />
                    </label>
                    <label class="workbench-field">
                        <span>"退出距离 %"</span>
                        <input
                            type="text"
                            inputmode="decimal"
                            autocomplete="off"
                            spellcheck="false"
                            placeholder="0.1"
                            bind:value=draft.liquidation_distance_pct
                        />
                    </label>
                </section>
                <button class="workbench-save" type="button" on:click=move |_| save(draft, data)>
                    "保存退出保护"
                </button>
                <p class="automation-protection-message">
                    {move || data.protection_notice.get().unwrap_or_else(|| "至少启用一项后，自动入场才可启动。".to_owned())}
                </p>
            </div>
        </details>
    }
}

fn capital_calibration(draft: AutomationProtectionDraft, data: AutomationData) -> impl IntoView {
    view! {
        <section class=move || capital_calibration_class(draft, data)>
            <div>
                <span>
                    <strong>{move || capital_label(data)}</strong>
                    <small>{move || capital_assessment_message(draft, data)}</small>
                </span>
                <button
                    type="button"
                    prop:disabled=move || automation_capital_usd(data).is_none()
                    title="仅更新本页草稿，不会保存或恢复实盘"
                    on:click=move |_| apply_small_live_preset(draft, data)
                >"应用推荐组合"</button>
            </div>
            <p>"按当前资金填值并勾选三项保护；只更新草稿，保存后也不会自动恢复实盘。"</p>
        </section>
    }
}

fn guard_fields(
    title: &'static str,
    detail: &'static str,
    enabled: RwSignal<bool>,
    first: (&'static str, RwSignal<String>, &'static str),
    second: (&'static str, RwSignal<String>, &'static str),
) -> impl IntoView {
    view! {
        <section class="automation-protection-group">
            <label class="automation-protection-toggle">
                <span><strong>{title}</strong><small>{detail}</small></span>
                <input type="checkbox" bind:checked=enabled />
            </label>
            <div class="automation-protection-fields">
                {decimal_field(first)}
                {decimal_field(second)}
            </div>
        </section>
    }
}

fn decimal_field(field: (&'static str, RwSignal<String>, &'static str)) -> impl IntoView {
    view! {
        <label class="workbench-field">
            <span>{field.0}</span>
            <input
                type="text"
                inputmode="decimal"
                autocomplete="off"
                spellcheck="false"
                placeholder=field.2
                bind:value=field.1
            />
        </label>
    }
}

fn save(draft: AutomationProtectionDraft, data: AutomationData) {
    match draft.patch() {
        Ok(patch) => data.update_protection.run(patch),
        Err(message) => data.protection_notice.set(Some(message)),
    }
}

fn apply_small_live_preset(draft: AutomationProtectionDraft, data: AutomationData) {
    let Some(capital_usd) = automation_capital_usd(data) else {
        data.protection_notice
            .set(Some("运行资金尚未读取，无法生成建议值".to_owned()));
        return;
    };
    draft.apply_small_live_preset(capital_usd);
    data.protection_notice.set(Some(format!(
        "已按 ${capital_usd:.2} 填入并勾选三项保护；尚未保存，自动化仍暂停"
    )));
}

pub(super) fn automation_capital_usd(data: AutomationData) -> Option<f64> {
    data.status.with(|state| {
        state.value().and_then(|status| {
            let capital_usd = status.config.capital_usd;
            (capital_usd.is_finite() && capital_usd > 0.0).then_some(capital_usd)
        })
    })
}

fn capital_label(data: AutomationData) -> String {
    automation_capital_usd(data).map_or_else(
        || "等待运行资金".to_owned(),
        |capital_usd| format!("${capital_usd:.2} 小额实盘校准"),
    )
}

fn capital_assessment_message(draft: AutomationProtectionDraft, data: AutomationData) -> String {
    automation_capital_usd(data)
        .and_then(|capital_usd| draft.capital_assessment(capital_usd))
        .map_or_else(
            || "输入有效的止盈与止损金额后显示资金占比".to_owned(),
            |assessment| assessment.message,
        )
}

fn capital_calibration_class(draft: AutomationProtectionDraft, data: AutomationData) -> String {
    let state = automation_capital_usd(data)
        .and_then(|capital_usd| draft.capital_assessment(capital_usd))
        .map_or("", |assessment| {
            if assessment.needs_calibration {
                " is-warning"
            } else {
                " is-calibrated"
            }
        });
    format!("automation-protection-calibration{state}")
}

pub(super) fn protection_ready(data: AutomationData) -> bool {
    let Some(capital_usd) = automation_capital_usd(data) else {
        return false;
    };
    data.protection.with(|state| {
        state
            .value()
            .is_some_and(|config| protection_capital_ready(config, capital_usd))
    })
}

pub(super) fn protection_label(data: AutomationData) -> String {
    match data.protection.get() {
        LoadState::Loading => "读取中".to_owned(),
        LoadState::Error(_) => "读取失败".to_owned(),
        LoadState::Ready(config) | LoadState::Stale { value: config, .. } => {
            let count = [
                config.enabled,
                config.stop_loss_enabled,
                config.liquidation_guard_enabled,
            ]
            .into_iter()
            .filter(|enabled| *enabled)
            .count();
            if count == 0 {
                "必须配置".to_owned()
            } else if automation_capital_usd(data)
                .is_none_or(|capital_usd| !protection_capital_ready(&config, capital_usd))
            {
                "需要校准".to_owned()
            } else {
                format!("已配置 {count}/3")
            }
        }
    }
}
