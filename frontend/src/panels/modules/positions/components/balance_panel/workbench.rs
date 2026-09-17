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
    groups: Vec<VenueBalanceGroup>,
    field_quality: &[AccountFieldQuality],
    row_health: &[AccountDataHealth],
    selected_venue: RwSignal<Option<String>>,
) -> AnyView {
    let selector_groups = groups.clone();
    let account_count = groups.len();
    let groups = StoredValue::new(groups);
    let field_quality = StoredValue::new(field_quality.to_vec());
    let row_health = StoredValue::new(row_health.to_vec());
    let active_key = Memo::new(move |_| {
        let selected = selected_venue.get();
        groups.with_value(|groups| resolved_group_key(groups, selected.as_deref()))
    });

    view! {
        <section class="balance-account-workbench" aria-label="交易所账户余额">
            <div class="balance-account-picker">
                <div class="balance-account-picker-meta">
                    <label for="balance-account-select">"交易所账户"</label>
                    <span>{format!("{account_count} 个已接入")}</span>
                </div>
                <select
                    id="balance-account-select"
                    aria-label="选择交易所账户"
                    prop:value=move || active_key.get().unwrap_or_default()
                    on:change=move |event| {
                        selected_venue.set(Some(event_target_value(&event)));
                    }
                >
                    {selector_groups.iter().map(account_option).collect_view()}
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
                {move || {
                    let Some(active) = active_key.get() else { return ().into_any() };
                    let selected_group = groups.with_value(|groups| {
                        groups.iter().find(|group| group_key(group) == active).cloned()
                    });
                    let Some(group) = selected_group else { return ().into_any() };
                    field_quality.with_value(|quality| {
                        row_health.with_value(|health| balance_group(group, quality, health))
                    })
                }}
            </div>
        </section>
    }
    .into_any()
}

fn account_option(group: &VenueBalanceGroup) -> impl IntoView {
    let key = group_key(group);
    let venue = group.venue.trim().to_ascii_uppercase();
    let equity = group
        .summary
        .as_ref()
        .map(|summary| precise_money(summary.total_equity_usd))
        .unwrap_or_else(|| "待确认".to_owned());
    let attention = if group.unknown_valuation_count == 0 {
        String::new()
    } else {
        format!(" · 待估 {}", group.unknown_valuation_count)
    };
    let label = format!("{venue} · {equity}{attention}");

    view! {
        <option value=key>{label}</option>
    }
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
