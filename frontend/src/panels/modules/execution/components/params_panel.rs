use crate::panels::modules::rate_format::{bps_input_as_percent, percent_input_as_bps};
use crate::state::load_state::LoadState;
use leptos::prelude::*;

use super::super::data::ExecutionPreview;
use super::super::draft::ExecutionDraft;
use super::super::selection::ExecutionSelection;

#[path = "params_panel/capability.rs"]
mod capability;
use capability::{
    capability_hint_for_state, disabled_margin_modes_for_state, disabled_order_types_for_state,
    disabled_time_in_force_for_state, first_enabled_option, option_disabled,
};

const TIME_IN_FORCE_OPTIONS: &[&str] = &["IOC", "FOK", "GTC", "GTX"];
const ORDER_TYPE_OPTIONS: &[&str] = &["Limit", "Market", "Post-only"];
const MARGIN_MODE_OPTIONS: &[&str] = &["Cross", "Isolated"];

pub(in crate::panels::modules::execution) fn params_panel(
    selection: Memo<ExecutionSelection>,
    draft: ExecutionDraft,
    preview_state: RwSignal<LoadState<ExecutionPreview>>,
) -> impl IntoView {
    let disabled_tif = Memo::new(move |_| preview_state.with(disabled_time_in_force_for_state));
    let disabled_order_types =
        Memo::new(move |_| preview_state.with(disabled_order_types_for_state));
    let disabled_margin_modes =
        Memo::new(move |_| preview_state.with(disabled_margin_modes_for_state));
    sync_blocked_select(draft.time_in_force, disabled_tif, TIME_IN_FORCE_OPTIONS);
    sync_blocked_select(draft.order_type, disabled_order_types, ORDER_TYPE_OPTIONS);
    sync_blocked_select(
        draft.margin_mode,
        disabled_margin_modes,
        MARGIN_MODE_OPTIONS,
    );
    view! {
        <section class="execution-section params-section">
            <div class="execution-section-head">
                <div>
                    <span>"执行参数"</span>
                    <strong>{move || format!("{} 草案", selection.get().pair)}</strong>
                </div>
                <em>{move || preview_state.with(capability_hint_for_state)}</em>
            </div>
            <div class="execution-param-grid">
                <InputParam label="计划本金 USD" value=draft.capital_usd/>
                <InputParam label="杠杆" value=draft.leverage/>
                <SelectParam
                    label="保证金模式"
                    value=draft.margin_mode
                    options=MARGIN_MODE_OPTIONS
                    disabled_options=disabled_margin_modes
                />
                <SelectParam
                    label="订单有效期"
                    value=draft.time_in_force
                    options=TIME_IN_FORCE_OPTIONS
                    disabled_options=disabled_tif
                />
                <SelectParam
                    label="订单类型"
                    value=draft.order_type
                    options=ORDER_TYPE_OPTIONS
                    disabled_options=disabled_order_types
                />
                <PercentParam label="保护偏移 %" value=draft.limit_offset_bps/>
            </div>
        </section>
    }
}

#[component]
fn InputParam(#[prop(into)] label: String, value: RwSignal<String>) -> impl IntoView {
    view! {
        <label class="param-field">
            <span>{label}</span>
            <input
                inputmode="decimal"
                prop:value=move || value.get()
                on:input=move |ev| value.set(event_target_value(&ev))
            />
        </label>
    }
}

#[component]
fn PercentParam(#[prop(into)] label: String, value: RwSignal<String>) -> impl IntoView {
    let editing = RwSignal::new(false);
    let display = RwSignal::new(bps_input_as_percent(&value.get_untracked()));
    Effect::new(move |_| {
        let next = bps_input_as_percent(&value.get());
        if !editing.get_untracked() {
            display.set(next);
        }
    });
    view! {
        <label class="param-field">
            <span>{label}</span>
            <input
                inputmode="decimal"
                prop:value=move || display.get()
                on:focus=move |_| editing.set(true)
                on:input=move |ev| {
                    let raw = event_target_value(&ev);
                    display.set(raw.clone());
                    value.set(percent_input_as_bps(&raw));
                }
                on:blur=move |_| {
                    editing.set(false);
                    display.set(bps_input_as_percent(&value.get_untracked()));
                }
            />
        </label>
    }
}

#[component]
fn SelectParam(
    #[prop(into)] label: String,
    value: RwSignal<String>,
    options: &'static [&'static str],
    disabled_options: Memo<Vec<String>>,
) -> impl IntoView {
    view! {
        <label class="param-field">
            <span>{label}</span>
            <select
                prop:value=move || value.get()
                on:change=move |ev| value.set(event_target_value(&ev))
            >
                {options.iter().map(|option| view! {
                    <option
                        value=*option
                        disabled=move || option_disabled(option, &disabled_options.get())
                    >
                        {option_label(option)}
                    </option>
                }).collect_view()}
            </select>
        </label>
    }
}

fn option_label(option: &str) -> &str {
    match option {
        "Limit" => "限价",
        "Market" => "市价",
        "Post-only" => "只挂单",
        "Cross" => "全仓",
        "Isolated" => "逐仓",
        _ => option,
    }
}

fn sync_blocked_select(
    value: RwSignal<String>,
    disabled_options: Memo<Vec<String>>,
    options: &'static [&'static str],
) {
    Effect::new(move |_| {
        let current = value.get();
        if option_disabled(&current, &disabled_options.get()) {
            value.set(first_enabled_option(options, &disabled_options.get()));
        }
    });
}
