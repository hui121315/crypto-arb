use leptos::prelude::*;

use super::super::data::quantity_from_notional_text;
use super::super::draft::{format_price, ExecutionDraft};
use super::super::selection::ExecutionSelection;
use super::fields::{mini_stat, read_only_field};
use crate::panels::modules::market_evidence::leg_evidence_label;
use crate::panels::modules::opportunity_format::missing_quote_text;
use shared_types::OpportunityLegMarketEvidence;

pub(in crate::panels::modules::execution) fn execution_ticket(
    selection: Memo<ExecutionSelection>,
) -> impl IntoView {
    view! {
        <div class="execution-ticket">
            <div class="ticket-copy">
                <span>"执行机会"</span>
                <h3>{move || ticket_heading(&selection.get())}</h3>
                <p>{move || selection.get().summary}</p>
            </div>
            <div class="ticket-stats">
                {mini_stat("净价差", move || selection_metric(
                    &selection.get(),
                    |current| current.edge_label.clone(),
                ), "good")}
                {mini_stat("快照时效", move || selection_metric(
                    &selection.get(),
                    |current| current.freshness_label.clone(),
                ), "good")}
                {mini_stat("费后净利", move || selection_metric(
                    &selection.get(),
                    |current| format!("{:+.3}%", current.one_cycle_net_bps / 100.0),
                ), "info")}
                {mini_stat("来源", move || selection_metric(
                    &selection.get(),
                    |current| current.source_module.to_owned(),
                ), "info")}
            </div>
        </div>
    }
}

pub(in crate::panels::modules::execution) fn leg_panel(
    selection: Memo<ExecutionSelection>,
    draft: ExecutionDraft,
) -> impl IntoView {
    let long = LegSignals::new(draft.long_price, draft.long_notional_usd);
    let short = LegSignals::new(draft.short_price, draft.short_notional_usd);

    view! {
        <div class="leg-config-grid">
            <LegEditor
                title="多腿"
                side="做多 / 买入"
                is_long=true
                selection=selection
                signals=long
                draft=draft
            />
            <LegEditor
                title="空腿"
                side="做空 / 卖出"
                is_long=false
                selection=selection
                signals=short
                draft=draft
            />
        </div>
    }
}

fn ticket_heading(selection: &ExecutionSelection) -> String {
    if selection.opportunity_id.trim().is_empty() {
        "等待选择套利机会".to_owned()
    } else {
        format!("{} · {}", selection.pair, selection.strategy_label)
    }
}

fn selection_metric(
    selection: &ExecutionSelection,
    value: impl FnOnce(&ExecutionSelection) -> String,
) -> String {
    if selection.opportunity_id.trim().is_empty() {
        "-".to_owned()
    } else {
        value(selection)
    }
}

#[derive(Clone, Copy)]
struct LegSignals {
    limit: RwSignal<String>,
    notional: RwSignal<String>,
}

impl LegSignals {
    const fn new(limit: RwSignal<String>, notional: RwSignal<String>) -> Self {
        Self { limit, notional }
    }
}

#[component]
fn LegEditor(
    #[prop(into)] title: String,
    #[prop(into)] side: String,
    is_long: bool,
    selection: Memo<ExecutionSelection>,
    signals: LegSignals,
    draft: ExecutionDraft,
) -> impl IntoView {
    let venue = move || {
        let selection = selection.get();
        if is_long {
            selection.long_leg_label
        } else {
            selection.short_leg_label
        }
    };
    let card_class = if is_long {
        "leg-control-card long-leg"
    } else {
        "leg-control-card short-leg"
    };
    view! {
        <div class=card_class>
            <div class="leg-control-head">
                <div>
                    <span>{title}</span>
                    <strong>{side}</strong>
                </div>
                <em>{venue}</em>
            </div>
            <LegFieldGrid
                is_long=is_long
                selection=selection
                signals=signals
                draft=draft
                venue=venue
            />
        </div>
    }
}

#[component]
fn LegFieldGrid(
    is_long: bool,
    selection: Memo<ExecutionSelection>,
    signals: LegSignals,
    draft: ExecutionDraft,
    venue: impl Fn() -> String + Copy + Send + 'static,
) -> impl IntoView {
    view! {
        <div class="leg-fields">
            <LegIdentityFields
                is_long=is_long
                selection=selection
                signals=signals
                draft=draft
                venue=venue
            />
        </div>
    }
}

#[component]
fn LegIdentityFields(
    is_long: bool,
    selection: Memo<ExecutionSelection>,
    signals: LegSignals,
    draft: ExecutionDraft,
    venue: impl Fn() -> String + Copy + Send + 'static,
) -> impl IntoView {
    view! {
        <div class="leg-field-group">
            {read_only_field("场所", venue)}
            {read_only_field("标的", move || selection.get().pair)}
            {read_only_field("参考价", move || {
                let selection = selection.get();
                let preview = draft.preview.get();
                let fallback = if is_long {
                    selection.long_price_label
                } else {
                    selection.short_price_label
                };
                let reference = if is_long {
                    preview.long_reference_price
                } else {
                    preview.short_reference_price
                };
                leg_price_text(reference, &fallback)
            })}
            {read_only_field("价格证据", move || {
                let preview = draft.preview.get();
                let evidence = if is_long {
                    preview.long_market_evidence.as_ref()
                } else {
                    preview.short_market_evidence.as_ref()
                };
                leg_market_evidence_text(evidence)
            })}
            <EditableField label="限价/保护价" value=signals.limit/>
            <EditableField label="名义金额 USD" value=signals.notional/>
            {read_only_field("数量", move || {
                quantity_from_notional_text(
                    &selection.get().pair,
                    &signals.limit.get(),
                    &signals.notional.get(),
                )
            })}
            {read_only_field("保证金", move || {
                format!("{} {}x", draft.margin_mode.get(), draft.leverage.get())
            })}
            {read_only_field("执行策略", move || {
                format!("{} · {}", draft.order_type.get(), draft.time_in_force.get())
            })}
        </div>
    }
}

fn leg_price_text(reference: Option<f64>, fallback: &str) -> String {
    reference
        .filter(|price| price.is_finite() && *price > f64::EPSILON)
        .map(format_price)
        .or_else(|| positive_fallback_text(fallback))
        .unwrap_or_else(|| missing_quote_text(None))
}

fn leg_market_evidence_text(evidence: Option<&OpportunityLegMarketEvidence>) -> String {
    leg_evidence_label(evidence).unwrap_or_else(|| "缺腿级行情证据".to_owned())
}

fn positive_fallback_text(value: &str) -> Option<String> {
    let price = value.trim().parse::<f64>().ok()?;
    if !price.is_finite() || price <= f64::EPSILON {
        return None;
    }
    Some(format_price(price))
}

#[component]
fn EditableField(#[prop(into)] label: String, value: RwSignal<String>) -> impl IntoView {
    view! {
        <label class="leg-field">
            <span>{label}</span>
            <input
                prop:value=move || value.get()
                on:input=move |ev| value.set(event_target_value(&ev))
            />
        </label>
    }
}

#[cfg(test)]
mod tests;
