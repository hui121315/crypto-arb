use crate::api::rest::ApiClient;
use crate::api::ws::{start_automation_stream_with_state, WsChannelState};
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use crate::state::module_runtime::ModuleRuntimeState;
use crate::state::polling::{
    use_conditional_polling_load_state, use_ws_channel_context_snapshot_fallback,
    SnapshotFallbackTiming,
};
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
}

pub(super) fn use_automation_data(runtime: AutomationRuntime) -> AutomationData {
    let client = use_global().client;
    let status = runtime.status;
    let protection = RwSignal::new(LoadState::Loading);
    let protection_notice = RwSignal::new(None);
    let webhook = use_conditional_polling_load_state(Duration::from_secs(5), || true, {
        let client = client.clone();
        move || {
            let client = client.clone();
            async move { client.webhook_status().await.map_err(|error| error.problem) }
        }
    });
    let webhook_problem = RwSignal::new(None);
    let channel_state = RwSignal::new(WsChannelState::new("automation"));
    let seed_client = client.clone();
    Effect::new(move |_| {
        let client = seed_client.clone();
        let anchor = automation_status_version(&status.get_untracked());
        spawn_local(async move {
            let result = client
                .automation_status()
                .await
                .map_err(|error| error.problem);
            status.update(|state| apply_automation_fetch(state, anchor, result));
        });
    });
    let protection_client = client.clone();
    Effect::new(move |_| {
        let client = protection_client.clone();
        spawn_local(async move {
            let result = client
                .trading_status()
                .await
                .map(|response| response.risk.auto_profit_close)
                .map_err(|error| error.problem);
            protection.update(|state| state.apply_result(result));
        });
    });
    let handle = start_automation_stream_with_state(
        channel_state,
        move |next| status.update(|state| apply_automation_snapshot(state, next)),
        move |problem| status.update(|state| state.apply_result(Err(problem))),
    );
    on_cleanup(move || handle.cancel());
    use_ws_channel_context_snapshot_fallback(
        channel_state,
        SnapshotFallbackTiming {
            period: Duration::from_secs(5),
            grace: Duration::from_secs(8),
            stale_after: Duration::from_secs(10),
        },
        || true,
        {
            let client = client.clone();
            move || {
                let client = client.clone();
                let anchor = automation_status_version(&status.get_untracked());
                async move {
                    (
                        anchor,
                        client
                            .automation_status()
                            .await
                            .map_err(|error| error.problem),
                    )
                }
            }
        },
        move |anchor, result| {
            status.update(|state| apply_automation_fetch(state, anchor, result));
        },
    );
    let update = Callback::new({
        let client = client.clone();
        move |patch| {
            let client = client.clone();
            spawn_local(async move {
                let result = client
                    .update_automation_config(&patch)
                    .await
                    .map_err(|error| error.problem);
                status.update(|state| apply_automation_action(state, result));
            });
        }
    });
    let update_protection = Callback::new({
        let client = client.clone();
        move |auto_profit_close| {
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
                match client
                    .update_trading_risk_config(&patch)
                    .await
                    .map_err(|error| error.problem)
                {
                    Ok(response) => {
                        protection.set(LoadState::Ready(response.risk.auto_profit_close));
                        protection_notice.set(Some("退出保护已保存".to_owned()));
                    }
                    Err(problem) => {
                        protection_notice.set(Some(format!("保存失败：{}", problem.message)));
                        protection.update(|state| state.apply_result(Err(problem)));
                    }
                }
            });
        }
    });
    let control = Callback::new({
        let client = client.clone();
        move |request| {
            let client = client.clone();
            spawn_local(async move {
                let result = client
                    .control_automation(&request)
                    .await
                    .map_err(|error| error.problem);
                status.update(|state| apply_automation_action(state, result));
            });
        }
    });
    let test_webhook = webhook_test_callback(client, webhook_problem);
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
    }
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
) -> Callback<WebhookTestRequest> {
    Callback::new(move |request| {
        let client = client.clone();
        webhook_problem.set(None);
        spawn_local(async move {
            if let Err(error) = client.test_webhook(&request).await {
                webhook_problem.set(Some(error.problem));
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

    fn status_at(updated_at_ms: i64) -> AutomationRuntimeStatus {
        AutomationRuntimeStatus {
            updated_at_ms,
            ..AutomationRuntimeStatus::default()
        }
    }
}
