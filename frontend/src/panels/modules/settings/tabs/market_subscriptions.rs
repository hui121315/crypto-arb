use crate::state::load_state::LoadState;
use crate::state::action_state::ActionState;
use crate::state::module_runtime::ModuleRuntimeState;
use super::super::runtime::{action_health, PaneState};
use leptos::prelude::*;
use shared_types::{
    MarketSubscriptionFeedRuntime, MarketSubscriptionPatch, MarketSubscriptionRuntimeState,
};

use super::super::data::{use_market_subscriptions, MarketSubscriptionData};
use super::{action_message, problem_message};

pub(in crate::panels::modules::settings) fn market_subscriptions_tab(data: MarketSubscriptionData, pane: PaneState) -> impl IntoView {
    let data = use_market_subscriptions(data);
    pane.track(move || ModuleRuntimeState::combine([
        ModuleRuntimeState::from_load_state(&data.state.get()),
        action_health(data.journal, &data.action.get()),
    ]));
    let rows = Memo::new(move |_| {
        data.state.with(|state| {
            state
                .value()
                .map(|value| value.venues.clone())
                .unwrap_or_default()
        })
    });
    view! {
        <div class="settings-stack settings-market-subscriptions">
            {super::super::data::settings_recovery_panel(data.journal, data.recheck)}
            <div class="settings-summary-line">
                <strong>"行情订阅"</strong>
                <span>{move || data.state.with(|state| match state {
                    LoadState::Loading => "正在读取配置".to_owned(),
                    LoadState::Error(_) => "配置不可用".to_owned(),
                    LoadState::Ready(value) | LoadState::Stale { value, .. } => format!("{}{}/{} 场所启用",
                        if matches!(state, LoadState::Stale { .. }) { "上次配置 · " } else { "" },
                        value.venues.iter().filter(|row| row.spot_enabled || row.perp_enabled).count(), value.venues.len()),
                })}</span>
                <button class="icon-button" title="刷新行情订阅" aria-label="刷新行情订阅"
                    disabled=move || data.refreshing.get() || data.action.get().is_pending()
                    on:click=move |_| data.refresh.run(())>"↻"</button>
            </div>
            {move || match data.state.get() {
                LoadState::Loading => Some(view! { <div class="empty-cell">"正在读取行情订阅"</div> }.into_any()),
                LoadState::Error(problem) => Some(view! {
                    <em class="settings-message is-error" role="alert">{problem_message("行情订阅不可用", &problem)}</em>
                }.into_any()),
                LoadState::Stale { problem, .. } => Some(view! {
                    <em class="settings-message is-error" role="alert">{problem_message("读取失败，显示上次配置", &problem)}</em>
                }.into_any()),
                LoadState::Ready(_) => None,
            }}
            <Show when=move || data.state.with(|state| state.value().is_some())>
                <div class="table-wrap">
                    <table class="clean-table settings-table market-subscription-table">
                        <thead><tr><th>"交易所"</th><th>"现货"</th><th>"永续"</th><th>"Funding"</th></tr></thead>
                        <tbody>
                            <For each=move || rows.get() key=|row| row.venue.clone() children=move |row| view! {
                                <tr>
                                    <td><strong>{row.venue.to_ascii_uppercase()}</strong></td>
                                    <td>{subscription_toggle("现货", row.venue.clone(), data, SubscriptionField::Spot)}</td>
                                    <td>{subscription_toggle("永续", row.venue.clone(), data, SubscriptionField::Perp)}</td>
                                    <td>{subscription_toggle("Funding", row.venue, data, SubscriptionField::Funding)}</td>
                                </tr>
                            }/>
                        </tbody>
                    </table>
                </div>
            </Show>
            <Show when=move || data.state.with(|state| state.value().is_some()) || !matches!(data.action.get(), ActionState::Idle)>
            <p class="settings-message" role="status">{move || match data.action.get() {
                ActionState::Idle => "开关为已保存配置，连接状态单独核对",
                ActionState::Pending { .. } => "正在保存订阅配置",
                ActionState::Accepted { .. } => "订阅保存结果待核对",
                ActionState::Succeeded { .. } => "上次订阅配置已保存",
                ActionState::Failed { .. } => "订阅保存未确认，请核对原操作或查看处理结果",
            }}</p>
            </Show>
            <Show when=move || !matches!(data.action.get(), ActionState::Idle)>
                <details class="settings-environment-evidence">
                    <summary>"操作结果"</summary>
                    <p class="state-note">{move || action_message("", &data.action.get())}</p>
                </details>
            </Show>
        </div>
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
    data: MarketSubscriptionData,
    field: SubscriptionField,
) -> impl IntoView {
    let aria_label = format!("{venue} {label}");
    let checked_venue = venue.clone();
    let runtime_venue = venue.clone();
    let dependency_venue = venue.clone();
    let funding_suspended = Memo::new(move |_| {
        matches!(field, SubscriptionField::Funding) && data.state.with(|state| {
            state.value()
                .and_then(|value| value.venues.iter().find(|row| row.venue == dependency_venue))
                .is_some_and(|row| row.funding_enabled && !row.perp_enabled)
        })
    });
    let checked = Memo::new(move |_| {
        data.state.with(|state| {
            state
                .value()
                .and_then(|value| value.venues.iter().find(|row| row.venue == checked_venue))
                .is_some_and(|row| match field {
                    SubscriptionField::Spot => row.spot_enabled,
                    SubscriptionField::Perp => row.perp_enabled,
                    SubscriptionField::Funding => row.funding_enabled,
                })
        })
    });
    let runtime = Memo::new(move |_| {
        data.state.with(|state| {
            state
                .value()
                .and_then(|value| value.runtime.iter().find(|row| row.venue == runtime_venue))
                .map(|row| match field {
                    SubscriptionField::Spot => row.spot.clone(),
                    SubscriptionField::Perp => row.perp.clone(),
                    SubscriptionField::Funding => row.funding.clone(),
                })
        })
    });
    let blocked = Memo::new(move |_| {
        data.journal.locked() || !matches!(data.state.get(), LoadState::Ready(_))
    });
    view! {
        <div class="market-subscription-control">
            <label class="market-subscription-toggle">
                <input type="checkbox" aria-label=aria_label prop:checked=move || checked.get() disabled=move || blocked.get()
                    on:change=move |event| {
                        let enabled = event_target_checked(&event);
                        event_target::<web_sys::HtmlInputElement>(&event).set_checked(checked.get_untracked());
                        if !blocked.get_untracked() { data.submit.run(subscription_patch(&venue, field, enabled)); }
                    }/>
                <span>{move || if funding_suspended.get() { "随永续暂停" } else if checked.get() { "已订阅" } else { "已停用" }}</span>
            </label>
            <small class=move || format!("market-subscription-runtime {}", runtime.get().map(|r| runtime_state_class(r.state)).unwrap_or("is-warming"))>
                {move || if funding_suspended.get() { "需启用永续".into() } else {
                    runtime.get().map(|r| runtime_label(&r)).unwrap_or_else(|| "等待运行数据依据".into())
                }}
            </small>
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
        MarketSubscriptionRuntimeState::Disabled => "订阅已停用".to_owned(),
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
