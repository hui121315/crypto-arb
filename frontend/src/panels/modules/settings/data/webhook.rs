use crate::api::ws::{start_webhook_stream_with_state, WsChannelState};
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use crate::state::polling::{use_ws_channel_context_snapshot_fallback, SnapshotFallbackTiming};
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{ApiProblem, WebhookConfigPatch, WebhookRuntimeStatus, WebhookTestRequest};
use std::time::Duration;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::settings) struct WebhookData {
    pub state: RwSignal<LoadState<WebhookRuntimeStatus>>,
    pub action_problem: RwSignal<Option<ApiProblem>>,
    pub pending: RwSignal<bool>,
    pub refreshing: RwSignal<bool>,
    pub message: RwSignal<Option<String>>,
    pub saved: RwSignal<Option<WebhookRuntimeStatus>>,
    pub refresh: Callback<()>,
    pub save: Callback<WebhookConfigPatch>,
    pub update: Callback<WebhookConfigPatch>,
    pub test: Callback<WebhookTestRequest>,
}

pub(in crate::panels::modules::settings) fn use_webhook_data() -> WebhookData {
    let client = use_global().client;
    let state = RwSignal::new(LoadState::Loading);
    let revision = RwSignal::new(0_u64);
    let pending = RwSignal::new(false);
    let refreshing = RwSignal::new(false);
    let initialized = RwSignal::new(false);
    let action_problem = RwSignal::new(None);
    let message = RwSignal::new(None);
    let saved = RwSignal::new(None);
    let channel = RwSignal::new(WsChannelState::new("webhook"));
    let stream = start_webhook_stream_with_state(
        channel,
        move |value| {
            if accept_status(state, value) {
                revision.update(|v| *v = v.wrapping_add(1));
            }
        },
        move |problem| {
            revision.update(|v| *v = v.wrapping_add(1));
            state.update(|current| current.apply_result(Err(problem)));
        },
    );
    on_cleanup(move || stream.cancel());

    let refresh = Callback::new({
        let client = client.clone();
        move |()| {
            if refreshing.get_untracked() || pending.get_untracked() {
                return;
            }
            refreshing.set(true);
            let client = client.clone();
            let anchor = revision.get_untracked();
            let base = client.base_url();
            spawn_local(async move {
                let result = client.webhook_status().await.map_err(|e| e.problem);
                if refreshing.is_disposed() {
                    return;
                }
                refreshing.set(false);
                initialized.set(true);
                if base == client.base_url() && revision.get_untracked() == anchor {
                    apply_read(state, result);
                }
            });
        }
    });
    refresh.run(());
    use_ws_channel_context_snapshot_fallback(
        channel,
        SnapshotFallbackTiming {
            period: Duration::from_secs(5),
            grace: Duration::from_secs(8),
            stale_after: Duration::from_secs(30),
        },
        move || {
            initialized.get_untracked() && !pending.get_untracked() && !refreshing.get_untracked()
        },
        {
            let client = client.clone();
            move || {
                refreshing.set(true);
                let client = client.clone();
                let anchor = (client.base_url(), revision.get_untracked());
                async move { (anchor, client.webhook_status().await.map_err(|e| e.problem)) }
            }
        },
        {
            let client = client.clone();
            move |(base, anchor), result| {
                refreshing.set(false);
                if base == client.base_url() && revision.get_untracked() == anchor {
                    apply_read(state, result);
                }
            }
        },
    );

    let submit = Callback::new({
        let client = client.clone();
        move |(patch, from_form): (WebhookConfigPatch, bool)| {
            if pending.get_untracked() || !matches!(state.get_untracked(), LoadState::Ready(_)) {
                return;
            }
            pending.set(true);
            revision.update(|v| *v = v.wrapping_add(1));
            action_problem.set(None);
            message.set(None);
            let client = client.clone();
            spawn_local(async move {
                let result = client.update_webhook_config(&patch).await;
                if pending.is_disposed() {
                    return;
                }
                pending.set(false);
                revision.update(|v| *v = v.wrapping_add(1));
                match result {
                    Ok(status) => {
                        accept_status(state, status.clone());
                        if from_form {
                            saved.set(Some(status));
                        }
                        message.set(Some("Webhook 配置已保存，以后端回执为准".into()));
                    }
                    Err(error) => {
                        state.update(|current| current.apply_result(Err(error.problem.clone())));
                        action_problem.set(Some(error.problem));
                        refresh.run(());
                    }
                }
            });
        }
    });
    let test = Callback::new(move |request| {
        if pending.get_untracked() || !matches!(state.get_untracked(), LoadState::Ready(_)) {
            return;
        }
        pending.set(true);
        action_problem.set(None);
        message.set(None);
        let client = client.clone();
        spawn_local(async move {
            let result = client.test_webhook(&request).await;
            if pending.is_disposed() {
                return;
            }
            pending.set(false);
            match result {
                Err(error) => action_problem.set(Some(error.problem)),
                Ok(()) => message.set(Some("测试请求已受理；实际送达以投递回执为准".into())),
            }
        });
    });
    WebhookData {
        state,
        action_problem,
        pending,
        refreshing,
        message,
        saved,
        refresh,
        save: Callback::new(move |patch| submit.run((patch, true))),
        update: Callback::new(move |patch| submit.run((patch, false))),
        test,
    }
}

fn accept_status(
    state: RwSignal<LoadState<WebhookRuntimeStatus>>,
    value: WebhookRuntimeStatus,
) -> bool {
    if state.with_untracked(|state| {
        state
            .value()
            .is_some_and(|current| current.updated_at_ms > value.updated_at_ms)
    }) {
        return false;
    }
    state.set(LoadState::Ready(value));
    true
}

fn apply_read(
    state: RwSignal<LoadState<WebhookRuntimeStatus>>,
    result: Result<WebhookRuntimeStatus, ApiProblem>,
) {
    match result {
        Ok(value) => {
            accept_status(state, value);
        }
        Err(problem) => state.update(|current| current.apply_result(Err(problem))),
    }
}
