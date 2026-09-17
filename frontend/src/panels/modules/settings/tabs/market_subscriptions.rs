use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{
    MarketSubscriptionFeedRuntime, MarketSubscriptionPatch, MarketSubscriptionRuntimeState,
    MarketSubscriptionsResponse, VenueMarketSubscription, VenueMarketSubscriptionRuntime,
};
use std::collections::BTreeMap;

use super::super::data::{
    settings_state, use_market_subscription_update_action, use_market_subscriptions,
    MarketSubscriptionUpdateAction,
};
use super::{action_message, problem_cell, problem_message};

pub(in crate::panels::modules::settings) fn market_subscriptions_tab() -> impl IntoView {
    let refresh_nonce = RwSignal::new(0_u64);
    let subscriptions = use_market_subscriptions(refresh_nonce);
    let action = use_market_subscription_update_action(refresh_nonce);

    view! {
        <div class="settings-stack">
            {move || subscription_content(settings_state(subscriptions), action)}
        </div>
    }
}

fn subscription_content(
    state: LoadState<MarketSubscriptionsResponse>,
    action: MarketSubscriptionUpdateAction,
) -> AnyView {
    let (response, stale_problem) = match state {
        LoadState::Ready(response) => (response, None),
        LoadState::Stale { value, problem } => (value, Some(problem)),
        LoadState::Error(problem) => return problem_cell("读取行情订阅失败", &problem),
        LoadState::Loading => {
            return view! { <div class="empty-cell">"正在读取行情订阅"</div> }.into_any();
        }
    };
    let enabled = response
        .venues
        .iter()
        .filter(|row| row.spot_enabled || row.perp_enabled || row.funding_enabled)
        .count();
    let total = response.venues.len();
    let mut runtime = response
        .runtime
        .into_iter()
        .map(|row| (row.venue.clone(), row))
        .collect::<BTreeMap<_, _>>();
    let rows = response
        .venues
        .into_iter()
        .map(|row| {
            let row_runtime = runtime.remove(&row.venue);
            subscription_row(row, row_runtime, action)
        })
        .collect_view();
    let stale_message = stale_problem
        .as_ref()
        .map(|problem| problem_message("行情订阅快照已降级", problem));

    view! {
        <div class="settings-summary-line">
            <strong>"行情订阅"</strong>
            <span>{format!("{enabled}/{total} 场所启用")}</span>
        </div>
        {stale_message.map(|message| view! {
            <em class="settings-message is-error">{message}</em>
        })}
        <div class="table-wrap">
            <table class="clean-table settings-table market-subscription-table">
                <thead>
                    <tr>
                        <th>"交易所"</th>
                        <th>"现货"</th>
                        <th>"永续"</th>
                        <th>"Funding"</th>
                    </tr>
                </thead>
                <tbody>{rows}</tbody>
            </table>
        </div>
        <em class="settings-message">
            {move || action_message("订阅状态已同步", &action.state.get())}
        </em>
    }
    .into_any()
}

fn subscription_row(
    row: VenueMarketSubscription,
    runtime: Option<VenueMarketSubscriptionRuntime>,
    action: MarketSubscriptionUpdateAction,
) -> impl IntoView {
    let runtime = runtime.unwrap_or_default();
    let venue = row.venue;
    let spot_venue = venue.clone();
    let perp_venue = venue.clone();
    let funding_venue = venue.clone();
    let pending = move || action.state.get().is_pending();
    view! {
        <tr>
            <td><strong>{venue.to_ascii_uppercase()}</strong></td>
            <td>{subscription_toggle(
                "现货",
                spot_venue,
                row.spot_enabled,
                runtime.spot,
                action,
                pending,
                SubscriptionField::Spot,
            )}</td>
            <td>{subscription_toggle(
                "永续",
                perp_venue,
                row.perp_enabled,
                runtime.perp,
                action,
                pending,
                SubscriptionField::Perp,
            )}</td>
            <td>{subscription_toggle(
                "Funding",
                funding_venue,
                row.funding_enabled,
                runtime.funding,
                action,
                pending,
                SubscriptionField::Funding,
            )}</td>
        </tr>
    }
}

#[derive(Clone, Copy)]
enum SubscriptionField {
    Spot,
    Perp,
    Funding,
}

fn subscription_toggle(
    label: &'static str,
    venue: String,
    checked: bool,
    runtime: MarketSubscriptionFeedRuntime,
    action: MarketSubscriptionUpdateAction,
    pending: impl Fn() -> bool + Copy + Send + Sync + 'static,
    field: SubscriptionField,
) -> impl IntoView {
    let aria_label = format!("{venue} {label}");
    let runtime_class = format!(
        "market-subscription-runtime {}",
        runtime_state_class(runtime.state)
    );
    let runtime_text = runtime_label(&runtime);
    view! {
        <div class="market-subscription-control">
            <label class="market-subscription-toggle">
                <input
                    type="checkbox"
                    aria-label=aria_label
                    prop:checked=checked
                    prop:disabled=pending
                    on:change=move |event| {
                        let enabled = event_target_checked(&event);
                        action.submit.run(subscription_patch(&venue, field, enabled));
                    }
                />
                <span>{if checked { "已订阅" } else { "已停用" }}</span>
            </label>
            <small class=runtime_class>{runtime_text}</small>
        </div>
    }
}

fn runtime_state_class(state: MarketSubscriptionRuntimeState) -> &'static str {
    match state {
        MarketSubscriptionRuntimeState::Disabled => "is-disabled",
        MarketSubscriptionRuntimeState::Warming => "is-warming",
        MarketSubscriptionRuntimeState::Live => "is-live",
        MarketSubscriptionRuntimeState::Degraded => "is-degraded",
    }
}

fn runtime_label(runtime: &MarketSubscriptionFeedRuntime) -> String {
    match runtime.state {
        MarketSubscriptionRuntimeState::Disabled => "连接已关闭".to_owned(),
        MarketSubscriptionRuntimeState::Warming => "WS 预热中".to_owned(),
        MarketSubscriptionRuntimeState::Live => format!("WS 实时 · {} 行", runtime.rows),
        MarketSubscriptionRuntimeState::Degraded => format!(
            "{} · {} 行",
            runtime
                .source
                .as_deref()
                .map(runtime_source_label)
                .unwrap_or("数据降级"),
            runtime.rows
        ),
    }
}

fn runtime_source_label(source: &str) -> &'static str {
    match source {
        "ws_push" => "WS 降级",
        "rest_cold_start" => "REST 冷启动",
        "rest_baseline" => "REST 基线",
        "local_cache" => "本地缓存",
        _ => "数据降级",
    }
}

fn subscription_patch(
    venue: &str,
    field: SubscriptionField,
    enabled: bool,
) -> MarketSubscriptionPatch {
    let mut patch = MarketSubscriptionPatch {
        venue: venue.to_owned(),
        spot_enabled: None,
        perp_enabled: None,
        funding_enabled: None,
    };
    match field {
        SubscriptionField::Spot => patch.spot_enabled = Some(enabled),
        SubscriptionField::Perp => patch.perp_enabled = Some(enabled),
        SubscriptionField::Funding => patch.funding_enabled = Some(enabled),
    }
    patch
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::action_state::ActionState;

    #[test]
    fn toggle_patch_only_changes_the_selected_feed() {
        let patch = subscription_patch("kraken", SubscriptionField::Spot, false);

        assert_eq!(patch.venue, "kraken");
        assert_eq!(patch.spot_enabled, Some(false));
        assert_eq!(patch.perp_enabled, None);
        assert_eq!(patch.funding_enabled, None);
    }

    #[test]
    fn pending_state_is_reserved_for_transport_mutations() {
        assert!(!ActionState::Idle.is_pending());
        assert!(ActionState::pending("updating").is_pending());
    }
}
