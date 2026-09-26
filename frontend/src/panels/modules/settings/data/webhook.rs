use super::{validate_setting_response, SettingsJournal};
use super::resources::settings_read;
use crate::api::rest::with_mutation_timeout;
use crate::api::ws::{start_webhook_stream_with_state, WsChannelState};
use crate::state::load_state::LoadState;
use crate::state::polling::use_ws_channel_fallback_polling;
use crate::state::read_scope::{ReadScope, ScopedRead};
use gloo_timers::callback::Interval;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{
    ActionRunKind, ActionRunStatus, ApiProblem, WebhookConfigPatch, WebhookRuntimeStatus,
    WebhookTestRequest,
};
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
    pub test_runtime: crate::panels::shared::webhook_test::WebhookTestRuntime,
    pub journal: SettingsJournal,
    pub recheck: Callback<()>,
    pub recovered: RwSignal<u64>,
    revision: RwSignal<u64>,
    read_epoch: RwSignal<u64>,
    active: RwSignal<bool>,
    initialized: RwSignal<bool>,
    read: ScopedRead,
}

pub(in crate::panels::modules::settings) fn create_webhook_data() -> WebhookData {
    let test_runtime = expect_context::<crate::panels::shared::webhook_test::WebhookTestRuntime>();
    let journal = test_runtime.config;
    let state = RwSignal::new(LoadState::Loading);
    let revision = RwSignal::new(0_u64);
    let read_epoch = RwSignal::new(0_u64);
    let active = RwSignal::new(false);
    let pending = RwSignal::new(false);
    let refreshing = RwSignal::new(false);
    let initialized = RwSignal::new(false);
    let action_problem = RwSignal::new(None);
    let message = RwSignal::new(None);
    let saved = RwSignal::new(None);
    let recovered = RwSignal::new(0_u64);
    let scope = ReadScope::new(|| {});
    let read = scope.request();
    let refresh = Callback::new({
        move |()| {
            if !active.get_untracked() || refreshing.get_untracked() || pending.get_untracked() {
                return;
            }
            refreshing.set(true);
            let operation = journal.epoch.get_untracked();
            let anchor = revision.get_untracked();
            let epoch = read_epoch.get_untracked();
            read.run(|client| async move {
                settings_read(client.webhook_status()).await.map_err(|e| e.problem)
            }, move |result| {
                if read_epoch.try_get_untracked() != Some(epoch) {
                    return;
                }
                refreshing.set(false);
                initialized.set(true);
                if journal.current(operation) && revision.get_untracked() == anchor {
                    apply_read(state, result);
                }
            });
        }
    });
    let recheck = journal.recheck(Callback::new(move |run: shared_types::ActionRun| {
        revision.update(|v| *v = v.wrapping_add(1));
        state.set(LoadState::Loading);
        saved.set(None);
        action_problem.set(run.problem);
        message.set(Some(
            if run.status == ActionRunStatus::Succeeded {
                recovered.update(|v| *v = v.wrapping_add(1));
                "已核对：上次配置保存成功"
            } else {
                "已核对：上次配置保存失败"
            }
            .into(),
        ));
        spawn_local(async move {
            refresh.run(());
        });
    }));
    Effect::new(move |_| {
        journal.connection.track();
        scope.track();
        read.cancel();
        revision.update(|v| *v = v.wrapping_add(1));
        state.set(LoadState::Loading);
        pending.set(false);
        refreshing.set(false);
        saved.set(None);
        message.set(None);
        action_problem.set(None);
        refresh.run(());
    });
    let submit = Callback::new(move |(patch, from_form): (WebhookConfigPatch, bool)| {
        if pending.get_untracked()
            || test_runtime.pending.get_untracked()
            || journal.locked()
            || !matches!(state.get_untracked(), LoadState::Ready(status) if status.configuration_problem.is_none())
        {
            return;
        }
        let Some(attempt) = journal.begin(
            ActionRunKind::WebhookConfigUpdate,
            "webhook-delivery".into(),
        ) else {
            return;
        };
        let epoch = journal.epoch.get_untracked();
        read.cancel();
        refreshing.set(false);
        pending.set(true);
        revision.update(|v| *v = v.wrapping_add(1));
        action_problem.set(None);
        message.set(None);
        saved.set(None);
        let client = journal.client();
        spawn_local(async move {
            let result = with_mutation_timeout(
                "保存 Webhook 配置",
                client.update_webhook_config_with_context(&patch, &attempt.context),
            )
            .await
            .and_then(|response| {
                validate_setting_response(&attempt, &response)?;
                Ok(response)
            });
            if !journal.current(epoch) {
                return;
            }
            pending.set(false);
            revision.update(|v| *v = v.wrapping_add(1));
            match result {
                Ok(status) => {
                    accept_status(state, status);
                    if journal.resolve(&attempt) {
                        if from_form {
                            saved.set(state.with_untracked(|current| current.value().cloned()));
                        }
                        message.set(Some("Webhook 配置已保存，以后端处理结果为准".into()));
                    }
                    journal.busy.set(false);
                }
                Err(error) => {
                    journal.failed(&attempt, &error);
                    action_problem.set(Some(error.problem));
                    refresh.run(());
                }
            }
        });
    });
    let test = test_runtime.send;
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
        test_runtime,
        journal,
        recheck,
        recovered,
        revision,
        read_epoch,
        active,
        initialized,
        read,
    }
}

pub(in crate::panels::modules::settings) fn use_webhook_data(data: WebhookData) -> WebhookData {
    let WebhookData {
        state,
        revision,
        read_epoch,
        active,
        initialized,
        refreshing,
        pending,
        read,
        ..
    } = data;
    let epoch = read_epoch.get_untracked().wrapping_add(1);
    read_epoch.set(epoch);
    active.set(true);
    initialized.set(state.with_untracked(|state| state.value().is_some()));
    refreshing.set(false);
    let channel = RwSignal::new(WsChannelState::new("webhook"));
    let stream = start_webhook_stream_with_state(
        channel,
        move |value| {
            if read_epoch.try_get_untracked() == Some(epoch) && accept_status(state, value) {
                read.cancel();
                refreshing.set(false);
                initialized.set(true);
                revision.update(|v| *v = v.wrapping_add(1));
            }
        },
        move |problem| {
            if read_epoch.try_get_untracked() == Some(epoch) {
                revision.update(|v| *v = v.wrapping_add(1));
                state.update(|current| current.apply_result(Err(problem)));
            }
        },
    );
    on_cleanup(move || {
        stream.cancel();
        if read_epoch.try_get_untracked() == Some(epoch) {
            read.cancel();
            read_epoch.set(epoch.wrapping_add(1));
            active.set(false);
            refreshing.set(false);
        }
    });
    data.refresh.run(());
    let fallback = use_ws_channel_fallback_polling(channel, Duration::from_secs(8), Duration::from_secs(30));
    let tick = RwSignal::new(0_u64);
    Effect::new(move |previous: Option<Interval>| previous.unwrap_or_else(|| {
        Interval::new(5_000, move || { tick.try_update(|v| *v = v.wrapping_add(1)); })
    }));
    Effect::new(move |_| {
        tick.get();
        if initialized.get_untracked() && fallback.get_untracked() && !pending.get_untracked() {
            data.refresh.run(());
        }
    });
    data
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
