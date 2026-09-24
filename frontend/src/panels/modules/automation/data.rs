use crate::api::rest::ApiClient;
use crate::api::ws::{start_automation_stream_with_state, WsChannelState};
use crate::panels::shared::WebhookTestFeedback;
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
    AutomationControlRequest, AutomationRuntimeStatus, RiskConfigPatch, WebhookRuntimeStatus,
    WebhookTestRequest,
};
use std::time::Duration;

#[derive(Clone, Copy)]
pub(in crate::panels) struct AutomationRuntime {
    status: RwSignal<LoadState<AutomationRuntimeStatus>>,
}

pub(in crate::panels) fn create_automation_runtime() -> AutomationRuntime {
    AutomationRuntime {
        status: RwSignal::new(LoadState::Loading),
    }
}

impl AutomationRuntime {
    pub(in crate::panels) fn module_runtime_state(self) -> ModuleRuntimeState {
        ModuleRuntimeState::from_load_state(&self.status.get())
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
    pub busy: RwSignal<bool>,
    pub reading: RwSignal<bool>,
    pub notice: RwSignal<Option<String>>,
    pub config_saved: RwSignal<u64>,
    pub protection_saved: RwSignal<u64>,
    pub refresh: Callback<()>,
    pub source: RwSignal<&'static str>,
}

#[derive(Clone, Copy)]
struct Requests {
    status: RwSignal<LoadState<AutomationRuntimeStatus>>,
    busy: RwSignal<bool>,
    reading: RwSignal<bool>,
    revision: RwSignal<u64>,
    last_received: RwSignal<i64>,
    notice: RwSignal<Option<String>>,
    source: RwSignal<&'static str>,
}

impl Requests {
    fn accept_read(
        self,
        revision: u64,
        anchor: Option<i64>,
        result: Result<AutomationRuntimeStatus, shared_types::ApiProblem>,
    ) {
        if self.revision.try_get_untracked() != Some(revision) {
            return;
        }
        if result.as_ref().is_ok_and(|next| {
            automation_status_version(&self.status.get_untracked())
                .is_none_or(|old| next.updated_at_ms >= old)
        }) {
            self.last_received
                .set(crate::panels::modules::timestamp::now_ms());
            self.source.set("后台读取");
        }
        self.status
            .update(|state| apply_automation_fetch(state, anchor, result));
    }
}

pub(super) fn use_automation_data(runtime: AutomationRuntime) -> AutomationData {
    let client = use_global().client;
    let status = runtime.status;
    status.update(|state| {
        if state.value().is_some() {
            state.apply_result(Err(shared_types::ApiProblem::new(
                "AUTOMATION_RECONNECTING",
                "正在重新确认自动化状态",
            )));
        }
    });
    let protection = RwSignal::new(LoadState::Loading);
    let protection_notice = RwSignal::new(None);
    let config_saved = RwSignal::new(0_u64);
    let protection_saved = RwSignal::new(0_u64);
    let requests = Requests {
        status,
        busy: RwSignal::new(false),
        reading: RwSignal::new(false),
        revision: RwSignal::new(0),
        last_received: RwSignal::new(0),
        notice: RwSignal::new(None),
        source: RwSignal::new("等待连接"),
    };
    let webhook = use_conditional_polling_load_state(Duration::from_secs(5), || true, {
        let client = client.clone();
        move || {
            let client = client.clone();
            async move { client.webhook_status().await.map_err(|error| error.problem) }
        }
    });
    let webhook_problem = RwSignal::new(None);
    let channel_state = RwSignal::new(WsChannelState::new("automation"));
    let refresh = Callback::new({
        let client = client.clone();
        move |_| {
            if requests.reading.get_untracked() || requests.busy.get_untracked() {
                return;
            }
            requests.reading.set(true);
            let revision = requests.revision.get_untracked();
            let anchor = automation_status_version(&status.get_untracked());
            let client = client.clone();
            spawn_local(async move {
                let result = client
                    .automation_status()
                    .await
                    .map_err(|error| error.problem);
                requests.accept_read(revision, anchor, result);
                requests.reading.try_set(false);
            });
        }
    });
    Effect::new(move |_| {
        refresh.run(());
    });
    let trading = expect_context::<TradingStatusState>();
    Effect::new(move |_| {
        protection.set(match trading.state.get() {
            LoadState::Loading => LoadState::Loading,
            LoadState::Error(problem) => LoadState::Error(problem),
            LoadState::Ready(value) => LoadState::Ready(value.risk.auto_profit_close),
            LoadState::Stale { value, problem } => LoadState::Stale {
                value: value.risk.auto_profit_close,
                problem,
            },
        });
    });
    let handle = start_automation_stream_with_state(
        channel_state,
        move |next| {
            if requests.busy.try_get_untracked().is_none() {
                return;
            }
            if automation_status_version(&status.get_untracked())
                .is_none_or(|old| next.updated_at_ms >= old)
            {
                requests
                    .last_received
                    .set(crate::panels::modules::timestamp::now_ms());
                requests.source.set("WS 推送");
            }
            status.update(|state| apply_automation_snapshot(state, next));
        },
        move |problem| {
            if requests.busy.try_get_untracked().is_some() {
                status.update(|state| state.apply_result(Err(problem)));
            }
        },
    );
    on_cleanup(move || handle.cancel());
    use_ws_channel_context_snapshot_fallback(
        channel_state,
        SnapshotFallbackTiming {
            period: Duration::from_secs(5),
            grace: Duration::from_secs(8),
            stale_after: Duration::from_secs(10),
        },
        move || {
            requests.busy.try_get_untracked() == Some(false)
                && requests.reading.try_get_untracked() == Some(false)
        },
        {
            let client = client.clone();
            move || {
                let client = client.clone();
                let anchor = automation_status_version(&status.get_untracked());
                let revision = requests.revision.get_untracked();
                requests.reading.set(true);
                async move {
                    (
                        (revision, anchor),
                        client
                            .automation_status()
                            .await
                            .map_err(|error| error.problem),
                    )
                }
            }
        },
        move |(revision, anchor), result| {
            requests.accept_read(revision, anchor, result);
            requests.reading.try_set(false);
        },
    );
    let timer = StoredValue::new_local(None::<Interval>);
    Effect::new(move |_| {
        timer.set_value(Some(Interval::new(1_000, move || {
            let Some(last) = requests.last_received.try_get_untracked() else {
                return;
            };
            if last > 0 && crate::panels::modules::timestamp::now_ms().saturating_sub(last) > 15_000
            {
                status.try_update(|state| {
                    if matches!(state, LoadState::Ready(_)) {
                        state.apply_result(Err(shared_types::ApiProblem::new(
                            "AUTOMATION_STATUS_STALE",
                            "自动化状态超过 15 秒未确认",
                        )));
                    }
                });
            }
        })));
    });
    on_cleanup(move || {
        timer.update_value(|timer| {
            timer.take();
        })
    });
    let update = Callback::new({
        let client = client.clone();
        move |patch: AutomatedArbitrageConfigPatch| {
            if requests.busy.get_untracked()
                || !matches!(status.get_untracked(), LoadState::Ready(_))
            {
                return;
            }
            requests.busy.set(true);
            requests.revision.update(|version| *version += 1);
            requests.notice.set(Some("正在保存自动化配置…".into()));
            let saving_draft = patch.capital_usd.is_some();
            let client = client.clone();
            spawn_local(async move {
                let result = client
                    .update_automation_config(&patch)
                    .await
                    .map_err(|error| error.problem);
                if requests.busy.try_get_untracked().is_none() {
                    return;
                }
                let saved = result.is_ok() && saving_draft;
                finish_action(requests, result, "自动化配置已保存");
                if saved {
                    config_saved.update(|version| *version += 1);
                }
                refresh.run(());
            });
        }
    });
    let update_protection = Callback::new({
        let client = client.clone();
        move |auto_profit_close| {
            if requests.busy.get_untracked()
                || !matches!(protection.get_untracked(), LoadState::Ready(_))
            {
                return;
            }
            requests.busy.set(true);
            let client = client.clone();
            protection_notice.set(Some("正在保存退出保护…".to_owned()));
            spawn_local(async move {
                let patch = RiskConfigPatch {
                    max_order_notional: None,
                    max_open_orders: None,
                    max_hedge_imbalance_pct: None,
                    allowed_exchanges: None,
                    allowed_symbols: None,
                    protected_positions: None,
                    auto_profit_close: Some(auto_profit_close),
                };
                let result = client
                    .update_trading_risk_config(&patch)
                    .await
                    .map_err(|error| error.problem);
                if requests.busy.try_get_untracked().is_none() {
                    return;
                }
                match result {
                    Ok(response) => {
                        protection.set(LoadState::Ready(response.risk.auto_profit_close.clone()));
                        trading.accept_receipt(response);
                        protection_saved.update(|version| *version += 1);
                        protection_notice.set(Some("退出保护已保存".to_owned()));
                    }
                    Err(problem) => {
                        protection_notice.set(Some(format!("保存失败：{}", problem.message)));
                    }
                }
                requests.busy.set(false);
            });
        }
    });
    let control = Callback::new({
        let client = client.clone();
        move |request: AutomationControlRequest| {
            if requests.busy.get_untracked()
                || request.action == shared_types::AutomationControlAction::Resume
                    && !matches!(status.get_untracked(), LoadState::Ready(_))
            {
                return;
            }
            requests.busy.set(true);
            requests.revision.update(|version| *version += 1);
            requests.notice.set(Some("正在更新自动化状态…".into()));
            let client = client.clone();
            spawn_local(async move {
                let result = client
                    .control_automation(&request)
                    .await
                    .map_err(|error| error.problem);
                if requests.busy.try_get_untracked().is_none() {
                    return;
                }
                finish_action(requests, result, "自动化控制已确认");
                refresh.run(());
            });
        }
    });
    let webhook_feedback = WebhookTestFeedback {
        pending: RwSignal::new(false),
        message: RwSignal::new(None),
    };
    let test_webhook = webhook_test_callback(client, webhook_problem, webhook_feedback);
    AutomationData {
        status,
        protection,
        protection_notice,
        webhook,
        webhook_problem,
        update,
        update_protection,
        control,
        test_webhook,
        webhook_feedback,
        busy: requests.busy,
        reading: requests.reading,
        notice: requests.notice,
        config_saved,
        protection_saved,
        refresh,
        source: requests.source,
    }
}

fn finish_action(
    requests: Requests,
    result: Result<AutomationRuntimeStatus, shared_types::ApiProblem>,
    success: &str,
) {
    requests.notice.set(Some(result.as_ref().map_or_else(
        |error| format!("操作未确认：{}", error.message),
        |_| success.to_owned(),
    )));
    if result.is_ok() {
        requests
            .last_received
            .set(crate::panels::modules::timestamp::now_ms());
        requests.source.set("操作回执");
    }
    requests
        .status
        .update(|state| apply_automation_action(state, result));
    requests.busy.set(false);
}

fn automation_status_version(state: &LoadState<AutomationRuntimeStatus>) -> Option<i64> {
    state.value().map(|status| status.updated_at_ms)
}

fn apply_automation_snapshot(
    state: &mut LoadState<AutomationRuntimeStatus>,
    next: AutomationRuntimeStatus,
) {
    if automation_status_version(state).is_some_and(|current| current > next.updated_at_ms) {
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
        Ok(next) => apply_automation_snapshot(state, next),
        Err(problem) => state.apply_result(Err(problem)),
    }
}

fn webhook_test_callback(
    client: ApiClient,
    webhook_problem: RwSignal<Option<shared_types::ApiProblem>>,
    feedback: WebhookTestFeedback,
) -> Callback<WebhookTestRequest> {
    Callback::new(move |request| {
        if feedback.pending.get_untracked() {
            return;
        }
        feedback.pending.set(true);
        feedback.message.set(None);
        let client = client.clone();
        webhook_problem.set(None);
        spawn_local(async move {
            let result = client.test_webhook(&request).await;
            if feedback.pending.try_get_untracked().is_none() {
                return;
            }
            feedback.pending.set(false);
            match result {
                Err(error) => webhook_problem.set(Some(error.problem)),
                Ok(()) => feedback
                    .message
                    .set(Some("测试请求已受理；实际送达以最近投递回执为准".into())),
            }
        });
    })
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
                busy: RwSignal::new(false), reading: RwSignal::new(false), revision: RwSignal::new(1),
                last_received: RwSignal::new(20), notice: RwSignal::new(None), source: RwSignal::new("操作回执") };
            let mut old = status_at(20);
            old.config.enabled = true;
            requests.accept_read(0, Some(20), Ok(old));
            requests.accept_read(0, Some(20), Err(shared_types::ApiProblem::new("TIMEOUT", "old read")));
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
