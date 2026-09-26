use crate::api::ws::{start_automation_stream_with_state, WsChannelState};
use crate::panels::shared::WebhookTestFeedback;
use crate::panels::shared::operation_journal::OperationJournal;
use crate::panels::modules::settings::tabs::risk_config::RiskConfigRuntime;
use crate::panels::shared::confirmation::ConfirmedAt;
use crate::panels::status_bar::data::VenueOperationHealthState;
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use crate::state::module_runtime::ModuleRuntimeState;
use crate::state::polling::{
    use_conditional_polling_load_state, use_ws_channel_context_snapshot_fallback,
    SnapshotFallbackTiming,
};
use crate::state::trading_status::TradingStatusState;
use gloo_timers::callback::Interval;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{
    AutoProfitCloseConfig, AutoProfitCloseConfigPatch, AutomatedArbitrageConfigPatch,
    AutomationControlRequest, AutomationRuntimeStatus, WebhookRuntimeStatus,
    WebhookTestRequest,
};
use std::time::Duration;
use super::draft::{AutomationConfigDraft, AutomationProtectionDraft};

mod mutations;
use mutations::AutomationMutations;

#[derive(Clone, Copy)]
pub(in crate::panels) struct AutomationRuntime {
    requests: Requests,
    confirmed: RwSignal<LoadState<AutomationRuntimeStatus>>,
    protection: RwSignal<LoadState<AutoProfitCloseConfig>>,
    protection_notice: RwSignal<Option<String>>,
    protection_request: RwSignal<Option<String>>,
    pub(super) draft: AutomationConfigDraft,
    pub(super) protection_draft: AutomationProtectionDraft,
    mutations: AutomationMutations,
}

pub(in crate::panels) fn create_automation_runtime() -> AutomationRuntime {
    let status = RwSignal::new(LoadState::Loading);
    let confirmed = RwSignal::new(LoadState::Loading);
    let journal = OperationJournal::new("automation");
    let needs_current = RwSignal::new(false);
    let blocked = Signal::derive(move || journal.locked() || needs_current.get() || journal.connection.get() != 0);
    let health = expect_context::<VenueOperationHealthState>();
    Effect::new(move |_| {
        let mut next = status.get();
        if blocked.get() {
            next.apply_result(Err(shared_types::ApiProblem::new("AUTOMATION_RESULT_UNKNOWN",
                if journal.connection.get() != 0 { "连接已变化，请刷新页面后核对当前后台" }
                else { "操作结果待核对；当前运行状态尚未确认" })));
        }
        if matches!(next, LoadState::Ready(_)) {
            if let Some(problem) = health.state.with(super::health::worker_problem) {
                next.apply_result(Err(problem));
            }
        }
        if confirmed.get_untracked() != next { confirmed.set(next); }
    });
    let protection = RwSignal::new(LoadState::Loading);
    let config_saved = RwSignal::new(0);
    let protection_saved = RwSignal::new(0);
    let requests = Requests {
            status,
            blocked,
            reading: RwSignal::new(false),
            revision: RwSignal::new(0),
            last_received: RwSignal::new(None),
            notice: RwSignal::new(None),
            source: RwSignal::new("等待连接"),
    };
    let mutations = AutomationMutations::new(requests, journal, needs_current, config_saved);
    let runtime = AutomationRuntime {
        requests,
        confirmed,
        protection,
        protection_notice: RwSignal::new(None),
        protection_request: RwSignal::new(None),
        draft: AutomationConfigDraft::new(status, config_saved),
        protection_draft: AutomationProtectionDraft::new(protection, protection_saved),
        mutations,
    };
    let trading = expect_context::<TradingStatusState>();
    let risk = expect_context::<RiskConfigRuntime>();
    runtime.protection_request.set(risk.kill.journal.pending.get_untracked()
        .filter(|attempt| attempt.kind == shared_types::ActionRunKind::TradingRiskConfigUpdate)
        .map(|attempt| attempt.context.request_id().to_owned()));
    Effect::new(move |_| {
        protection.set(match trading.state.get() {
            LoadState::Loading => LoadState::Loading,
            LoadState::Error(problem) => LoadState::Error(problem),
            LoadState::Ready(value) => LoadState::Ready(value.risk.auto_profit_close),
            LoadState::Stale { value, problem } => LoadState::Stale { value: value.risk.auto_profit_close, problem },
        });
    });
    Effect::new(move |_| {
        let state = risk.save_state();
        let own_request = runtime.protection_request.get();
        if own_request.is_none() || state.evidence().and_then(|value| value.request_id.as_ref()) != own_request.as_ref() {
            return;
        }
        if !matches!(state, shared_types::ActionState::Idle) {
            runtime.protection_notice.set(Some(if matches!(state, shared_types::ActionState::Succeeded { .. }) {
                "退出保护已保存".into()
            } else { state.message("退出保护状态待确认") }));
        }
        if matches!(state, shared_types::ActionState::Succeeded { .. }) {
            runtime.protection_draft.dirty.set(false);
            protection_saved.update(|version| *version += 1);
            runtime.protection_request.set(None);
        }
    });
    Effect::new(move |previous: Option<u64>| {
        let connection = journal.connection.get();
        if previous.is_some_and(|old| old != connection) {
            requests.invalidate_reads();
            requests.status.set(LoadState::Loading);
            requests.last_received.set(None);
            runtime.draft.dirty.set(false);
        }
        connection
    });
    runtime
}

impl AutomationRuntime {
    pub(in crate::panels) fn module_runtime_state(self) -> ModuleRuntimeState {
        if self.mutations.journal.busy.get() {
            ModuleRuntimeState::from_action_state(&shared_types::ActionState::pending("正在更新自动化"))
        } else if self.requests.blocked.get() {
            ModuleRuntimeState::from_action_state(&shared_types::ActionState::accepted("自动化操作待核对"))
        } else {
            ModuleRuntimeState::from_load_state(&self.confirmed.get())
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct AutomationData {
    pub status: RwSignal<LoadState<AutomationRuntimeStatus>>,
    pub protection: RwSignal<LoadState<AutoProfitCloseConfig>>,
    pub protection_notice: RwSignal<Option<String>>,
    pub webhook: RwSignal<LoadState<WebhookRuntimeStatus>>,
    pub webhook_problem: RwSignal<Option<shared_types::ApiProblem>>,
    pub update: Callback<AutomatedArbitrageConfigPatch>,
    pub update_protection: Callback<AutoProfitCloseConfigPatch>,
    pub control: Callback<AutomationControlRequest>,
    pub test_webhook: Callback<WebhookTestRequest>,
    pub webhook_feedback: WebhookTestFeedback,
    pub busy: Signal<bool>,
    pub mutations: StoredValue<AutomationMutations>,
    pub risk: StoredValue<RiskConfigRuntime>,
    pub reading: RwSignal<bool>,
    pub notice: RwSignal<Option<String>>,
    pub refresh: Callback<()>,
    pub source: RwSignal<&'static str>,
}

#[derive(Clone, Copy)]
struct Requests {
    status: RwSignal<LoadState<AutomationRuntimeStatus>>,
    blocked: Signal<bool>,
    reading: RwSignal<bool>,
    revision: RwSignal<u64>,
    last_received: RwSignal<Option<ConfirmedAt>>,
    notice: RwSignal<Option<String>>,
    source: RwSignal<&'static str>,
}

impl Requests {
    fn invalidate_reads(self) {
        self.revision.try_update(|version| *version = version.wrapping_add(1));
        self.reading.try_set(false);
    }

    fn accept_read(
        self,
        revision: u64,
        anchor: Option<i64>,
        started: ConfirmedAt,
        result: Result<AutomationRuntimeStatus, shared_types::ApiProblem>,
    ) {
        if self.revision.try_get_untracked() != Some(revision)
            || self.blocked.try_get_untracked() != Some(false) {
            return;
        }
        self.reading.set(false);
        if started.expired() {
            if self.last_received.get_untracked().is_none_or(ConfirmedAt::expired) {
                self.status.update(|state| state.apply_result(Err(automation_status_stale())));
            }
            return;
        }
        if result.as_ref().is_ok_and(|next| {
            accepts_snapshot(&self.status.get_untracked(), next)
        }) {
            self.last_received.update(|last| {
                *last = Some(last.map_or(started, |previous| previous.latest(started)));
            });
            self.source.set("后台读取");
        }
        self.status
            .update(|state| apply_automation_fetch(state, anchor, result));
    }
}

pub(super) fn use_automation_data(runtime: AutomationRuntime) -> AutomationData {
    let client = use_global().client;
    // Drafts and writes outlive navigation; reads and streams remain page-scoped.
    let requests = runtime.requests;
    let status = requests.status;
    let confirmed = runtime.confirmed;
    let page = RwSignal::new(());
    status.update(|state| {
        if !requests.blocked.get_untracked() && state.value().is_some() {
            state.apply_result(Err(shared_types::ApiProblem::new(
                "AUTOMATION_RECONNECTING",
                "正在重新确认自动化状态",
            )));
        }
    });
    let protection = runtime.protection;
    let protection_notice = runtime.protection_notice;
    let risk = expect_context::<RiskConfigRuntime>();
    let busy = Signal::derive(move || requests.blocked.get() || risk.kill.journal.locked());
    let webhook = use_conditional_polling_load_state(Duration::from_secs(5), || true, {
        let client = client.clone();
        move || {
            let client = client.clone();
            async move { client.webhook_status().await.map_err(|error| error.problem) }
        }
    });
    let webhook_feedback = expect_context::<WebhookTestFeedback>();
    let webhook_problem = webhook_feedback.problem;
    let channel_state = RwSignal::new(WsChannelState::new("automation"));
    let refresh = Callback::new({
        let client = client.clone();
        move |_| {
            if requests.reading.get_untracked() || requests.blocked.get_untracked() {
                return;
            }
            requests.reading.set(true);
            let revision = requests.revision.get_untracked();
            let anchor = automation_status_version(&status.get_untracked());
            let started = ConfirmedAt::now();
            let client = client.clone();
            spawn_local(async move {
                let result = client
                    .automation_status()
                    .await
                    .map_err(|error| error.problem);
                requests.accept_read(revision, anchor, started, result);
            });
        }
    });
    Effect::new(move |_| {
        if !requests.blocked.get() && !matches!(status.get_untracked(), LoadState::Ready(_)) {
            refresh.run(());
        }
    });
    let handle = start_automation_stream_with_state(
        channel_state,
        move |next| {
            if page.try_get_untracked().is_none() || requests.blocked.get_untracked() {
                return;
            }
            if accepts_snapshot(&status.get_untracked(), &next)
            {
                requests
                    .last_received
                    .set(Some(ConfirmedAt::now()));
                requests.source.set("WS 推送");
            }
            status.update(|state| apply_automation_snapshot(state, next));
        },
        move |problem| {
            if page.try_get_untracked().is_some() && !requests.blocked.get_untracked() {
                status.update(|state| state.apply_result(Err(problem)));
            }
        },
    );
    on_cleanup(move || {
        handle.cancel();
        requests.invalidate_reads();
    });
    use_ws_channel_context_snapshot_fallback(
        channel_state,
        SnapshotFallbackTiming {
            period: Duration::from_secs(5),
            grace: Duration::from_secs(8),
            stale_after: Duration::from_secs(10),
        },
        move || {
            requests.blocked.try_get_untracked() == Some(false)
                && requests.reading.try_get_untracked() == Some(false)
        },
        {
            let client = client.clone();
            move || {
                let client = client.clone();
                let anchor = automation_status_version(&status.get_untracked());
                let revision = requests.revision.get_untracked();
                let started = ConfirmedAt::now();
                requests.reading.set(true);
                async move {
                    (
                        (revision, anchor, started),
                        client
                            .automation_status()
                            .await
                            .map_err(|error| error.problem),
                    )
                }
            }
        },
        move |(revision, anchor, started), result| {
            requests.accept_read(revision, anchor, started, result);
        },
    );
    let timer = StoredValue::new_local(None::<Interval>);
    Effect::new(move |_| {
        timer.set_value(Some(Interval::new(1_000, move || {
            let Some(last) = requests.last_received.try_get_untracked() else {
                return;
            };
            if last.is_some_and(ConfirmedAt::expired)
                && status.with_untracked(|state| matches!(state, LoadState::Ready(_)))
            {
                status.try_update(|state| state.apply_result(Err(automation_status_stale())));
            }
        })));
    });
    on_cleanup(move || {
        timer.update_value(|timer| {
            timer.take();
        })
    });
    let update = Callback::new(move |patch: AutomatedArbitrageConfigPatch| {
        if !busy.get_untracked() && matches!(confirmed.get_untracked(), LoadState::Ready(_)) {
            runtime.mutations.update.run(patch);
        }
    });
    let update_protection = Callback::new(move |patch| {
        if !busy.get_untracked() && matches!(protection.get_untracked(), LoadState::Ready(_)) {
            risk.save_exit(patch);
            runtime.protection_request.set(risk.save_state().evidence().and_then(|value| value.request_id.clone()));
        }
    });
    let control = Callback::new(move |request: AutomationControlRequest| {
        if busy.get_untracked() || request.action == shared_types::AutomationControlAction::Resume
            && !matches!(confirmed.get_untracked(), LoadState::Ready(_)) { return; }
        runtime.mutations.control.run(request);
    });
    let test_webhook = webhook_feedback.send;
    AutomationData {
        status: confirmed,
        protection,
        protection_notice,
        webhook,
        webhook_problem,
        update,
        update_protection,
        control,
        test_webhook,
        webhook_feedback,
        busy,
        mutations: StoredValue::new(runtime.mutations),
        risk: StoredValue::new(risk),
        reading: requests.reading,
        notice: requests.notice,
        refresh,
        source: requests.source,
    }
}


fn automation_status_stale() -> shared_types::ApiProblem {
    shared_types::ApiProblem::new("AUTOMATION_STATUS_STALE", "自动化状态超过 15 秒未确认")
}

fn automation_status_version(state: &LoadState<AutomationRuntimeStatus>) -> Option<i64> {
    state.value().map(|status| status.updated_at_ms)
}

fn accepts_snapshot(state: &LoadState<AutomationRuntimeStatus>, next: &AutomationRuntimeStatus) -> bool {
    state.value().is_none_or(|current| next.updated_at_ms > current.updated_at_ms
        || (next.updated_at_ms == current.updated_at_ms && next.config == current.config))
}

fn apply_automation_snapshot(
    state: &mut LoadState<AutomationRuntimeStatus>,
    next: AutomationRuntimeStatus,
) {
    if !accepts_snapshot(state, &next) {
        return;
    }
    *state = LoadState::Ready(next);
}

fn apply_automation_fetch(
    state: &mut LoadState<AutomationRuntimeStatus>,
    request_anchor: Option<i64>,
    result: Result<AutomationRuntimeStatus, shared_types::ApiProblem>,
) {
    match result {
        Ok(next) => apply_automation_snapshot(state, next),
        Err(_) if automation_status_version(state) > request_anchor => {}
        Err(problem) => state.apply_result(Err(problem)),
    }
}

fn apply_automation_action(
    state: &mut LoadState<AutomationRuntimeStatus>,
    result: Result<AutomationRuntimeStatus, shared_types::ApiProblem>,
) {
    match result {
        // A serialized mutation receipt may change configuration within the same millisecond.
        Ok(next) if automation_status_version(state).is_none_or(|at| next.updated_at_ms >= at) => {
            *state = LoadState::Ready(next);
        }
        Ok(_) => {}
        Err(problem) => state.apply_result(Err(problem)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_rest_snapshot_cannot_replace_newer_ws_runtime() {
        let mut state = LoadState::Ready(status_at(20));

        apply_automation_fetch(&mut state, Some(10), Ok(status_at(15)));

        assert_eq!(automation_status_version(&state), Some(20));
    }

    #[test]
    fn obsolete_fetch_error_cannot_degrade_newer_ws_runtime() {
        let mut state = LoadState::Ready(status_at(20));

        apply_automation_fetch(
            &mut state,
            Some(10),
            Err(shared_types::ApiProblem::new("TIMEOUT", "old request")),
        );

        assert!(matches!(state, LoadState::Ready(_)));
    }

    #[test]
    fn action_failure_remains_visible_on_the_current_runtime() {
        let mut state = LoadState::Ready(status_at(20));

        apply_automation_action(
            &mut state,
            Err(shared_types::ApiProblem::new("ACTION_FAILED", "rejected")),
        );

        assert!(matches!(state, LoadState::Stale { .. }));
    }

    #[test]
    fn read_started_before_a_write_cannot_restore_old_config_even_with_equal_time() {
        let owner = Owner::new();
        owner.with(|| {
            let requests = Requests { status: RwSignal::new(LoadState::Ready(status_at(20))),
                blocked: Signal::derive(|| false), reading: RwSignal::new(false), revision: RwSignal::new(1),
                last_received: RwSignal::new(Some(ConfirmedAt::now())), notice: RwSignal::new(None), source: RwSignal::new("操作结果") };
            let mut old = status_at(20);
            old.config.enabled = true;
            requests.accept_read(0, Some(20), ConfirmedAt::now(), Ok(old));
            requests.accept_read(0, Some(20), ConfirmedAt::now(), Err(shared_types::ApiProblem::new("TIMEOUT", "old read")));
            assert!(matches!(requests.status.get_untracked(), LoadState::Ready(status) if !status.config.enabled));
        });
    }

    fn status_at(updated_at_ms: i64) -> AutomationRuntimeStatus {
        AutomationRuntimeStatus {
            updated_at_ms,
            ..AutomationRuntimeStatus::default()
        }
    }
}
