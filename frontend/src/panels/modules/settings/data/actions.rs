//! Settings 模块提交类 hooks：凭证保存 / 风控保存 / Kill Switch，含幂等重放键。

use crate::api::rest::MutationRequestContext;
use crate::panels::modules::kill_switch_idempotency;
use crate::state::action_state::ActionState;
use crate::state::context::use_global;
use crate::state::trading_status::TradingStatusState;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{ActionRun, ActionRunKind, KillSwitchRequest, RiskConfigPatch};

use super::format::{credential_success_message, kill_switch_success_message};
use super::resources::{bump_refresh, SettingsResource};
use super::use_action_run_recovery;

#[path = "actions/transport.rs"]
mod transport;
use transport::{
    credential_response_evidence, kill_switch_evidence, save_venue_credentials_task,
    select_trading_adapter_task, set_kill_switch_task, trading_status_evidence,
    update_risk_config_task,
};

#[path = "actions/replay.rs"]
mod replay;
pub(in crate::panels::modules::settings) use replay::{
    credential_save_fingerprint, credential_save_replay_key, risk_config_replay_slot,
    should_reuse_credential_replay_key, CredentialSaveReplay, RiskConfigReplay,
};

#[path = "api_base.rs"]
mod api_base;
pub(in crate::panels::modules::settings) use api_base::*;

pub(in crate::panels::modules::settings) struct VenueCredentialSave {
    pub venue: String,
    pub fields: Vec<(String, String)>,
}

#[derive(Clone, Copy)]
pub(in crate::panels::modules::settings) struct VenueCredentialSaveAction {
    pub state: RwSignal<ActionState>,
    pub saved_revision: RwSignal<u64>,
    pub submit: Callback<VenueCredentialSave>,
}

#[derive(Clone, Copy)]
pub(in crate::panels::modules::settings) struct RiskConfigSaveAction {
    pub state: RwSignal<ActionState>,
    pub receipt: RwSignal<Option<shared_types::TradingStatusResponse>>,
    pub submit: Callback<RiskConfigPatch>,
}

#[derive(Clone, Copy)]
pub(in crate::panels::modules::settings) struct KillSwitchAction {
    pub state: RwSignal<ActionState>,
    pub submit: Callback<KillSwitchRequest>,
}

#[derive(Clone, Copy)]
pub(in crate::panels::modules::settings) struct TradingAdapterSelectAction {
    pub state: RwSignal<ActionState>,
    pub submit: Callback<String>,
}

pub(in crate::panels::modules::settings) fn use_trading_adapter_select_action(
    refresh_nonce: RwSignal<u64>,
    adapters: SettingsResource<shared_types::TradingAdaptersResponse>,
) -> TradingAdapterSelectAction {
    let client = use_global().client;
    let shared_status = use_context::<TradingStatusState>();
    let state = RwSignal::new(ActionState::Idle);
    let submit = Callback::new(move |adapter_id: String| {
        if state.get_untracked().is_pending() {
            return;
        }
        let adapter_id = adapter_id.trim().to_owned();
        let context = MutationRequestContext::with_idempotency_key(format!(
            "settings-adapter-select:{adapter_id}:{}",
            crate::api::ws::now_ms()
        ));
        let idempotency_key = context.idempotency_key().unwrap_or_default().to_owned();
        let pending_evidence = context.evidence();
        state.set(ActionState::pending("正在切换执行环境").with_evidence(pending_evidence.clone()));
        let client = client.clone();
        spawn_local(async move {
            match select_trading_adapter_task(client, adapter_id, context).await {
                Ok(response) => {
                    if let Some(shared) = shared_status {
                        shared.accept_receipt(response.clone());
                    }
                    if state.is_disposed() {
                        return;
                    }
                    bump_refresh(refresh_nonce);
                    adapters.update(|state| match state {
                        crate::state::load_state::LoadState::Ready(value)
                        | crate::state::load_state::LoadState::Stale { value, .. } => {
                            value.current.clone_from(&response.adapter);
                            value.current_environment = response.environment;
                        }
                        _ => {}
                    });
                    let mode =
                        crate::panels::shared::execution_environment_label(response.environment);
                    state.set(
                        ActionState::succeeded(format!("执行环境已切换为{mode}")).with_evidence(
                            trading_status_evidence(&response, &idempotency_key, pending_evidence),
                        ),
                    );
                }
                Err(error) => {
                    if state.is_disposed() {
                        return;
                    }
                    bump_refresh(refresh_nonce);
                    state.set(
                        ActionState::failed("执行环境切换失败", error.problem)
                            .with_evidence(pending_evidence),
                    );
                }
            }
        });
    });
    TradingAdapterSelectAction { state, submit }
}

pub(in crate::panels::modules::settings) fn use_venue_credential_save_action(
    credentials_refresh_nonce: RwSignal<u64>,
    runtime_health_refresh_nonce: RwSignal<u64>,
    account_state_refresh_nonce: RwSignal<u64>,
    action_runs: SettingsResource<Vec<ActionRun>>,
) -> VenueCredentialSaveAction {
    let client = use_global().client;
    let state = RwSignal::new(ActionState::Idle);
    let saved_revision = RwSignal::new(0_u64);
    use_action_run_recovery(
        state,
        action_runs,
        vec![ActionRunKind::VenueCredentialsUpdate],
    );
    let replay = RwSignal::new(None::<CredentialSaveReplay>);
    let submit = Callback::new(move |request: VenueCredentialSave| {
        if state.get_untracked().is_pending() {
            return;
        }
        let fingerprint = credential_save_fingerprint(&request.venue, &request.fields);
        let slot = CredentialSaveReplay {
            key: credential_save_replay_key(replay.get_untracked(), &fingerprint),
            fingerprint,
        };
        replay.set(Some(slot.clone()));
        let context = MutationRequestContext::with_idempotency_key(slot.key.clone());
        let pending_evidence = context.evidence();
        state.set(
            ActionState::pending("正在校验凭证证据并保存字段")
                .with_evidence(pending_evidence.clone()),
        );
        let client = client.clone();
        spawn_local(async move {
            let result =
                save_venue_credentials_task(client, request.venue, request.fields, context).await;
            if state.is_disposed() {
                return;
            }
            match result {
                Ok(response) => {
                    saved_revision.update(|revision| *revision = revision.wrapping_add(1));
                    replay.set(None);
                    bump_refresh(credentials_refresh_nonce);
                    bump_refresh(runtime_health_refresh_nonce);
                    bump_refresh(account_state_refresh_nonce);
                    state.set(
                        ActionState::succeeded(credential_success_message(&response))
                            .with_evidence(credential_response_evidence(
                                &response,
                                &slot.key,
                                pending_evidence,
                            )),
                    );
                }
                Err(error) => {
                    if should_reuse_credential_replay_key(&error) {
                        replay.set(Some(slot));
                    } else {
                        replay.set(None);
                    }
                    bump_refresh(credentials_refresh_nonce);
                    state.set(
                        ActionState::failed("保存失败", error.problem)
                            .with_evidence(pending_evidence),
                    );
                }
            }
        });
    });
    VenueCredentialSaveAction {
        state,
        saved_revision,
        submit,
    }
}

pub(in crate::panels::modules::settings) fn use_risk_config_save_action(
    refresh_nonce: RwSignal<u64>,
    action_runs: SettingsResource<Vec<ActionRun>>,
) -> RiskConfigSaveAction {
    let client = use_global().client;
    let state = RwSignal::new(ActionState::Idle);
    let receipt = RwSignal::new(None);
    let shared_status = use_context::<TradingStatusState>();
    use_action_run_recovery(
        state,
        action_runs,
        vec![ActionRunKind::TradingRiskConfigUpdate],
    );
    let replay = RwSignal::new(None::<RiskConfigReplay>);
    let submit = Callback::new(move |patch: RiskConfigPatch| {
        if state.get_untracked().is_pending() {
            return;
        }
        let slot = risk_config_replay_slot(replay.get_untracked(), &patch);
        replay.set(Some(slot.clone()));
        let context = MutationRequestContext::with_idempotency_key(slot.key.clone());
        let pending_evidence = context.evidence();
        state.set(ActionState::pending("正在保存风控参数").with_evidence(pending_evidence.clone()));
        let client = client.clone();
        spawn_local(async move {
            match update_risk_config_task(client, patch, context).await {
                Ok(response) => {
                    if let Some(shared) = shared_status {
                        shared.accept_receipt(response.clone());
                    }
                    if state.is_disposed() {
                        return;
                    }
                    receipt.set(Some(response.clone()));
                    replay.set(None);
                    bump_refresh(refresh_nonce);
                    state.set(
                        ActionState::succeeded(super::format::risk_config_success_message(
                            &response, &slot.key,
                        ))
                        .with_evidence(trading_status_evidence(
                            &response,
                            &slot.key,
                            pending_evidence,
                        )),
                    );
                }
                Err(error) => {
                    if state.is_disposed() {
                        return;
                    }
                    if should_reuse_credential_replay_key(&error) {
                        replay.set(Some(slot));
                    } else {
                        replay.set(None);
                    }
                    bump_refresh(refresh_nonce);
                    state.set(
                        ActionState::failed("保存失败", error.problem)
                            .with_evidence(pending_evidence),
                    );
                }
            }
        });
    });
    RiskConfigSaveAction {
        state,
        receipt,
        submit,
    }
}

pub(in crate::panels::modules::settings) fn use_kill_switch_action(
    refresh_nonce: RwSignal<u64>,
    action_runs: SettingsResource<Vec<ActionRun>>,
) -> KillSwitchAction {
    let client = use_global().client;
    let state = RwSignal::new(ActionState::Idle);
    let shared_status = use_context::<TradingStatusState>();
    use_action_run_recovery(state, action_runs, vec![ActionRunKind::TradingKillSwitch]);
    let replay = RwSignal::new(None::<kill_switch_idempotency::KillSwitchReplaySlot>);
    let submit = Callback::new(move |request: KillSwitchRequest| {
        if state.get_untracked().is_pending() {
            return;
        }
        let slot = kill_switch_idempotency::replay_slot(replay.get_untracked(), &request);
        replay.set(Some(slot.clone()));
        let context = MutationRequestContext::with_idempotency_key(slot.key.clone());
        let pending_evidence = context.evidence();
        state.set(
            ActionState::pending("正在更新 Kill Switch").with_evidence(pending_evidence.clone()),
        );
        let client = client.clone();
        spawn_local(async move {
            match set_kill_switch_task(client, request, context).await {
                Ok(response) => {
                    if let Some(shared) = shared_status {
                        shared.accept_receipt(response.status.clone());
                    }
                    if state.is_disposed() {
                        return;
                    }
                    replay.set(None);
                    bump_refresh(refresh_nonce);
                    state.set(
                        ActionState::succeeded(kill_switch_success_message(&response))
                            .with_evidence(kill_switch_evidence(
                                &response,
                                &slot.key,
                                pending_evidence,
                            )),
                    );
                }
                Err(error) => {
                    if state.is_disposed() {
                        return;
                    }
                    if kill_switch_idempotency::should_reuse_replay_key(&error) {
                        replay.set(Some(slot));
                    } else {
                        replay.set(None);
                    }
                    bump_refresh(refresh_nonce);
                    state.set(
                        ActionState::failed("更新失败", error.problem)
                            .with_evidence(pending_evidence),
                    );
                }
            }
        });
    });
    KillSwitchAction { state, submit }
}
