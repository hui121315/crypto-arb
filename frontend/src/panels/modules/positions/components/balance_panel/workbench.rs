use leptos::prelude::*;
use shared_types::{AccountDataHealth, AccountFieldQuality};

use super::super::format::precise_money;
use super::balance_group;
use super::derive::VenueBalanceGroup;

const BALANCE_ACCOUNT_PANEL_ID: &str = "balance-account-panel";

pub(super) fn balance_status_note(text: String) -> impl IntoView {
    view! { <div class="risk-empty stale-note">{text}</div> }
}

pub(super) fn balance_account_workbench(
    groups: Memo<Vec<VenueBalanceGroup>>,
    field_quality: Memo<Vec<AccountFieldQuality>>,
    row_health: Memo<Vec<AccountDataHealth>>,
    selected_venue: RwSignal<Option<String>>,
) -> AnyView {
    let unvalued_open = RwSignal::new(false);
    let active_key = Memo::new(move |_| {
        let selected = selected_venue.get();
        groups.with(|groups| resolved_group_key(groups, selected.as_deref()))
    });

    view! {
        <section class="balance-account-workbench" aria-label="交易所账户余额">
            <div class="balance-account-picker">
                <div class="balance-account-picker-meta">
                    <label for="balance-account-select">"交易所账户"</label>
                    <span>{move || format!("{} 个账户", groups.with(Vec::len))}</span>
                </div>
                <select
                    id="balance-account-select"
                    aria-label="选择交易所账户"
                    prop:value=move || active_key.get().unwrap_or_default()
                    on:change=move |event| {
                        selected_venue.set(Some(event_target_value(&event)));
                        unvalued_open.set(false);
                    }
                >
                    <For
                        each=move || groups.with(|rows| rows.iter().map(group_key).collect::<Vec<_>>())
                        key=|key| key.clone()
                        children=move |key| {
                            let lookup = key.clone();
                            view! { <option value=key>{move || groups.with(|rows| {
                                rows.iter().find(|row| group_key(row) == lookup).map(account_option_label).unwrap_or_default()
                            })}</option> }
                        }
                    />
                </select>
            </div>
            <div
                class="balance-account-detail"
                role="region"
                id=BALANCE_ACCOUNT_PANEL_ID
                aria-labelledby="balance-account-select"
                aria-live="polite"
                tabindex="0"
            >
                <For
                    each=move || {
                        let active = active_key.get();
                        groups.with(|rows| rows.iter()
                            .filter(|group| Some(group_key(group)) == active)
                            .cloned().collect::<Vec<_>>())
                    }
                    key=group_key
                    children=move |initial| {
                        let key = group_key(&initial);
                        let group = Memo::new(move |_| groups.with(|rows| {
                            rows.iter().find(|group| group_key(group) == key)
                                .cloned().unwrap_or_else(|| initial.clone())
                        }));
                        balance_group(group, field_quality, row_health, unvalued_open)
                    }
                />
            </div>
        </section>
    }
    .into_any()
}

fn account_option_label(group: &VenueBalanceGroup) -> String {
    let venue = group.venue.trim().to_ascii_uppercase();
    let equity = group
        .summary
        .as_ref()
        .filter(|summary| summary.total_equity_usd.is_finite())
        .map(|summary| precise_money(summary.total_equity_usd))
        .unwrap_or_else(|| "待确认".to_owned());
    let attention = if group.unknown_valuation_count == 0 {
        String::new()
    } else {
        format!(" · 待估 {}", group.unknown_valuation_count)
    };
    format!("{venue} · {equity}{attention}")
}

pub(super) fn resolved_group_key(
    groups: &[VenueBalanceGroup],
    selected: Option<&str>,
) -> Option<String> {
    selected
        .filter(|selected| groups.iter().any(|group| group_key(group) == *selected))
        .map(str::to_owned)
        .or_else(|| groups.first().map(group_key))
}

fn group_key(group: &VenueBalanceGroup) -> String {
    shared_types::normalized_venue_name(&group.venue)
}
