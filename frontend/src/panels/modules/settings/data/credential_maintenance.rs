use crate::api::rest::{ApiClient, ApiError, MutationRequestContext};
use crate::state::action_state::ActionState;
use crate::state::context::use_global;
use futures::future::{select, Either};
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{ActionEvidence, ActionRun, ActionRunKind, VenueCredentialMaintenanceResponse};

use super::actions::should_reuse_credential_replay_key;
use super::format::credential_maintenance_success_message;
use super::resources::bump_refresh;
use super::resources::SettingsResource;
use super::use_action_run_recovery;

pub(in crate::panels::modules::settings) enum VenueCredentialMaintenance {
    Clear { venue: String, fields: Vec<String> },
    Migrate { venue: String },
}

impl VenueCredentialMaintenance {
    fn fingerprint(&self) -> String {
        match self {
            Self::Clear { venue, fields } => {
                let mut fields = fields.iter().map(|field| field.trim()).collect::<Vec<_>>();
                fields.sort_unstable();
                format!(
                    "clear:{}:{}",
                    venue.trim().to_ascii_lowercase(),
                    fields.join(",")
                )
            }
            Self::Migrate { venue } => format!("migrate:{}", venue.trim().to_ascii_lowercase()),
        }
    }

    fn pending_message(&self) -> &'static str {
        match self {
            Self::Clear { .. } => "正在清空凭证字段并使运行态证据失效",
            Self::Migrate { .. } => "正在迁移凭证到当前 secret backend",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::panels::modules::settings) struct CredentialMaintenanceReplay {
    pub(in crate::panels::modules::settings) fingerprint: String,
    pub(in crate::panels::modules::settings) key: String,
}

#[derive(Clone, Copy)]
pub(in crate::panels::modules::settings) struct VenueCredentialMaintenanceAction {
    pub state: RwSignal<ActionState>,
    pub submit: Callback<VenueCredentialMaintenance>,
}

pub(in crate::panels::modules::settings) fn use_venue_credential_maintenance_action(
    credentials_refresh_nonce: RwSignal<u64>,
    runtime_health_refresh_nonce: RwSignal<u64>,
    account_state_refresh_nonce: RwSignal<u64>,
    action_runs: SettingsResource<Vec<ActionRun>>,
) -> VenueCredentialMaintenanceAction {
    let client = use_global().client;
    let state = RwSignal::new(ActionState::Idle);
    use_action_run_recovery(
        state,
        action_runs,
        vec![
            ActionRunKind::VenueCredentialsClear,
            ActionRunKind::VenueCredentialsMigrate,
        ],
    );
    let replay = RwSignal::new(None::<CredentialMaintenanceReplay>);
    let submit = Callback::new(move |request: VenueCredentialMaintenance| {
        if state.get_untracked().is_pending() {
            return;
        }
        let fingerprint = request.fingerprint();
        let slot = CredentialMaintenanceReplay {
            key: credential_maintenance_replay_key(replay.get_untracked(), &fingerprint),
            fingerprint,
        };
        replay.set(Some(slot.clone()));
        let context = MutationRequestContext::with_idempotency_key(slot.key.clone());
        let pending_evidence = context.evidence();
        state.set(
            ActionState::pending(request.pending_message()).with_evidence(pending_evidence.clone()),
        );
        let client = client.clone();
        spawn_local(async move {
            match credential_maintenance_task(client, request, context).await {
                Ok(response) => {
                    replay.set(None);
                    bump_refresh(credentials_refresh_nonce);
                    bump_refresh(runtime_health_refresh_nonce);
                    bump_refresh(account_state_refresh_nonce);
                    state.set(
                        ActionState::succeeded(credential_maintenance_success_message(
                            &response, &slot.key,
                        ))
                        .with_evidence(maintenance_response_evidence(
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
                        ActionState::failed("凭证维护失败", error.problem)
                            .with_evidence(pending_evidence),
                    );
                }
            }
        });
    });
    VenueCredentialMaintenanceAction { state, submit }
}

async fn credential_maintenance_task(
    client: ApiClient,
    request: VenueCredentialMaintenance,
    context: MutationRequestContext,
) -> Result<VenueCredentialMaintenanceResponse, ApiError> {
    match request {
        VenueCredentialMaintenance::Clear { venue, fields } => {
            await_credential_maintenance(
                client.clear_venue_credentials_with_context(&venue, &fields, &context),
            )
            .await
        }
        VenueCredentialMaintenance::Migrate { venue } => {
            await_credential_maintenance(
                client.migrate_venue_credentials_with_context(&venue, &context),
            )
            .await
        }
    }
}

async fn await_credential_maintenance(
    request: impl std::future::Future<Output = Result<VenueCredentialMaintenanceResponse, ApiError>>,
) -> Result<VenueCredentialMaintenanceResponse, ApiError> {
    let timeout = TimeoutFuture::new(20_000);
    futures::pin_mut!(request, timeout);
    match select(request, timeout).await {
        Either::Left((result, _)) => result,
        Either::Right((_, _)) => Err(ApiError::client(
            "TIMEOUT",
            "凭证维护超时：请检查 API Base 或后端服务",
        )),
    }
}

pub(in crate::panels::modules::settings) fn credential_maintenance_replay_key(
    existing: Option<CredentialMaintenanceReplay>,
    fingerprint: &str,
) -> String {
    existing
        .filter(|slot| slot.fingerprint == fingerprint)
        .map_or_else(next_credential_maintenance_key, |slot| slot.key)
}

fn next_credential_maintenance_key() -> String {
    format!(
        "settings-credential-maintenance-{}",
        credential_maintenance_entropy()
    )
}

fn maintenance_response_evidence(
    response: &VenueCredentialMaintenanceResponse,
    idempotency_key: &str,
    pending: ActionEvidence,
) -> ActionEvidence {
    pending
        .with_request_id(response.request_id.clone())
        .with_action_run_id(response.action_run_id.clone())
        .with_idempotency_key(Some(idempotency_key.to_owned()))
}

#[cfg(target_arch = "wasm32")]
fn credential_maintenance_entropy() -> String {
    let now = js_sys::Date::now().to_bits();
    let random = js_sys::Math::random().to_bits();
    format!("{now:016x}{random:016x}")
}

#[cfg(not(target_arch = "wasm32"))]
fn credential_maintenance_entropy() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(1);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{sequence:016x}")
}
