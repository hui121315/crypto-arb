use super::derive::FundingDisplay;
use super::*;

#[path = "row/close_confirmation.rs"]
mod close_confirmation;
#[path = "row/labels.rs"]
mod labels;
#[path = "row/mobile.rs"]
mod mobile;

use close_confirmation::{
    close_confirmation_dom_id, close_confirmation_row, CloseConfirmationInput,
};
use labels::{
    close_button_title, liquidation_distance_label, liquidation_price_label, pair_risk_label,
};
use mobile::{mobile_risk_summary, MobileRiskSummaryInput};

#[derive(Clone, Copy)]
pub(super) struct RowBodyContext {
    pub(super) all_rows: Memo<Arc<[PositionRow]>>,
    pub(super) field_quality: Memo<Vec<AccountFieldQuality>>,
    pub(super) row_health: Memo<Vec<AccountDataHealth>>,
    pub(super) on_close: Callback<PositionRow>,
    pub(super) on_close_pair: Callback<PositionRow>,
    pub(super) closing_key: RwSignal<Option<String>>,
    pub(super) close_confirmation_key: RwSignal<Option<String>>,
    pub(super) expanded_evidence_key: RwSignal<Option<String>>,
    pub(super) live_account_close_ready: Memo<bool>,
}

pub(super) fn render_body(
    page: Memo<TablePage>,
    rows: Memo<Vec<Arc<PositionRow>>>,
    context: RowBodyContext,
) -> impl IntoView {
    let empty_text = Memo::new(move |_| {
        let page = page.get();
        (page.total == 0).then(|| table_empty_text(&page))
    });

    view! {
        {move || empty_text.get().map(|text| view! {
            <tr><td colspan="8" class="empty-cell">{text}</td></tr>
        })}
        <For
            each=move || rows.get()
            key=|row| Arc::as_ptr(row) as usize
            children=move |row| view! {
                <PositionTableRow
                    row=row
                    all_rows=context.all_rows
                    field_quality=context.field_quality
                    row_health=context.row_health
                    on_close=context.on_close
                    on_close_pair=context.on_close_pair
                    closing_key=context.closing_key
                    close_confirmation_key=context.close_confirmation_key
                    expanded_evidence_key=context.expanded_evidence_key
                    live_account_close_ready=context.live_account_close_ready
                />
            }
        />
    }
}

#[component]
fn PositionTableRow(
    row: Arc<PositionRow>,
    all_rows: Memo<Arc<[PositionRow]>>,
    field_quality: Memo<Vec<AccountFieldQuality>>,
    row_health: Memo<Vec<AccountDataHealth>>,
    on_close: Callback<PositionRow>,
    on_close_pair: Callback<PositionRow>,
    closing_key: RwSignal<Option<String>>,
    close_confirmation_key: RwSignal<Option<String>>,
    expanded_evidence_key: RwSignal<Option<String>>,
    live_account_close_ready: Memo<bool>,
) -> impl IntoView {
    let quality_target = Arc::clone(&row);
    let quality_rows = Memo::new(move |_| {
        field_quality.with(|quality| position_quality_for_row(&quality_target, quality))
    });
    let health_target = Arc::clone(&row);
    let health_rows = Memo::new(move |_| {
        row_health.with(|health| position_row_health_for_row(&health_target, health))
    });

    let has_pair = has_pair_evidence(&row);
    let pair_display = pair_label(&row).unwrap_or_else(|| "未配对".into());
    let pair_detail = format!(
        "{} · {} · {}",
        row.venue,
        side_label(row.side),
        format_qty(row.quantity)
    );
    let pair_risk_target = Arc::clone(&row);
    let pair_risk_text = Memo::new(move |_| {
        all_rows.with(|rows| pair_risk_label(&pair_risk_target, rows, has_pair))
    });

    let label_key = position_key(&row);
    let pair_key = pair_close_key(&row);
    let confirmation_key = pair_key.clone().unwrap_or_else(|| label_key.clone());
    let confirmation_panel_id = close_confirmation_dom_id(&confirmation_key);
    let confirmation_match_key = confirmation_key.clone();
    let confirmation_open = Memo::new(move |_| {
        close_confirmation_key
            .with(|active| active.as_deref() == Some(confirmation_match_key.as_str()))
    });
    let evidence_key = label_key.clone();
    let evidence_panel_id = position_evidence_panel_dom_id(&label_key);
    let evidence_expanded = Memo::new(move |_| {
        expanded_evidence_key.with(|active| active.as_deref() == Some(evidence_key.as_str()))
    });
    let toggle_key = label_key.clone();
    let toggle_evidence = Callback::new(move |_| {
        close_confirmation_key.set(None);
        expanded_evidence_key.update(|active| {
            if active.as_deref() == Some(toggle_key.as_str()) {
                *active = None;
            } else {
                *active = Some(toggle_key.clone());
            }
        });
    });
    let row_class = severity_class(row.severity);
    let venue = row.venue.clone();
    let symbol = row.symbol.clone();
    let hedge_href = futures_symbol_href(&symbol);
    let mobile_hedge_href = hedge_href.clone();
    let compact_hedge_href = hedge_href.clone();
    let hedge_label = format!("筛选 {symbol} 独立双腿机会，不会补齐当前仓位");
    let mobile_hedge_label = hedge_label.clone();
    let compact_hedge_label = hedge_label.clone();
    let requires_live = position_close_requires_live(&row);
    let confirmation_trigger_id = confirmation_panel_id.clone();
    let arm_confirmation_key = confirmation_key.clone();
    let close_expanded_evidence = expanded_evidence_key;
    let close_trigger = NodeRef::<leptos::html::Button>::new();

    let identity_quality = quality_rows;
    let identity_health = health_rows;
    let evidence_control_id = evidence_panel_id.clone();
    let identity_cell = view! {
        <td class="position-identity-cell">
            <div class="position-identity-main">
                <strong>{symbol}</strong>
                <span class=side_class(row.side)>{side_label(row.side)}</span>
            </div>
            <small class="position-venue-label">{venue}</small>
            {move || position_row_evidence_toggle(
                &identity_quality.get(),
                &identity_health.get(),
                evidence_expanded,
                evidence_control_id.clone(),
                toggle_evidence,
            )}
        </td>
    };

    let size_row = Arc::clone(&row);
    let size_quality = quality_rows;
    let size_cell = move || {
        let quality = size_quality.get();
        let leverage_quality = position_quality_by_field(&quality, "leverage");
        let leverage_text = value_or_missing(
            leverage_quality.as_ref(),
            format!("{:.1}x", size_row.leverage),
        );
        view! {
            <td class="num position-stacked-cell">
                <strong>{format_qty(size_row.quantity)}</strong>
                <small>{leverage_text}</small>
            </td>
        }
    };

    let valuation_row = Arc::clone(&row);
    let valuation_quality = quality_rows;
    let valuation_cell = move || {
        let quality = valuation_quality.get();
        let mark_quality = position_quality_by_field(&quality, "markPrice");
        let mark_text = value_or_missing(mark_quality.as_ref(), price(valuation_row.mark_price));
        view! {
            <td class="num position-stacked-cell">
                <strong>{price(valuation_row.entry_price)}</strong>
                <small>{mark_text}</small>
            </td>
        }
    };

    let pnl_row = Arc::clone(&row);
    let pnl_quality = quality_rows;
    let pnl_margin_cell = move || {
        let quality = pnl_quality.get();
        let mark_quality = position_quality_by_field(&quality, "markPrice");
        let margin_quality = position_quality_by_field(&quality, "margin");
        let pnl = pnl_display(pnl_row.unrealized_pnl_usd, mark_quality.as_ref());
        let margin = value_or_missing(margin_quality.as_ref(), money(pnl_row.margin_usd));
        view! {
            <td class="num position-stacked-cell">
                <strong class=pnl.class>{pnl.value}</strong>
                <small>{margin}</small>
            </td>
        }
    };

    let liquidation_row = Arc::clone(&row);
    let liquidation_cell = move || {
        let distance = liquidation_distance_label(&liquidation_row);
        let liquidation = liquidation_price_label(liquidation_row.liquidation_price);
        view! {
            <td class="num position-stacked-cell position-liquidation-cell">
                <strong>{distance}</strong>
                <small>{liquidation}</small>
                <small class="pair-risk-text">{pair_risk_text.get()}</small>
            </td>
        }
    };

    let funding_row = Arc::clone(&row);
    let funding_quality_rows = quality_rows;
    let funding_cell = move || {
        let quality = funding_quality_rows.get();
        let funding_quality = position_quality_by_field(&quality, "fundingRate8h");
        let funding = funding_display(&funding_row, funding_quality.as_ref());
        view! {
            <td class="num position-stacked-cell position-funding-cell">
                <strong class=funding.class>{funding.window}</strong>
                <small>{funding.detail.unwrap_or_default()}</small>
            </td>
        }
    };

    let pair_support = if has_pair {
        "双腿联动"
    } else {
        "无配对证据"
    };
    let mobile_risk_summary = mobile_risk_summary(MobileRiskSummaryInput {
        row: Arc::clone(&row),
        quality_rows,
        pair_display: pair_display.clone(),
        pair_support,
        hedge_href: mobile_hedge_href,
        hedge_label: mobile_hedge_label,
        has_pair,
    });
    let pair_guidance = if has_pair {
        view! { <small>{pair_support}</small> }.into_any()
    } else {
        view! {
            <a class="position-pair-action" href=hedge_href aria-label=hedge_label>
                "筛选独立机会"
            </a>
        }
        .into_any()
    };
    let action_cells = view! {
        <td class="position-pair-cell">
            <strong class="pair-text" title=pair_detail>{pair_display.clone()}</strong>
            {pair_guidance}
        </td>
        <td class="positions-action-column">
            <div class="position-action-stack">
                {(!has_pair).then(|| view! {
                    <a
                        class="position-action-hedge"
                        href=compact_hedge_href
                        aria-label=compact_hedge_label
                    >"新双腿"</a>
                })}
                <button
                    node_ref=close_trigger
                    type="button"
                    class="row-close-button"
                    title=move || close_button_title(has_pair, requires_live, live_account_close_ready.get())
                    aria-expanded=move || confirmation_open.get().to_string()
                    aria-controls=confirmation_trigger_id
                    disabled=move || {
                        closing_key.get().is_some()
                            || !position_close_enabled(requires_live, live_account_close_ready.get())
                    }
                    on:click=move |_| {
                        close_expanded_evidence.set(None);
                        close_confirmation_key.set(Some(arm_confirmation_key.clone()));
                    }
                >
                    {move || if row_is_closing(closing_key.get().as_deref(), &label_key, pair_key.as_deref()) {
                        "提交中"
                    } else if !position_close_enabled(requires_live, live_account_close_ready.get()) {
                        "需实盘"
                    } else if confirmation_open.get() {
                        "待确认"
                    } else if has_pair {
                        "平配对"
                    } else {
                        "平仓"
                    }}
                </button>
            </div>
        </td>
    };

    let evidence_quality = quality_rows;
    let evidence_health = health_rows;
    let evidence_label = format!(
        "{} · {} · {}",
        row.symbol,
        row.venue.to_uppercase(),
        side_label(row.side),
    );
    let evidence_region_label = format!("{evidence_label} 持仓证据");
    let close_confirmation = close_confirmation_row(CloseConfirmationInput {
        row: row.as_ref().clone(),
        pair_display,
        has_pair,
        confirmation_key,
        panel_id: confirmation_panel_id,
        open: confirmation_open,
        trigger: close_trigger,
        active_key: close_confirmation_key,
        closing_key,
        live_ready: live_account_close_ready,
        on_close,
        on_close_pair,
    });

    view! {
        <tr class=row_class>
            {identity_cell}
            {size_cell}
            {valuation_cell}
            {pnl_margin_cell}
            {liquidation_cell}
            {funding_cell}
            {action_cells}
        </tr>
        {mobile_risk_summary}
        {close_confirmation}
        {move || evidence_expanded.get().then(|| view! {
            <tr class="position-evidence-row">
                <td colspan="8">
                    <div
                        id=evidence_panel_id.clone()
                        class="position-evidence-panel"
                        role="region"
                        aria-label=evidence_region_label.clone()
                    >
                        <strong>{evidence_label.clone()}</strong>
                        {position_row_evidence_panel(
                            &evidence_quality.get(),
                            &evidence_health.get(),
                        )}
                    </div>
                </td>
            </tr>
        })}
    }
}

fn position_evidence_panel_dom_id(key: &str) -> String {
    let token = key
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    format!("position-evidence-{token}")
}

pub(super) fn position_close_requires_live(row: &PositionRow) -> bool {
    row.origin == shared_types::PositionOrigin::AccountPrivate
}

pub(super) const fn position_close_enabled(requires_live: bool, live_ready: bool) -> bool {
    !requires_live || live_ready
}

fn futures_symbol_href(symbol: &str) -> String {
    format!("#futures?symbol={}&origin=position", symbol.trim())
}
