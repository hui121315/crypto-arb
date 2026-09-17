use crate::api::ws::{
    start_onchain_stream_with_state, start_webhook_stream_with_state, WsChannelState,
};
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use crate::state::module_runtime::ModuleRuntimeState;
use crate::state::polling::{
    use_conditional_polling_result, use_ws_channel_snapshot_fallback, SnapshotFallbackTiming,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{
    ApiProblem, OnchainBatchSnapshot, OnchainCexPairCatalog, OnchainComparisonConfigPatch,
    OnchainComparisonSnapshot, OnchainExecutionBuildRequest, OnchainExecutionBuildResponse,
    OnchainExecutionRunStatus, OnchainExecutionSubmitRequest, OnchainExecutionSubmitResponse,
    OnchainTokenApprovalBuildRequest, OnchainTokenApprovalBuildResponse,
    OnchainTokenApprovalRunStatus, OnchainTokenApprovalSubmitRequest,
    OnchainTokenApprovalSubmitResponse, OnchainTokenIdentity, OnchainTokenIdentityRequest,
    OnchainTokenResolution, WebhookRuntimeStatus,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[path = "data/cross_chain.rs"]
mod cross_chain;
#[path = "data/form.rs"]
mod form;
#[path = "data/replenishment.rs"]
mod replenishment;
#[path = "data/seed.rs"]
mod seed;
#[path = "data/token_resolution.rs"]
mod token_resolution;

use cross_chain::use_cross_chain;
pub(super) use cross_chain::{
    authorization_key, next_recheck_position, next_submit_position, OnchainCrossChainData,
};
pub(in crate::panels::modules::onchain) use form::token_address_ready;
pub(super) use form::use_onchain_form_data;
use replenishment::use_replenishment;
pub(super) use replenishment::OnchainReplenishmentData;
use seed::{apply_webhook_status, start_seed_reads};
use token_resolution::resolve_token_with_retry;

#[derive(Clone, Copy)]
pub(in crate::panels) struct OnchainRuntime {
    state: RwSignal<LoadState<OnchainComparisonSnapshot>>,
    cex_pairs: RwSignal<LoadState<OnchainCexPairCatalog>>,
    cex_pair_scope: RwSignal<Option<String>>,
    cex_pair_request_gate: RwSignal<Option<(String, u64)>>,
}

pub(in crate::panels) fn create_onchain_runtime() -> OnchainRuntime {
    OnchainRuntime {
        state: RwSignal::new(LoadState::Loading),
        cex_pairs: RwSignal::new(LoadState::Loading),
        cex_pair_scope: RwSignal::new(None),
        cex_pair_request_gate: RwSignal::new(None),
    }
}

impl OnchainRuntime {
    pub(in crate::panels) fn module_runtime_state(self) -> ModuleRuntimeState {
        ModuleRuntimeState::from_load_state(&self.state.get())
    }
}

#[derive(Clone, Copy)]
pub(super) struct OnchainData {
    pub state: RwSignal<LoadState<OnchainComparisonSnapshot>>,
    pub transport: RwSignal<WsChannelState>,
    pub action_problem: RwSignal<Option<String>>,
    pub saving: RwSignal<bool>,
    pub transfer_refreshing: RwSignal<bool>,
    pub update: Callback<OnchainComparisonConfigPatch>,
    pub refresh: Callback<()>,
    pub refresh_transfer_networks: Callback<()>,
    pub add_batch: Callback<OnchainComparisonConfigPatch>,
    pub remove_batch: Callback<String>,
    pub webhook_status: RwSignal<LoadState<WebhookRuntimeStatus>>,
    pub webhook_transport: RwSignal<WsChannelState>,
    pub form: OnchainFormData,
    pub execution: OnchainExecutionData,
    pub replenishment: OnchainReplenishmentData,
}

#[derive(Clone, Copy)]
pub(super) struct OnchainFormData {
    pub base_identity: RwSignal<TokenResolution>,
    pub quote_identity: RwSignal<TokenResolution>,
    pub base_identity_revision: RwSignal<u64>,
    pub quote_identity_revision: RwSignal<u64>,
    pub cex_pairs: RwSignal<LoadState<OnchainCexPairCatalog>>,
    pub cex_pair_scope: RwSignal<Option<String>>,
    pub cex_pair_request_gate: RwSignal<Option<(String, u64)>>,
    pub resolve_token: Callback<TokenResolveCommand>,
}

#[derive(Clone, Copy)]
pub(super) struct OnchainExecutionData {
    pub selected_replenishment: RwSignal<Vec<String>>,
    pub selected_approvals: RwSignal<Vec<String>>,
    pub approval_history: RwSignal<Option<Result<shared_types::OnchainTokenApprovalRunsResponse, String>>>,
    pub refresh_approval_history: Callback<()>,
    pub cross_chain: OnchainCrossChainData,
    pub execution_build: RwSignal<Option<Result<OnchainExecutionBuildResponse, ApiProblem>>>,
    pub building_execution: RwSignal<bool>,
    pub build_execution: Callback<OnchainExecutionBuildRequest>,
    pub execution_submit: RwSignal<Option<Result<OnchainExecutionSubmitResponse, String>>>,
    pub submitting_execution: RwSignal<bool>,
    pub submit_execution: Callback<String>,
    pub recovery_problem: RwSignal<Option<String>>,
    pub approval_build: RwSignal<Option<Result<OnchainTokenApprovalBuildResponse, String>>>,
    pub building_approval: RwSignal<bool>,
    pub approval_submit: RwSignal<Option<Result<OnchainTokenApprovalSubmitResponse, String>>>,
    pub submitting_approval: RwSignal<bool>,
    pub submit_approval: Callback<String>,
}

#[derive(Clone, Copy)]
struct OnchainExecutionSignals {
    approval_history: RwSignal<Option<Result<shared_types::OnchainTokenApprovalRunsResponse, String>>>,
    recovery_problem: RwSignal<Option<String>>,
    execution_build: RwSignal<Option<Result<OnchainExecutionBuildResponse, ApiProblem>>>,
    building_execution: RwSignal<bool>,
    execution_submit: RwSignal<Option<Result<OnchainExecutionSubmitResponse, String>>>,
    submitting_execution: RwSignal<bool>,
    approval_build: RwSignal<Option<Result<OnchainTokenApprovalBuildResponse, String>>>,
    building_approval: RwSignal<bool>,
    approval_submit: RwSignal<Option<Result<OnchainTokenApprovalSubmitResponse, String>>>,
    submitting_approval: RwSignal<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TokenLeg {
    Base,
    Quote,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum TokenResolution {
    Idle,
    Dirty,
    Loading,
    Ready(OnchainTokenIdentity),
    PrecisionOnly(OnchainTokenResolution),
    Error(String),
}

#[derive(Clone)]
pub(super) struct TokenResolveCommand {
    pub leg: TokenLeg,
    pub request: OnchainTokenIdentityRequest,
    pub current_chain: RwSignal<String>,
    pub current_address: RwSignal<String>,
    pub on_resolved: Callback<OnchainTokenResolution>,
    pub active: Arc<AtomicBool>,
}

impl TokenResolveCommand {
    fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    fn is_current(&self) -> bool {
        self.request
            .chain
            .eq_ignore_ascii_case(self.current_chain.get_untracked().trim())
            && self.request.address.trim() == self.current_address.get_untracked().trim()
    }
}

impl OnchainData {
    pub(super) fn token_state(self, leg: TokenLeg) -> RwSignal<TokenResolution> {
        match leg {
            TokenLeg::Base => self.form.base_identity,
            TokenLeg::Quote => self.form.quote_identity,
        }
    }

    pub(super) fn token_revision(self, leg: TokenLeg) -> RwSignal<u64> {
        match leg {
            TokenLeg::Base => self.form.base_identity_revision,
            TokenLeg::Quote => self.form.quote_identity_revision,
        }
    }

    pub(super) fn reset_token_states(self) {
        self.form.base_identity.set(TokenResolution::Idle);
        self.form.quote_identity.set(TokenResolution::Idle);
    }
}

fn config_updater(
    client: crate::api::rest::ApiClient,
    state: RwSignal<LoadState<OnchainComparisonSnapshot>>,
    saving: RwSignal<bool>,
    action_problem: RwSignal<Option<String>>,
) -> Callback<OnchainComparisonConfigPatch> {
    Callback::new(move |patch| {
        let client = client.clone();
        saving.set(true);
        action_problem.set(None);
        spawn_local(async move {
            match client.update_onchain_comparison(&patch).await {
                Ok(snapshot) => state.set(LoadState::Ready(snapshot)),
                Err(error) => action_problem.set(Some(error.to_string())),
            }
            saving.set(false);
        });
    })
}

fn snapshot_refresher(
    client: crate::api::rest::ApiClient,
    state: RwSignal<LoadState<OnchainComparisonSnapshot>>,
    saving: RwSignal<bool>,
    action_problem: RwSignal<Option<String>>,
) -> Callback<()> {
    Callback::new(move |_| {
        let client = client.clone();
        saving.set(true);
        action_problem.set(None);
        spawn_local(async move {
            match client.refresh_onchain_comparison().await {
                Ok(snapshot) => state.set(LoadState::Ready(snapshot)),
                Err(error) => action_problem.set(Some(error.to_string())),
            }
            saving.set(false);
        });
    })
}

fn transfer_network_refresher(
    client: crate::api::rest::ApiClient,
    state: RwSignal<LoadState<OnchainComparisonSnapshot>>,
    refreshing: RwSignal<bool>,
    action_problem: RwSignal<Option<String>>,
) -> Callback<()> {
    Callback::new(move |_| {
        if refreshing.get_untracked() {
            return;
        }
        let client = client.clone();
        refreshing.set(true);
        action_problem.set(None);
        spawn_local(async move {
            match client.refresh_onchain_transfer_networks().await {
                Ok(snapshot) => state.set(LoadState::Ready(snapshot)),
                Err(error) => action_problem.set(Some(error.to_string())),
            }
            refreshing.set(false);
        });
    })
}

fn token_resolver(
    client: crate::api::rest::ApiClient,
    base_identity: RwSignal<TokenResolution>,
    quote_identity: RwSignal<TokenResolution>,
) -> Callback<TokenResolveCommand> {
    Callback::new(move |command: TokenResolveCommand| {
        let client = client.clone();
        let target = match command.leg {
            TokenLeg::Base => base_identity,
            TokenLeg::Quote => quote_identity,
        };
        target.set(TokenResolution::Loading);
        spawn_local(async move {
            let result = resolve_token_with_retry(&client, &command).await;
            if !command.is_active() {
                return;
            }
            match result {
                Ok(resolution) if command.is_current() => {
                    command.on_resolved.run(resolution.clone());
                    target.set(match resolution.identity.clone() {
                        Some(identity) => TokenResolution::Ready(identity),
                        None => TokenResolution::PrecisionOnly(resolution),
                    });
                }
                Err(error) if command.is_current() => {
                    leptos::logging::error!("链上代币身份最终读取失败（已自动重试）: {error}");
                    target.set(TokenResolution::Error(error.to_string()));
                }
                Ok(_) | Err(_) => {}
            }
        });
    })
}

fn token_approval_builder(
    client: crate::api::rest::ApiClient,
    signals: OnchainExecutionSignals,
) -> Callback<OnchainTokenApprovalBuildRequest> {
    Callback::new(move |request| {
        let client = client.clone();
        signals.building_approval.set(true);
        signals.approval_build.set(None);
        signals.approval_submit.set(None);
        spawn_local(async move {
            let result = client
                .build_onchain_token_approval(&request)
                .await
                .map_err(|error| error.to_string());
            signals.approval_build.set(Some(result));
            signals.building_approval.set(false);
        });
    })
}

fn execution_builder(
    client: crate::api::rest::ApiClient,
    signals: OnchainExecutionSignals,
    build_approval: Callback<OnchainTokenApprovalBuildRequest>,
) -> Callback<OnchainExecutionBuildRequest> {
    Callback::new(move |request| {
        let client = client.clone();
        signals.building_execution.set(true);
        signals.execution_build.set(None);
        signals.execution_submit.set(None);
        signals.approval_build.set(None);
        signals.approval_submit.set(None);
        spawn_local(async move {
            match client.build_onchain_execution(&request).await {
                Ok(response) => signals.execution_build.set(Some(Ok(response))),
                Err(error) => {
                    let approval_required = error.problem.code == "ONCHAIN_TOKEN_APPROVAL_REQUIRED";
                    signals.execution_build.set(Some(Err(error.problem)));
                    if approval_required {
                        build_approval.run(OnchainTokenApprovalBuildRequest {
                            direction: request.direction,
                            expected_quote_observed_at_ms: request.expected_quote_observed_at_ms,
                        });
                    }
                }
            }
            signals.building_execution.set(false);
        });
    })
}

fn execution_submitter(
    client: crate::api::rest::ApiClient,
    signals: OnchainExecutionSignals,
) -> Callback<String> {
    Callback::new(move |build_id| {
        if signals.submitting_execution.get_untracked()
            || signals.recovery_problem.get_untracked().is_some()
        {
            return;
        }
        let client = client.clone();
        signals.submitting_execution.set(true);
        signals.execution_submit.set(None);
        spawn_local(async move {
            let result = client
                .submit_onchain_execution(&OnchainExecutionSubmitRequest { build_id })
                .await
                .map_err(|error| error.to_string());
            signals.execution_submit.set(Some(result));
            // Terminal responses do not trigger the pending-run poll. Refresh the write barrier once.
            match client.onchain_execution_runs(20).await {
                Ok(snapshot) => apply_execution_run_snapshot(
                    signals.execution_submit,
                    signals.recovery_problem,
                    snapshot,
                ),
                Err(_) => signals.recovery_problem.set(Some(
                    "执行恢复状态暂未读到；正在重新核对，请勿重复提交".into(),
                )),
            }
            signals.submitting_execution.set(false);
            signals.approval_history.set(Some(client.onchain_token_approval_runs(20).await.map_err(|e| e.to_string())));
        });
    })
}

fn token_approval_submitter(
    client: crate::api::rest::ApiClient,
    signals: OnchainExecutionSignals,
) -> Callback<String> {
    Callback::new(move |approval_id| {
        let client = client.clone();
        signals.submitting_approval.set(true);
        signals.approval_submit.set(None);
        spawn_local(async move {
            let result = client
                .submit_onchain_token_approval(&OnchainTokenApprovalSubmitRequest { approval_id })
                .await
                .map_err(|error| error.to_string());
            signals.approval_submit.set(Some(result));
            signals.submitting_approval.set(false);
            signals.approval_history.set(Some(client.onchain_token_approval_runs(20).await.map_err(|e| e.to_string())));
        });
    })
}

pub(super) fn use_onchain_data(runtime: OnchainRuntime) -> OnchainData {
    let client = use_global().client;
    let state = runtime.state;
    let transport = RwSignal::new(WsChannelState::new("onchain"));
    let action_problem = RwSignal::new(None);
    let saving = RwSignal::new(false);
    let transfer_refreshing = RwSignal::new(false);
    let base_identity = RwSignal::new(TokenResolution::Idle);
    let quote_identity = RwSignal::new(TokenResolution::Idle);
    let base_identity_revision = RwSignal::new(0);
    let quote_identity_revision = RwSignal::new(0);
    let cex_pairs = runtime.cex_pairs;
    let cex_pair_scope = runtime.cex_pair_scope;
    let cex_pair_request_gate = runtime.cex_pair_request_gate;
    let webhook_status = RwSignal::new(LoadState::Loading);
    let webhook_transport = RwSignal::new(WsChannelState::new("webhook"));
    let signals = OnchainExecutionSignals {
        approval_history: RwSignal::new(None),
        recovery_problem: RwSignal::new(None),
        execution_build: RwSignal::new(None),
        building_execution: RwSignal::new(false),
        execution_submit: RwSignal::new(None),
        submitting_execution: RwSignal::new(false),
        approval_build: RwSignal::new(None),
        building_approval: RwSignal::new(false),
        approval_submit: RwSignal::new(None),
        submitting_approval: RwSignal::new(false),
    };
    let cross_chain = use_cross_chain(&client);
    let replenishment = use_replenishment(&client);
    let selected_approvals = RwSignal::new(Vec::<String>::new());
    Effect::new(move |_| {
        let mut claimed = signals.approval_history.get().and_then(Result::ok)
            .map(|history| history.cost_owners.into_keys().collect::<Vec<_>>()).unwrap_or_default();
        if let Some(Ok(run)) = signals.execution_submit.get() {
            claimed.extend(run.approval_costs.into_iter().map(|cost| cost.run.run_id));
        }
        let before = selected_approvals.get_untracked();
        let remaining = before.iter().filter(|id| !claimed.contains(id)).cloned().collect::<Vec<_>>();
        if before != remaining {
            selected_approvals.set(remaining);
            signals.execution_build.set(None);
        }
    });
    start_seed_reads(
        &client,
        state,
        webhook_status,
        signals.execution_submit,
        signals.approval_submit,
        signals.approval_history,
        signals.recovery_problem,
    );
    use_execution_run_recovery(&client, signals.execution_submit, signals.recovery_problem);
    use_token_approval_run_recovery(&client, signals.approval_submit, signals.approval_history);
    let refresh_approval_history = {
        let client = client.clone();
        let loading = RwSignal::new(false);
        Callback::new(move |()| {
            if loading.get_untracked() { return; }
            loading.set(true);
            let client = client.clone();
            spawn_local(async move {
                let result = client.onchain_token_approval_runs(20).await.map_err(|e| e.to_string());
                signals.approval_history.set(Some(result));
                loading.set(false);
            });
        })
    };
    let handle = start_onchain_stream_with_state(
        transport,
        move |snapshot| state.set(LoadState::Ready(snapshot)),
        move |problem| state.update(|current| current.apply_result(Err(problem))),
    );
    let webhook_handle = start_webhook_stream_with_state(
        webhook_transport,
        move |status| apply_webhook_status(webhook_status, status),
        move |problem| webhook_status.update(|current| current.apply_result(Err(problem))),
    );
    on_cleanup(move || {
        handle.cancel();
        webhook_handle.cancel();
    });
    use_ws_channel_snapshot_fallback(
        transport,
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
                async move {
                    client
                        .onchain_comparison()
                        .await
                        .map_err(|error| error.problem)
                }
            }
        },
        move |result| state.update(|current| current.apply_result(result)),
    );
    let update = config_updater(client.clone(), state, saving, action_problem);
    let resolve_client = client.clone();
    let batch_client = client.clone();
    let approval_build = token_approval_builder(client.clone(), signals);
    let build_execution = execution_builder(client.clone(), signals, approval_build);
    let submit_execution = execution_submitter(client.clone(), signals);
    let submit_approval = token_approval_submitter(client.clone(), signals);
    let refresh_transfer_networks =
        transfer_network_refresher(client.clone(), state, transfer_refreshing, action_problem);
    let refresh = snapshot_refresher(client, state, saving, action_problem);
    let resolve_token = token_resolver(resolve_client, base_identity, quote_identity);
    let add_batch = Callback::new({
        let client = batch_client.clone();
        move |patch| {
            let client = client.clone();
            saving.set(true);
            action_problem.set(None);
            spawn_local(async move {
                match client.add_onchain_batch(&patch).await {
                    Ok(batch) => apply_batch_snapshot(state, batch),
                    Err(error) => action_problem.set(Some(error.to_string())),
                }
                saving.set(false);
            });
        }
    });
    let remove_batch = Callback::new(move |item_id: String| {
        let client = batch_client.clone();
        saving.set(true);
        action_problem.set(None);
        spawn_local(async move {
            match client.remove_onchain_batch(item_id).await {
                Ok(batch) => apply_batch_snapshot(state, batch),
                Err(error) => action_problem.set(Some(error.to_string())),
            }
            saving.set(false);
        });
    });
    OnchainData {
        state,
        transport,
        action_problem,
        saving,
        transfer_refreshing,
        update,
        refresh,
        refresh_transfer_networks,
        add_batch,
        remove_batch,
        webhook_status,
        webhook_transport,
        form: OnchainFormData {
            base_identity,
            quote_identity,
            base_identity_revision,
            quote_identity_revision,
            cex_pairs,
            cex_pair_scope,
            cex_pair_request_gate,
            resolve_token,
        },
        execution: OnchainExecutionData {
            selected_replenishment: RwSignal::new(Vec::new()),
            selected_approvals,
            approval_history: signals.approval_history,
            refresh_approval_history,
            cross_chain,
            execution_build: signals.execution_build,
            building_execution: signals.building_execution,
            build_execution,
            execution_submit: signals.execution_submit,
            submitting_execution: signals.submitting_execution,
            submit_execution,
            recovery_problem: signals.recovery_problem,
            approval_build: signals.approval_build,
            building_approval: signals.building_approval,
            approval_submit: signals.approval_submit,
            submitting_approval: signals.submitting_approval,
            submit_approval,
        },
        replenishment,
    }
}

fn use_execution_run_recovery(
    client: &crate::api::rest::ApiClient,
    execution_submit: RwSignal<Option<Result<OnchainExecutionSubmitResponse, String>>>,
    recovery_problem: RwSignal<Option<String>>,
) {
    let poll = use_conditional_polling_result(
        Duration::from_secs(2),
        move || {
            execution_submit
                .try_get_untracked()
                .flatten()
                .is_some_and(|result| {
                    result.is_ok_and(|run| {
                        execution_status_needs_poll(run.status)
                            || settlement_needs_poll(&run, crate::state::polling::now_ms() as i64)
                    })
                })
                || recovery_problem.try_get_untracked().flatten().is_some()
        },
        {
            let client = client.clone();
            move || {
                let client = client.clone();
                async move { client.onchain_execution_runs(20).await }
            }
        },
    );
    Effect::new(move |_| {
        let Some(Ok(snapshot)) = poll.get().and_then(|event| event.take().into_fetched()) else {
            return;
        };
        apply_execution_run_snapshot(execution_submit, recovery_problem, snapshot);
    });
}

fn apply_execution_run_snapshot(
    execution_submit: RwSignal<Option<Result<OnchainExecutionSubmitResponse, String>>>,
    recovery_problem: RwSignal<Option<String>>,
    snapshot: shared_types::OnchainExecutionRunsResponse,
) {
    recovery_problem.set(snapshot.recovery_problem);
    let Some(Ok(current)) = execution_submit.get_untracked() else {
        return;
    };
    if let Some(run) = snapshot
        .rows
        .into_iter()
        .find(|run| run.run_id == current.run_id && run.updated_at_ms >= current.updated_at_ms)
    {
        if run != current {
            execution_submit.set(Some(Ok(run)));
        }
    }
}

fn settlement_needs_poll(run: &OnchainExecutionSubmitResponse, now_ms: i64) -> bool {
    // Local receipt reads only, bounded even when a venue never sends fees.
    now_ms.saturating_sub(run.updated_at_ms) < 120_000
        && now_ms.saturating_sub(run.started_at_ms) < 600_000
        && (run
            .legs
            .iter()
            .filter_map(|leg| leg.settlement.as_ref())
            .any(|receipt| {
                matches!(
                    receipt.status,
                    shared_types::OnchainCexSettlementStatus::PendingFills
                        | shared_types::OnchainCexSettlementStatus::PendingFees
                )
            })
            || run
                .legs
                .iter()
                .filter_map(|leg| leg.chain_settlement.as_ref())
                .any(|receipt| {
                    receipt.status == shared_types::OnchainChainSettlementStatus::Pending
                })
            || run.accounting.as_ref().is_some_and(|accounting| {
                accounting.status
                    == shared_types::OnchainExecutionAccountingStatus::PendingValuation
            }))
}

fn use_token_approval_run_recovery(
    client: &crate::api::rest::ApiClient,
    approval_submit: RwSignal<Option<Result<OnchainTokenApprovalSubmitResponse, String>>>,
    history: RwSignal<Option<Result<shared_types::OnchainTokenApprovalRunsResponse, String>>>,
) {
    let poll = use_conditional_polling_result(
        Duration::from_secs(2),
        move || pending_approval_run_id(approval_submit.try_get_untracked().flatten()).is_some(),
        {
            let client = client.clone();
            move || {
                let client = client.clone();
                async move { client.onchain_token_approval_runs(20).await }
            }
        },
    );
    Effect::new(move |_| {
        let Some(Ok(snapshot)) = poll.get().and_then(|event| event.take().into_fetched()) else {
            return;
        };
        history.set(Some(Ok(snapshot.clone())));
        let Some(run_id) = pending_approval_run_id(approval_submit.get_untracked()) else {
            return;
        };
        if let Some(run) = snapshot.rows.into_iter().find(|run| run.run_id == run_id) {
            approval_submit.set(Some(Ok(run)));
        }
    });
}

fn execution_status_needs_poll(status: OnchainExecutionRunStatus) -> bool {
    matches!(
        status,
        OnchainExecutionRunStatus::Executing | OnchainExecutionRunStatus::AwaitingChainFinality
    )
}

fn pending_approval_run_id(
    result: Option<Result<OnchainTokenApprovalSubmitResponse, String>>,
) -> Option<String> {
    result.and_then(|result| match result {
        Ok(run) if run.status == OnchainTokenApprovalRunStatus::AwaitingFinality || run.receipt_check_pending() => {
            Some(run.run_id)
        }
        Ok(_) | Err(_) => None,
    })
}

#[cfg(test)]
mod execution_recovery_tests {
    use super::*;

    #[test]
    fn execution_accounting_poll_waits_for_saved_valuation_but_remains_bounded() {
        let mut run = completed_run();
        run.accounting = Some(shared_types::OnchainExecutionAccounting {
            status: shared_types::OnchainExecutionAccountingStatus::PendingValuation,
            flows: vec![],
            net_assets: vec![],
            usd_value: None,
            problems: vec![],
        });
        assert!(settlement_needs_poll(&run, 30));
        assert!(!settlement_needs_poll(&run, 600_001));
        run.accounting.as_mut().unwrap().status =
            shared_types::OnchainExecutionAccountingStatus::Valued;
        assert!(!settlement_needs_poll(&run, 30));
    }

    #[test]
    fn active_execution_stages_keep_polling_until_terminal() {
        assert!(execution_status_needs_poll(
            OnchainExecutionRunStatus::Executing
        ));
        assert!(execution_status_needs_poll(
            OnchainExecutionRunStatus::AwaitingChainFinality
        ));
        assert!(!execution_status_needs_poll(
            OnchainExecutionRunStatus::Completed
        ));
        assert!(!execution_status_needs_poll(
            OnchainExecutionRunStatus::Exposed
        ));
    }

    #[test]
    fn cex_settlement_late_fee_poll_is_bounded_and_stops_when_complete() {
        let mut run = completed_run();
        let leg: shared_types::OnchainExecutionLegResult = serde_json::from_value(serde_json::json!({
            "position": 1, "kind": "primary_cex", "status": "filled", "venue": "kraken",
            "symbol": "SOL/USD", "orderId": "order", "transactionId": null,
            "filledQuantity": 1, "message": "filled", "settlement": {
                "basis": {"orderId": "order", "venue": "kraken", "symbol": "SOL/USD", "side": "sell",
                    "baseAsset": "SOL", "quoteAsset": "USD", "confirmedQuantity": 1},
                "status": "pending_fees", "grossBaseAmount": "1", "grossQuoteAmount": "100",
                "debitAmount": null, "creditAmount": null, "fees": [], "fillEventIds": ["fill"],
                "observedAtMs": 20, "problem": "fee pending"
            }
        })).unwrap();
        run.legs.push(leg);
        assert!(settlement_needs_poll(&run, 30));
        assert!(!settlement_needs_poll(&run, 120_020));
        run.updated_at_ms = 600_000;
        assert!(!settlement_needs_poll(&run, 600_001));
        run.legs[0].settlement.as_mut().unwrap().status =
            shared_types::OnchainCexSettlementStatus::Complete;
        assert!(!settlement_needs_poll(&run, 30));
    }

    #[test]
    fn chain_settlement_poll_stops_at_review_or_timeout() {
        let mut run = completed_run();
        let leg: shared_types::OnchainExecutionLegResult = serde_json::from_value(serde_json::json!({
            "position":2,"kind":"chain","status":"confirmed","venue":"solana","symbol":null,"orderId":null,
            "transactionId":"signature","filledQuantity":null,"message":"confirmed","chainSettlement":{
                "basis":{"chain":"solana","wallet":"wallet","transactionId":"signature","assets":{
                    "input":{"symbol":"USDT","address":"mint-a","decimals":6},"output":{"symbol":"USDC","address":"mint-b","decimals":6}},
                    "maximumInputRaw":"100000000","minimumOutputRaw":"104000000"},"status":"pending",
                    "inputAmountRaw":null,"outputAmountRaw":null,"additionalNativeChangeRaw":null,"networkCost":null,
                    "blockRef":null,"observedAtMs":null,"problem":null}})).unwrap();
        run.legs.push(leg);
        assert!(settlement_needs_poll(&run, 30));
        assert!(!settlement_needs_poll(&run, 600001));
        run.legs[0].chain_settlement.as_mut().unwrap().status =
            shared_types::OnchainChainSettlementStatus::ReviewRequired;
        assert!(!settlement_needs_poll(&run, 30));
    }

    fn completed_run() -> OnchainExecutionSubmitResponse {
        OnchainExecutionSubmitResponse {
            run_id: "current".into(),
            build_id: "build".into(),
            status: OnchainExecutionRunStatus::Completed,
            cex_order_id: Some("order".into()),
            cex_order_state: None,
            cex_filled_quantity: Some(1.0),
            chain_transaction_id: Some("tx".into()),
            compensation_order_id: None,
            legs: Vec::new(),
            recovery_actions: Vec::new(),
            replenishment_costs: Vec::new(),
            approval_costs: Vec::new(),
            estimated_net_profit_usd: 1.0,
            remaining_exposure_usd: 0.0,
            quantity_reconciled: false,
            accounting: None,
            message: "completed".into(),
            problem: None,
            started_at_ms: 1,
            updated_at_ms: 20,
        }
    }

    #[test]
    fn terminal_execution_refreshes_recovery_barrier_without_losing_the_result() {
        Owner::new().with(|| {
            let run = completed_run();
            let execution = RwSignal::new(Some(Ok(run.clone())));
            let problem = RwSignal::new(None);
            apply_execution_run_snapshot(
                execution,
                problem,
                shared_types::OnchainExecutionRunsResponse {
                    rows: vec![],
                    observed_at_ms: 30,
                    recovery_problem: Some("journal unavailable".into()),
                },
            );
            assert_eq!(
                problem.get_untracked().as_deref(),
                Some("journal unavailable")
            );
            assert_eq!(execution.get_untracked(), Some(Ok(run.clone())));
            apply_execution_run_snapshot(
                execution,
                problem,
                shared_types::OnchainExecutionRunsResponse {
                    rows: vec![run],
                    observed_at_ms: 40,
                    recovery_problem: None,
                },
            );
            assert!(problem.get_untracked().is_none());
        });
    }

    #[test]
    fn execution_refresh_never_regresses_or_replaces_another_run() {
        Owner::new().with(|| {
            let current = completed_run();
            let execution = RwSignal::new(Some(Ok(current.clone())));
            let problem = RwSignal::new(None);
            let mut old = current.clone();
            old.status = OnchainExecutionRunStatus::Executing;
            old.updated_at_ms = 10;
            let mut unrelated = current.clone();
            unrelated.run_id = "other".into();
            unrelated.updated_at_ms = 30;
            apply_execution_run_snapshot(
                execution,
                problem,
                shared_types::OnchainExecutionRunsResponse {
                    rows: vec![old, unrelated],
                    observed_at_ms: 40,
                    recovery_problem: None,
                },
            );
            assert_eq!(execution.get_untracked(), Some(Ok(current.clone())));
            let mut updated = current;
            updated.updated_at_ms = 50;
            updated.status = OnchainExecutionRunStatus::FinalityUnresolved;
            apply_execution_run_snapshot(
                execution,
                problem,
                shared_types::OnchainExecutionRunsResponse {
                    rows: vec![updated.clone()],
                    observed_at_ms: 50,
                    recovery_problem: None,
                },
            );
            assert_eq!(execution.get_untracked(), Some(Ok(updated)));
        });
    }
}

fn apply_batch_snapshot(
    state: RwSignal<LoadState<OnchainComparisonSnapshot>>,
    batch: OnchainBatchSnapshot,
) {
    state.update(|current| match current {
        LoadState::Ready(snapshot)
        | LoadState::Stale {
            value: snapshot, ..
        } => {
            snapshot.batch = batch;
        }
        LoadState::Loading | LoadState::Error(_) => {}
    });
}
