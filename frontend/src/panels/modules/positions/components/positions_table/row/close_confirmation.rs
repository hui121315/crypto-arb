use super::*;

#[derive(Clone)]
pub(super) struct CloseConfirmationInput {
    pub(super) row: PositionRow,
    pub(super) pair_display: String,
    pub(super) has_pair: bool,
    pub(super) confirmation_key: String,
    pub(super) panel_id: String,
    pub(super) open: Memo<bool>,
    pub(super) trigger: NodeRef<leptos::html::Button>,
    pub(super) active_key: RwSignal<Option<String>>,
    pub(super) closing_key: RwSignal<Option<String>>,
    pub(super) live_ready: Memo<bool>,
    pub(super) on_close: Callback<PositionRow>,
    pub(super) on_close_pair: Callback<PositionRow>,
}

struct CloseConfirmationCopy {
    requires_live: bool,
    title: &'static str,
    final_label: &'static str,
    identity: String,
    scope: &'static str,
    environment: &'static str,
    mark_price: String,
    notional: String,
    pnl: String,
    pnl_class: &'static str,
    liquidation_distance: String,
    mark_label: &'static str,
    notional_label: &'static str,
    pnl_label: &'static str,
    liquidation_label: &'static str,
    region_label: String,
    description_id: String,
}

pub(super) fn close_confirmation_row(input: CloseConfirmationInput) -> impl IntoView {
    let CloseConfirmationInput {
        row,
        pair_display,
        has_pair,
        confirmation_key,
        panel_id,
        open,
        trigger,
        active_key,
        closing_key,
        live_ready,
        on_close,
        on_close_pair,
    } = input;
    let CloseConfirmationCopy {
        requires_live,
        title,
        final_label,
        identity,
        scope,
        environment,
        mark_price,
        notional,
        pnl,
        pnl_class,
        liquidation_distance,
        mark_label,
        notional_label,
        pnl_label,
        liquidation_label,
        region_label,
        description_id,
    } = close_confirmation_copy(&row, &pair_display, has_pair, &panel_id);
    let scope_description_id = description_id.clone();
    let cancel_key = confirmation_key.clone();
    let confirm_key = confirmation_key;
    let cancel_active_key = active_key;
    let cancel_button = NodeRef::<leptos::html::Button>::new();
    let cancel_focus = cancel_button;
    Effect::new(move |_| {
        if open.get() {
            if let Some(button) = cancel_focus.get() {
                let _ = button.focus();
            }
        }
    });
    let cancel_trigger = trigger;
    let escape_trigger = trigger;
    let submit_trigger = trigger;
    let escape_key = cancel_key.clone();
    let close_row = row.clone();
    let pair_close_row = row;

    view! {
        <tr class="position-close-confirmation-row" hidden=move || !open.get()>
            <td colspan="8">
                <div
                    id=panel_id
                    class="position-close-confirmation"
                    role="group"
                    aria-label=region_label
                    aria-describedby=description_id
                    on:keydown=move |event| {
                        if event.key() != "Escape" {
                            return;
                        }
                        event.prevent_default();
                        active_key.update(|active| {
                            if active.as_deref() == Some(escape_key.as_str()) {
                                *active = None;
                            }
                        });
                        if let Some(button) = escape_trigger.get() {
                            let _ = button.focus();
                        }
                    }
                >
                    <div class="position-close-confirmation-copy">
                        <span>
                            <strong>{title}</strong>
                            <em>{environment}</em>
                        </span>
                        <p>{identity}</p>
                        <div class="position-close-confirmation-facts">
                            <span><small>{mark_label}</small><strong>{mark_price}</strong></span>
                            <span><small>{notional_label}</small><strong>{notional}</strong></span>
                            <span><small>{pnl_label}</small><strong class=pnl_class>{pnl}</strong></span>
                            <span><small>{liquidation_label}</small><strong>{liquidation_distance}</strong></span>
                        </div>
                        <small id=scope_description_id>{scope}</small>
                    </div>
                    <div class="position-close-confirmation-actions">
                        <button
                            node_ref=cancel_button
                            type="button"
                            class="position-close-cancel"
                            on:click=move |_| {
                                cancel_active_key.update(|active| {
                                    if active.as_deref() == Some(cancel_key.as_str()) {
                                        *active = None;
                                    }
                                });
                                if let Some(button) = cancel_trigger.get() {
                                    let _ = button.focus();
                                }
                            }
                        >"取消"</button>
                        <button
                            type="button"
                            class="position-close-confirm"
                            disabled=move || {
                                closing_key.get().is_some()
                                    || !position_close_enabled(requires_live, live_ready.get())
                            }
                            on:click=move |_| {
                                active_key.update(|active| {
                                    if active.as_deref() == Some(confirm_key.as_str()) {
                                        *active = None;
                                    }
                                });
                                if let Some(button) = submit_trigger.get() {
                                    let _ = button.focus();
                                }
                                if has_pair {
                                    on_close_pair.run(pair_close_row.clone());
                                } else {
                                    on_close.run(close_row.clone());
                                }
                            }
                        >{final_label}</button>
                    </div>
                </div>
            </td>
        </tr>
    }
}

fn close_confirmation_copy(
    row: &PositionRow,
    pair_display: &str,
    has_pair: bool,
    panel_id: &str,
) -> CloseConfirmationCopy {
    let requires_live = position_close_requires_live(row);
    let title = if has_pair {
        "确认平配对"
    } else {
        "确认平仓"
    };
    let identity = if has_pair {
        format!(
            "{} {} {} {} ↔ {}",
            row.venue,
            row.symbol,
            side_label(row.side),
            format_qty(row.quantity),
            pair_display,
        )
    } else {
        format!(
            "{} {} {} {}",
            row.venue,
            row.symbol,
            side_label(row.side),
            format_qty(row.quantity),
        )
    };
    let notional_value = row.pair_evidence.as_ref().filter(|_| has_pair).map_or_else(
        || (row.quantity * row.mark_price).abs(),
        |evidence| evidence.matched_notional_usd,
    );
    let pnl_class = if !row.unrealized_pnl_usd.is_finite() {
        "muted"
    } else if row.unrealized_pnl_usd >= 0.0 {
        "positive"
    } else {
        "negative"
    };
    CloseConfirmationCopy {
        requires_live,
        title,
        final_label: match (has_pair, requires_live) {
            (true, true) => "实盘平配对",
            (true, false) => "模拟平配对",
            (false, true) => "实盘平仓",
            (false, false) => "模拟平仓",
        },
        scope: if has_pair {
            "两条配对腿将分别提交 reduce-only 市价平仓；双腿均取得交易所终态后才算完成"
        } else {
            "只平当前场所的这笔仓位；市价 reduce-only，成交仍以交易所终态为准"
        },
        environment: if requires_live { "实盘" } else { "模拟" },
        mark_price: price(row.mark_price),
        notional: money(notional_value),
        pnl: money(row.unrealized_pnl_usd),
        pnl_class,
        liquidation_distance: row
            .liquidation_distance_pct
            .filter(|distance| distance.is_finite())
            .map_or_else(|| "未知".to_owned(), |distance| format!("{distance:.1}%")),
        mark_label: if has_pair {
            "当前腿标记"
        } else {
            "标记价"
        },
        notional_label: if has_pair {
            "匹配名义"
        } else {
            "当前价值"
        },
        pnl_label: if has_pair {
            "当前腿 PnL"
        } else {
            "未实现 PnL"
        },
        liquidation_label: if has_pair {
            "当前腿强平距"
        } else {
            "强平距离"
        },
        region_label: format!("{title}：{identity}"),
        description_id: format!("{panel_id}-scope"),
        identity,
    }
}

pub(super) fn close_confirmation_dom_id(key: &str) -> String {
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
    format!("position-close-confirmation-{token}")
}
