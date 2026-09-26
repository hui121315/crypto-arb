use crate::api::ws::{
    start_onchain_stream_with_state, start_webhook_stream_with_state, WsChannelState,
};
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use crate::state::module_runtime::{ModuleRuntimeState, ModuleRuntimeStatus};
use crate::state::read_scope::bounded_read;
use crate::state::polling::{
    use_conditional_polling_result, use_ws_channel_context_snapshot_fallback, SnapshotFallbackTiming,
};
use futures::future::{AbortHandle, Abortable};
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
#[path = "data/execution_history.rs"]
mod execution_history;
#[path = "data/replenishment.rs"]
mod replenishment;
#[path = "data/preview_context.rs"]
mod preview_context;
#[path = "data/seed.rs"]
mod seed;
#[path = "data/snapshot_state.rs"]
mod snapshot_state;
#[path = "data/freshness.rs"]
mod freshness;
#[path = "data/token_resolution.rs"]
mod token_resolution;
#[path = "data/configuration.rs"]
mod configuration;

pub(super) use configuration::ConfigurationRuntime;

use cross_chain::use_cross_chain;
use execution_history::use_execution_history;
pub(super) use execution_history::ExecutionHistory;
pub(super) use cross_chain::{
    authorization_key, next_recheck_position, next_submit_position, OnchainCrossChainData,
};
pub(in crate::panels::modules::onchain) use form::token_address_ready;
pub(super) use form::use_onchain_form_data;
use replenishment::use_replenishment;
pub(super) use replenishment::OnchainReplenishmentData;
use seed::{apply_webhook_status, start_seed_reads};
use snapshot_state::SnapshotState;
use preview_context::{direction_mismatch, PreviewStamp};
pub(super) use preview_context::PreviewContext;
use token_resolution::resolve_token_with_retry;
pub(in crate::panels::modules::onchain) use token_resolution::same_token_address;
use super::draft::OnchainConfigDraft;

#[derive(Clone, Copy)]
pub(in crate::panels) struct OnchainRuntime {
    snapshots: SnapshotState,
    saving: RwSignal<bool>,
    action_problem: RwSignal<Option<String>>,
    configuration: ConfigurationRuntime,
    pub(super) draft: OnchainConfigDraft,
    cex_pairs: RwSignal<LoadState<OnchainCexPairCatalog>>,
    cex_pair_scope: RwSignal<Option<String>>,
}

pub(in crate::panels) fn create_onchain_runtime() -> OnchainRuntime {
    let state = RwSignal::new(LoadState::Loading);
    let saving = RwSignal::new(false);
    let action_problem = RwSignal::new(None);
    let journal = crate::panels::shared::operation_journal::OperationJournal::new("onchain-config");
    let needs_current = RwSignal::new(false);
    let gate = Signal::derive(move || journal.locked() || needs_current.get() || journal.connection.get() != 0);
    let snapshots = SnapshotState::new(state, saving).with_configuration_gate(gate);
    let configuration = ConfigurationRuntime::new(snapshots, journal, needs_current, action_problem);
    OnchainRuntime {
        snapshots,
        saving,
        action_problem,
        configuration,
        draft: OnchainConfigDraft::new(state),
        cex_pairs: RwSignal::new(LoadState::Loading),
        cex_pair_scope: RwSignal::new(None),
    }
}

impl OnchainRuntime {
    pub(in crate::panels) fn module_runtime_state(self) -> ModuleRuntimeState {
        if self.saving.get() || self.configuration.journal.locked() || self.configuration.needs_current.get()
            || self.configuration.journal.connection.get() != 0 {
            ModuleRuntimeState::from_action_state(&shared_types::ActionState::pending(
                if self.saving.get() || self.configuration.journal.busy.get() {
                    "正在更新链上监控"
                } else { "链上配置结果待核对" },
            ))
        } else {
            let state = self.snapshots.display_state().get();
            if state.problem().is_none() && state.value().is_some_and(|snapshot|
                snapshot.config.enabled && snapshot.quality == shared_types::OnchainComparisonQuality::Stale) {
                ModuleRuntimeState {
                    status: ModuleRuntimeStatus::Stale,
                    problem: Some(ApiProblem::new("ONCHAIN_QUOTES_STALE", "链上套利报价已过期，旧价格仅供参考")
                        .with_source("frontend.onchain.freshness")),
                    pending_label: None,
                }
            } else { ModuleRuntimeState::from_load_state(&state) }
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct OnchainData {
    pub state: RwSignal<LoadState<OnchainComparisonSnapshot>>,
    snapshots: SnapshotState,
    pub transport: RwSignal<WsChannelState>,
    pub action_problem: RwSignal<Option<String>>,
    pub saving: Signal<bool>,
    pub configuration: ConfigurationRuntime,
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
    pub preview: PreviewContext,
    pub history: ExecutionHistory,
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
    preview: PreviewContext,
    history: ExecutionHistory,
    execution_context: RwSignal<Option<PreviewStamp>>,
    approval_context: RwSignal<Option<PreviewStamp>>,
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
    pub draft: OnchainConfigDraft,
    pub current_address: RwSignal<String>,
    pub revision: RwSignal<u64>,
    pub requested_revision: u64,
    pub on_resolved: Callback<OnchainTokenResolution>,
    pub active: Arc<AtomicBool>,
}

impl TokenResolveCommand {
    fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    fn is_current(&self) -> bool {
        let current = form::token_identity_request(
            self.draft,
            self.draft.chain.get_untracked(),
            self.current_address.get_untracked(),
        );
        self.request
            .chain
            .eq_ignore_ascii_case(current.chain.trim())
            && same_token_address(&current.chain, &self.request.address, &current.address)
            && self.request.custom_rpc_url == current.custom_rpc_url
            && self.revision.get_untracked() == self.requested_revision
    }
}

impl OnchainData {
    pub(super) fn current_state(self) -> LoadState<OnchainComparisonSnapshot> {
        self.snapshots.current_state()
    }

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
        self.form.base_identity_revision.update(|value| *value = value.wrapping_add(1));
        self.form.quote_identity_revision.update(|value| *value = value.wrapping_add(1));
        self.form.base_identity.set(TokenResolution::Idle);
        self.form.quote_identity.set(TokenResolution::Idle);
    }
}

fn snapshot_refresher(
    client: crate::api::rest::ApiClient,
    snapshots: SnapshotState,
    action_problem: RwSignal<Option<String>>,
) -> Callback<()> {
    let active = StoredValue::new(None::<AbortHandle>);
    on_cleanup(move || {
        active.update_value(|slot| {
            if let Some(abort) = slot.take() {
                abort.abort();
            }
        });
    });
    Callback::new(move |_| {
        let Some(epoch) = snapshots.begin_action() else { return; };
        let client = client.clone();
        let (abort, registration) = AbortHandle::new_pair();
        active.set_value(Some(abort));
        action_problem.set(None);
        spawn_local(async move {
            let Ok(result) = Abortable::new(
                bounded_read(client.refresh_onchain_comparison()), registration,
            ).await else {
                // Release the read after page disposal, without notifying half-disposed controls.
                snapshots.finish_action(epoch);
                return;
            };
            active.set_value(None);
            if !snapshots.finish_action(epoch) { return; }
            if snapshots.read_stamp().is_none() { return; }
            match result {
                Ok(snapshot) => snapshots.apply_refreshed(snapshot),
                Err(problem) => {
                    action_problem.set(Some(format!("{} · {}", problem.message, problem.code)));
                    snapshots.apply_stream(Err(problem));
                }
            }
        });
    })
}

fn transfer_network_refresher(
    client: crate::api::rest::ApiClient,
    snapshots: SnapshotState,
    refreshing: RwSignal<bool>,
    action_problem: RwSignal<Option<String>>,
) -> Callback<()> {
    Callback::new(move |_| {
        if refreshing.try_get_untracked() != Some(false) {
            return;
        }
        let Some(stamp) = snapshots.read_stamp() else { return; };
        let client = client.clone();
        refreshing.set(true);
        action_problem.set(None);
        spawn_local(async move {
            let result = bounded_read(client.refresh_onchain_transfer_networks()).await;
            if refreshing.try_get_untracked().is_none() { return; }
            match result {
                Ok(snapshot) => snapshots.apply_read(stamp, Ok(snapshot)),
                Err(problem) if snapshots.accepts_read(stamp) => action_problem.set(Some(format!("{} · {}", problem.message, problem.code))),
                Err(_) => {},
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
                    leptos::logging::error!("链上代币身份读取失败: {error}");
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
    Callback::new(move |request: OnchainTokenApprovalBuildRequest| {
        if signals.building_approval.try_get_untracked() != Some(false)
            || signals.submitting_approval.try_get_untracked() != Some(false) { return; }
        let Some(stamp) = signals.preview.stamp(request.direction) else { return; };
        let client = client.clone();
        signals.building_approval.set(true);
        signals.approval_build.set(None);
        signals.approval_submit.set(None);
        signals.approval_context.set(None);
        spawn_local(async move {
            let result = client
                .build_onchain_token_approval(&request)
                .await
                .map_err(|error| error.to_string())
                .and_then(|response| if response.direction == request.direction {
                    Ok(response)
                } else { Err(direction_mismatch().message) });
            let _ = signals.building_approval.try_update(|busy| *busy = false);
            if !signals.preview.accepts(stamp) { return; }
            signals.approval_context.set(Some(stamp));
            signals.approval_build.set(Some(result));
        });
    })
}

fn execution_builder(
    client: crate::api::rest::ApiClient,
    signals: OnchainExecutionSignals,
    build_approval: Callback<OnchainTokenApprovalBuildRequest>,
) -> Callback<OnchainExecutionBuildRequest> {
    Callback::new(move |request: OnchainExecutionBuildRequest| {
        if signals.building_execution.try_get_untracked() != Some(false)
            || signals.building_approval.try_get_untracked() != Some(false)
            || signals.submitting_execution.try_get_untracked() != Some(false)
            || signals.submitting_approval.try_get_untracked() != Some(false) { return; }
        let Some(stamp) = signals.preview.stamp(request.direction) else { return; };
        let client = client.clone();
        signals.building_execution.set(true);
        signals.execution_build.set(None);
        signals.approval_build.set(None);
        signals.approval_submit.set(None);
        signals.execution_context.set(None);
        spawn_local(async move {
            let result = client.build_onchain_execution(&request).await;
            let _ = signals.building_execution.try_update(|busy| *busy = false);
            if !signals.preview.accepts(stamp) { return; }
            signals.execution_context.set(Some(stamp));
            match result {
                Ok(response) if response.direction == request.direction => signals.execution_build.set(Some(Ok(response))),
                Ok(_) => signals.execution_build.set(Some(Err(direction_mismatch()))),
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
        });
    })
}

fn execution_submitter(
    client: crate::api::rest::ApiClient,
    signals: OnchainExecutionSignals,
) -> Callback<String> {
    Callback::new(move |build_id: String| {
        if signals.submitting_execution.try_get_untracked() != Some(false)
            || signals.recovery_problem.get_untracked().is_some()
            || !signals.execution_context.get_untracked().is_some_and(|stamp| signals.preview.accepts(stamp))
            || !signals.execution_build.with_untracked(|result| result.as_ref().and_then(|result| result.as_ref().ok())
                .is_some_and(|build| build.build_id == build_id && build.submit_ready && build.valid_until_ms > crate::state::polling::now_ms() as i64))
        {
            return;
        }
        if !signals.history.state.begin_submission(&build_id) { return; }
        let client = client.clone();
        spawn_local(async move {
            let result = client
                .submit_onchain_execution(&OnchainExecutionSubmitRequest { build_id: build_id.clone() })
                .await;
            if signals.submitting_execution.try_get_untracked().is_none() { return; }
            let rejected = result.is_err();
            signals.history.state.finish_submission(&build_id, result);
            if rejected && signals.history.state.pending_build.get_untracked().is_none() {
                signals.execution_build.set(None);
                signals.execution_context.set(None);
            }
            signals.history.refresh.run(());
            let history = client.onchain_token_approval_runs(20).await.map_err(|e| e.to_string());
            let _ = signals.approval_history.try_update(|slot| *slot = Some(history));
        });
    })
}

fn token_approval_submitter(
    client: crate::api::rest::ApiClient,
    signals: OnchainExecutionSignals,
) -> Callback<String> {
    Callback::new(move |approval_id| {
        if signals.submitting_approval.try_get_untracked() != Some(false)
            || !signals.approval_context.get_untracked().is_some_and(|stamp| signals.preview.accepts(stamp))
            || !signals.approval_build.with_untracked(|result| result.as_ref().and_then(|result| result.as_ref().ok())
                .is_some_and(|build| build.approval_id == approval_id && build.submit_ready && build.valid_until_ms > crate::state::polling::now_ms() as i64)) { return; }
        let client = client.clone();
        signals.submitting_approval.set(true);
        signals.approval_submit.set(None);
        spawn_local(async move {
            let result = client
                .submit_onchain_token_approval(&OnchainTokenApprovalSubmitRequest { approval_id })
                .await
                .map_err(|error| error.to_string());
            if signals.submitting_approval.try_get_untracked().is_none() { return; }
            signals.approval_submit.set(Some(result));
            signals.submitting_approval.set(false);
            let history = client.onchain_token_approval_runs(20).await.map_err(|e| e.to_string());
            let _ = signals.approval_history.try_update(|slot| *slot = Some(history));
        });
    })
}

pub(super) fn use_onchain_data(runtime: OnchainRuntime) -> OnchainData {
    let client = use_global().client;
    let transport = RwSignal::new(WsChannelState::new("onchain"));
    let action_problem = runtime.action_problem;
    let configuration = runtime.configuration;
    let saving = Signal::derive(move || runtime.saving.get() || configuration.journal.locked()
        || configuration.needs_current.get() || configuration.journal.connection.get() != 0);
    let snapshots = runtime.snapshots.for_page();
    snapshots.start_clock();
    let transfer_refreshing = RwSignal::new(false);
    let base_identity = RwSignal::new(TokenResolution::Idle);
    let quote_identity = RwSignal::new(TokenResolution::Idle);
    let base_identity_revision = RwSignal::new(0);
    let quote_identity_revision = RwSignal::new(0);
    let cex_pairs = runtime.cex_pairs;
    let cex_pair_scope = runtime.cex_pair_scope;
    // In-flight reads belong to this page; only completed catalogs survive navigation.
    let cex_pair_request_gate = RwSignal::new(None);
    let webhook_status = RwSignal::new(LoadState::Loading);
    let webhook_transport = RwSignal::new(WsChannelState::new("webhook"));
    let history = use_execution_history(&client);
    let preview = PreviewContext::new(snapshots);
    let signals = OnchainExecutionSignals {
        preview,
        history,
        execution_context: RwSignal::new(None),
        approval_context: RwSignal::new(None),
        approval_history: RwSignal::new(None),
        recovery_problem: history.state.problem,
        execution_build: RwSignal::new(None),
        building_execution: RwSignal::new(false),
        execution_submit: history.state.selected,
        submitting_execution: history.state.submitting,
        approval_build: RwSignal::new(None),
        building_approval: RwSignal::new(false),
        approval_submit: RwSignal::new(None),
        submitting_approval: RwSignal::new(false),
    };
    Effect::new(move |_| {
        preview.track_changes();
        signals.execution_build.set(None);
        signals.approval_build.set(None);
        signals.execution_context.set(None);
        signals.approval_context.set(None);
    });
    let cross_chain = use_cross_chain(&client, snapshots);
    let replenishment = use_replenishment(&client, preview);
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
        snapshots,
        webhook_status,
        signals.approval_submit,
        signals.approval_history,
    );
    use_token_approval_run_recovery(&client, signals.approval_submit, signals.approval_history);
    let refresh_approval_history = {
        let client = client.clone();
        let loading = RwSignal::new(false);
        Callback::new(move |()| {
            if loading.try_get_untracked() != Some(false) { return; }
            loading.set(true);
            let client = client.clone();
            spawn_local(async move {
                let client = client.cancelable_reads();
                let result = bounded_read(client.onchain_token_approval_runs(20)).await.map_err(|e| e.message);
                if loading.try_get_untracked().is_none() { return; }
                signals.approval_history.set(Some(result));
                loading.set(false);
            });
        })
    };
    let handle = start_onchain_stream_with_state(
        transport,
        move |snapshot| snapshots.apply_stream(Ok(snapshot)),
        move |problem| snapshots.apply_stream(Err(problem)),
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
    use_ws_channel_context_snapshot_fallback(
        transport,
        SnapshotFallbackTiming {
            period: Duration::from_secs(5),
            grace: Duration::from_secs(8),
            stale_after: Duration::from_secs(10),
        },
        move || snapshots.read_stamp().is_some(),
        {
            let client = client.clone();
            move || {
                let client = client.clone().cancelable_reads();
                let stamp = snapshots.read_stamp();
                async move {
                    let result = bounded_read(client.onchain_comparison()).await;
                    (stamp, result)
                }
            }
        },
        move |stamp, result| {
            if let Some(stamp) = stamp { snapshots.apply_read(stamp, result); }
        },
    );
    let resolve_client = client.clone();
    let approval_build = token_approval_builder(client.clone(), signals);
    let build_execution = execution_builder(client.clone(), signals, approval_build);
    let submit_execution = execution_submitter(client.clone(), signals);
    let submit_approval = token_approval_submitter(client.clone(), signals);
    let refresh_transfer_networks =
        transfer_network_refresher(client.clone(), snapshots, transfer_refreshing, action_problem);
    let refresh = snapshot_refresher(client, snapshots, action_problem);
    let resolve_token = token_resolver(resolve_client, base_identity, quote_identity);
    OnchainData {
        state: snapshots.display_state(),
        snapshots,
        transport,
        action_problem,
        saving,
        configuration,
        transfer_refreshing,
        update: configuration.update,
        refresh,
        refresh_transfer_networks,
        add_batch: configuration.add,
        remove_batch: configuration.remove,
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
            preview,
            history,
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

    pub(super) fn completed_run() -> OnchainExecutionSubmitResponse {
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
            if batch.observed_at_ms >= snapshot.batch.observed_at_ms {
                snapshot.batch = batch;
            }
        }
        LoadState::Loading | LoadState::Error(_) => {}
    });
}
