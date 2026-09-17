use crate::api::rest::MutationRequestContext;
use crate::state::action_state::ActionState;
use crate::state::context::use_global;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{MarketSubscriptionPatch, MarketSubscriptionsResponse};

use super::resources::{bump_refresh, local_refresh_resource, SettingsResource};

#[derive(Clone, Copy)]
pub(in crate::panels::modules::settings) struct MarketSubscriptionUpdateAction {
    pub state: RwSignal<ActionState>,
    pub submit: Callback<MarketSubscriptionPatch>,
}

pub(in crate::panels::modules::settings) fn use_market_subscriptions(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<MarketSubscriptionsResponse> {
    let client = use_global().client;
    local_refresh_resource(refresh_nonce, move || {
        let client = client.clone();
        async move { client.market_subscriptions().await }
    })
}

pub(in crate::panels::modules::settings) fn use_market_subscription_update_action(
    refresh_nonce: RwSignal<u64>,
) -> MarketSubscriptionUpdateAction {
    let client = use_global().client;
    let state = RwSignal::new(ActionState::Idle);
    let submit = Callback::new(move |patch: MarketSubscriptionPatch| {
        if state.get_untracked().is_pending() {
            return;
        }
        let context = MutationRequestContext::with_idempotency_key(format!(
            "settings-market-subscription:{}:{}",
            patch.venue.trim().to_ascii_lowercase(),
            crate::api::ws::now_ms()
        ));
        let pending_evidence = context.evidence();
        state.set(ActionState::pending("正在更新行情订阅").with_evidence(pending_evidence.clone()));
        let client = client.clone();
        spawn_local(async move {
            match client
                .update_market_subscription_with_context(&patch, &context)
                .await
            {
                Ok(_) => {
                    bump_refresh(refresh_nonce);
                    state.set(
                        ActionState::succeeded("行情订阅已更新").with_evidence(pending_evidence),
                    );
                }
                Err(error) => {
                    state.set(
                        ActionState::failed("行情订阅更新失败", error.problem)
                            .with_evidence(pending_evidence),
                    );
                }
            }
        });
    });
    MarketSubscriptionUpdateAction { state, submit }
}
