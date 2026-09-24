//! 期货机会表格的行、详情与单元格渲染。

use leptos::prelude::*;
use std::sync::Arc;

use crate::panels::modules::futures::columns::ColumnId;
use crate::panels::modules::futures::data::{FuturesOpportunityRow, FUTURES_PAGE_SIZE};
use crate::panels::modules::market_evidence::compact_leg_evidence_label;
use crate::panels::modules::rate_format::signed_bps_percent;

#[path = "rows/details.rs"]
mod details;

use super::format::{
    alignment_text, breakeven_context_text, breakeven_text, execution_reason, execution_title,
    has_price_quote, hours_text, leg_funding_cashflow_text, leg_funding_text, leg_price_line,
    settlement_countdown_text, signed_bps_text, source_age_text,
};
use details::evidence_panel;
use shared_types::{HedgeLegRole, OpportunityLegMarketEvidence};

#[derive(Clone, Copy)]
pub(super) struct RenderBodyInput {
    pub(super) opportunities: Memo<Vec<FuturesOpportunityRow>>,
    pub(super) visible: RwSignal<Vec<ColumnId>>,
    pub(super) selected: RwSignal<Option<String>>,
    pub(super) on_build: Callback<FuturesOpportunityRow>,
    pub(super) on_evidence_close: Callback<()>,
    pub(super) can_build: Memo<bool>,
}

pub(super) fn render_body(input: RenderBodyInput) -> AnyView {
    let RenderBodyInput {
        opportunities,
        visible,
        selected,
        on_build,
        on_evidence_close,
        can_build,
    } = input;
    view! {
        <For
            each=move || opportunities.get().into_iter().take(FUTURES_PAGE_SIZE)
            key=|row| row.id.clone()
            children=move |initial| {
                let id = initial.id.clone();
                let selected_id_for_row = id.clone();
                let selected_id_for_evidence = id.clone();
                let evidence_panel_id = evidence_panel_id(&id);
                // Keep controls and expanded evidence mounted while replacing quote values.
                let row = Memo::new(move |_| opportunities.with(|rows| {
                    rows.iter().find(|row| row.id == id).cloned()
                        .unwrap_or_else(|| Arc::clone(&initial))
                }));
                view! {
                    <tr
                        class=move || if row.get().execution_eligible {
                            "futures-data-row execution-ready"
                        } else {
                            "futures-data-row observation-only"
                        }
                        class:is-selected=move || {
                            selected.get().as_deref() == Some(selected_id_for_row.as_str())
                        }
                    >
                        {move || visible.get().into_iter().map(|col| {
                            if col == ColumnId::Action {
                                render_action_cell(row, selected, on_build, can_build)
                            } else {
                                view! { {move || render_cell(&row.get(), col)} }.into_any()
                            }
                        }).collect_view()}
                    </tr>
                    <Show when=move || selected.get().as_deref() == Some(selected_id_for_evidence.as_str())>
                        <tr class="futures-evidence-row">
                            <td colspan=move || visible.get().len().max(1).to_string()>
                                {evidence_panel(row, on_evidence_close, evidence_panel_id.clone())}
                            </td>
                        </tr>
                    </Show>
                }
            }
        />
    }
    .into_any()
}

fn render_cell(opp: &FuturesOpportunityRow, col: ColumnId) -> AnyView {
    match col {
        ColumnId::StrategyKind => {
            let source = source_age_text(opp);
            let title = format!("{} · {}", opp.cost_evidence_label(), source);
            view! {
                <td class=col.cell_class() title=title>
                    <strong>{opp.strategy_label.clone()}</strong>
                    <small>{risk_text(&opp.risk)}</small>
                    {opp.spot_leg_mode_label()
                        .map(|label| view! { <small>{format!("现货腿 {label}")}</small> })}
                </td>
            }
            .into_any()
        }
        ColumnId::Symbol => {
            let title = source_age_text(opp);
            view! {
                <td class=col.cell_class() title=title>
                    <strong>{opp.pair.clone()}</strong>
                    <small>{risk_text(&opp.risk)}</small>
                    {opp.spot_leg_mode_label()
                        .map(|label| view! { <small>{format!("现货腿 {label}")}</small> })}
                </td>
            }
            .into_any()
        }
        ColumnId::LongLeg => view! {
            <td class=col.cell_class()>
                <LegCell
                    leg=opp.long_leg.clone()
                    price=opp.long_price.clone()
                    evidence=opp.long_market_evidence.clone()
                    evidence_raw=opp.long_market_evidence_raw.clone()
                    funding=opp.long_funding.clone()
                    role=HedgeLegRole::Long
                />
            </td>
        }
        .into_any(),
        ColumnId::ShortLeg => view! {
            <td class=col.cell_class()>
                <LegCell
                    leg=opp.short_leg.clone()
                    price=opp.short_price.clone()
                    evidence=opp.short_market_evidence.clone()
                    evidence_raw=opp.short_market_evidence_raw.clone()
                    funding=opp.short_funding.clone()
                    role=HedgeLegRole::Short
                />
            </td>
        }
        .into_any(),
        ColumnId::NetBasisBps => {
            let class_name = if opp.execution_eligible {
                "positive"
            } else {
                "muted"
            };
            view! {
                <td class=cell_state_class(col, class_name)>
                    {signed_bps_percent(opp.net_basis_bps)}
                </td>
            }
            .into_any()
        }
        ColumnId::GrossOneCycleBps => {
            let class_name = if !opp.execution_eligible || !opp.cost_verified {
                "muted"
            } else {
                "positive"
            };
            view! { <td class=cell_state_class(col, class_name)>{opp.gross_one_cycle_text()}</td> }
                .into_any()
        }
        ColumnId::OneCycleNetBps => {
            let class_name = if !opp.execution_eligible || !opp.cost_verified {
                "muted"
            } else if opp.one_cycle_covers_cost {
                "positive"
            } else {
                "negative"
            };
            view! { <td class=cell_state_class(col, class_name)>{opp.one_cycle_net_text()}</td> }
                .into_any()
        }
        ColumnId::RoundTripCostBps => {
            let class_name = if opp.cost_verified { "" } else { "muted" };
            view! {
                <td class=cell_state_class(col, class_name)>
                    <strong>{opp.round_trip_cost_text()}</strong>
                    <small>{opp.cost_evidence_label()}</small>
                </td>
            }
            .into_any()
        }
        ColumnId::PredictedFunding => view! {
            <td class=col.cell_class()>
                <div class="funding-cell">
                    <span class=if opp.execution_eligible { "positive" } else { "muted" }>
                        {signed_bps_text(opp.predicted_funding_bps, "缺证据")}
                    </span>
                    {super::super::sparkline::sparkline(opp.funding_curve.clone())}
                </div>
            </td>
        }
        .into_any(),
        ColumnId::CostBreakeven => view! {
            <td class=col.cell_class()>
                <div class="cost-cell">
                    <strong>{breakeven_text(opp)}</strong>
                    <span>{breakeven_context_text(opp)}</span>
                </div>
            </td>
        }
        .into_any(),
        ColumnId::IndexComposition => view! {
            <td class=col.cell_class() title=opp.index_composition.detail.clone()>
                {opp.index_composition.compact_text()}
            </td>
        }
        .into_any(),
        ColumnId::Action => ().into_any(),
        ColumnId::FundingCyclePercentile => {
            view! { <td class=col.cell_class()>{opp.funding_stats.percentile_text()}</td> }
                .into_any()
        }
        col @ (ColumnId::BorrowCost
        | ColumnId::FundingAlignment
        | ColumnId::FundingCapDistance
        | ColumnId::MinHold
        | ColumnId::SettlementCountdown) => render_auxiliary_cell(opp, col),
    }
}

fn render_auxiliary_cell(opp: &FuturesOpportunityRow, col: ColumnId) -> AnyView {
    let text = match col {
        ColumnId::BorrowCost => signed_bps_text(opp.borrow_cost_bps_per_day, "未接入"),
        ColumnId::FundingAlignment => alignment_text(opp.funding_alignment_minutes),
        ColumnId::FundingCapDistance => signed_bps_text(opp.funding_cap_distance_bps, "待窗口"),
        ColumnId::MinHold => hours_text(opp.min_hold_hours),
        ColumnId::SettlementCountdown => settlement_countdown_text(opp),
        _ => return ().into_any(),
    };
    view! { <td class=col.cell_class()>{text}</td> }.into_any()
}

fn render_action_cell(
    opp: Memo<FuturesOpportunityRow>,
    selected: RwSignal<Option<String>>,
    on_build: Callback<FuturesOpportunityRow>,
    can_build: Memo<bool>,
) -> AnyView {
    // A queued shared-readiness update can outlive a row removed by pagination.
    let title = move || {
        opp.try_get()
            .map(|row| execution_title(&row))
            .unwrap_or_default()
    };
    let selected_id = opp.get_untracked().id.clone();
    let selected_id_for_state = selected_id.clone();
    let selected_id_for_click = selected_id.clone();
    let evidence_panel_id = evidence_panel_id(&selected_id);
    let mobile_net_class = move || {
        let opp = opp.get();
        if !opp.execution_eligible || !opp.cost_verified {
            "row-mobile-net is-muted"
        } else if opp.one_cycle_covers_cost {
            "row-mobile-net is-positive"
        } else {
            "row-mobile-net is-negative"
        }
    };
    view! {
        <td class=ColumnId::Action.cell_class()>
            <div class="row-action-stack">
                <span class=mobile_net_class aria-hidden="true">
                    <small>"费后净边际"</small>
                    <strong>{move || opp.get().one_cycle_net_text()}</strong>
                </span>
                <Show when=move || opp.get().execution_eligible fallback=move || view! {
                    <span class="row-observation-status" title=title>"仅观察"</span>
                }>
                        <button
                            class="row-action"
                            title=move || if can_build.get() { title() } else { "快照更新中或不可用，恢复后可构建".to_owned() }
                            disabled=move || !can_build.get()
                            on:click=move |_| {
                                if can_build.get_untracked() {
                                    if let Some(row) = opp.try_get_untracked() { on_build.run(row); }
                                }
                            }
                        >
                            "构建新双腿"
                        </button>
                </Show>
                {move || execution_reason(&opp.get()).map(|text| {
                    view! { <small class="row-action-reason" title=title>{text}</small> }
                })}
                <button
                    type="button"
                    class="row-evidence-action"
                    aria-controls=evidence_panel_id
                    aria-expanded=move || {
                        (selected.get().as_deref() == Some(selected_id_for_state.as_str()))
                            .to_string()
                    }
                    on:click=move |_| {
                        let next = if selected.get_untracked().as_deref()
                            == Some(selected_id_for_click.as_str())
                        {
                            None
                        } else {
                            Some(selected_id_for_click.clone())
                        };
                        selected.set(next);
                    }
                >
                    {move || {
                        if selected.get().as_deref() == Some(selected_id.as_str()) {
                            "收起证据"
                        } else {
                            "查看证据"
                        }
                    }}
                </button>
            </div>
        </td>
    }
    .into_any()
}

fn evidence_panel_id(id: &str) -> String {
    let slug = id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    format!("futures-evidence-{slug}")
}

fn cell_state_class(col: ColumnId, state: &str) -> String {
    if state.is_empty() {
        col.cell_class().to_owned()
    } else {
        format!("{} {state}", col.cell_class())
    }
}

fn risk_text(risk: &str) -> String {
    let risk = risk.trim();
    if risk.is_empty() {
        "风险 未知".to_owned()
    } else {
        format!("风险 {risk}")
    }
}

#[component]
fn LegCell(
    #[prop(into)] leg: String,
    #[prop(into)] price: String,
    evidence: Option<String>,
    evidence_raw: Option<OpportunityLegMarketEvidence>,
    funding: Option<shared_types::OpportunityListLegFunding>,
    role: HedgeLegRole,
) -> impl IntoView {
    let price_line = leg_price_line(&price, evidence.as_deref());
    let evidence_title = evidence.clone();
    let compact_evidence = if has_price_quote(&price) {
        compact_leg_evidence_label(evidence_raw.as_ref())
            .or_else(|| evidence.map(|line| (line, "is-unknown")))
    } else {
        None
    };
    let cashflow = funding
        .as_ref()
        .map(|funding| leg_funding_cashflow_text(funding, role));
    view! {
        <div class="leg-price-cell">
            <div class="leg-price-heading">
                <strong>{leg}</strong>
                {cashflow.map(|(text, state)| {
                    view! { <small class=format!("leg-cashflow {state}")>{text}</small> }
                })}
            </div>
            <div class="leg-market-line">
                <span>{price_line}</span>
                {compact_evidence.map(|(line, state)| {
                    view! {
                        <small
                            class=format!("leg-market-evidence {state}")
                            title=evidence_title.unwrap_or_else(|| "行情证据待补充".to_owned())
                        >
                            {line}
                        </small>
                    }
                })}
            </div>
            {funding.map(|funding| {
                view! { <small class="leg-funding-line">{leg_funding_text(&funding)}</small> }
            })}
        </div>
    }
}
