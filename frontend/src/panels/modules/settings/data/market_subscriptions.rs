use crate::api::rest::MutationRequestContext;
use crate::state::action_state::ActionState;
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{MarketSubscriptionPatch, MarketSubscriptionsResponse};

use super::resources::SettingsResource;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::settings) struct MarketSubscriptionData {
    pub state: SettingsResource<MarketSubscriptionsResponse>,
    pub action: RwSignal<ActionState>,
    pub refreshing: RwSignal<bool>,
    pub refresh: Callback<()>,
    pub submit: Callback<MarketSubscriptionPatch>,
}

pub(in crate::panels::modules::settings) fn use_market_subscriptions() -> MarketSubscriptionData {
    let client = use_global().client;
    let state = RwSignal::new(LoadState::Loading);
    let action = RwSignal::new(ActionState::Idle);
    let refreshing = RwSignal::new(false);
    let revision = RwSignal::new(0_u64);
    let refresh = Callback::new({
        let client = client.clone();
        move |()| {
            if refreshing.get_untracked() || action.get_untracked().is_pending() {
                return;
            }
            refreshing.set(true);
            let client = client.clone();
            let base = client.base_url();
            let anchor = revision.get_untracked();
            spawn_local(async move {
                let result = client.market_subscriptions().await.map_err(|e| e.problem);
                if refreshing.is_disposed() {
                    return;
                }
                refreshing.set(false);
                if base == client.base_url() && anchor == revision.get_untracked() {
                    state.update(|state| state.apply_result(result));
                }
            });
        }
    });
    refresh.run(());
    let submit = Callback::new(move |patch: MarketSubscriptionPatch| {
        if action.get_untracked().is_pending()
            || !matches!(state.get_untracked(), LoadState::Ready(_))
        {
            return;
        }
        let context = MutationRequestContext::with_idempotency_key(format!(
            "settings-market-subscription:{}:{}",
            patch.venue.trim().to_ascii_lowercase(),
            crate::api::ws::now_ms()
        ));
        let evidence = context.evidence();
        action.set(ActionState::pending("正在更新行情订阅").with_evidence(evidence.clone()));
        revision.update(|v| *v = v.wrapping_add(1));
        let client = client.clone();
        spawn_local(async move {
            let result = client
                .update_market_subscription_with_context(&patch, &context)
                .await;
            if action.is_disposed() {
                return;
            }
            revision.update(|v| *v = v.wrapping_add(1));
            match result {
                Ok(response) => {
                    state.set(LoadState::Ready(response));
                    action.set(
                        ActionState::succeeded("行情订阅已保存；连接状态以运行证据为准")
                            .with_evidence(evidence),
                    );
                }
                Err(error) => {
                    state.update(|current| current.apply_result(Err(error.problem.clone())));
                    action.set(
                        ActionState::failed("行情订阅更新未确认", error.problem)
                            .with_evidence(evidence),
                    );
                    refresh.run(());
                }
            }
        });
    });
    MarketSubscriptionData {
        state,
        action,
        refreshing,
        refresh,
        submit,
    }
}
