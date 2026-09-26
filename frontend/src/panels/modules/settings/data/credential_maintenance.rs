use crate::api::rest::{ApiClient, ApiError, MutationRequestContext};
use crate::state::action_state::ActionState;
use futures::future::{select, Either};
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{ActionEvidence, ActionRunKind, VenueCredentialMaintenanceResponse};

use super::format::credential_maintenance_success_message;
use super::resources::bump_refresh;

pub(in crate::panels::modules::settings) enum VenueCredentialMaintenance {
    Clear { venue: String, fields: Vec<String> },
    Migrate { venue: String },
}

impl VenueCredentialMaintenance {
    fn pending_message(&self) -> &'static str {
        match self {
            Self::Clear { .. } => "正在清空凭证字段并使运行状态数据依据失效",
            Self::Migrate { .. } => "正在迁移凭证到当前 secret backend",
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::panels::modules::settings) struct VenueCredentialMaintenanceAction {
    pub state: RwSignal<ActionState>,
    pub submit: Callback<VenueCredentialMaintenance>,
    pub journal: super::SettingsJournal,
}

pub(in crate::panels::modules::settings) fn use_venue_credential_maintenance_action(
    credentials_refresh_nonce: RwSignal<u64>,
    runtime_health_refresh_nonce: RwSignal<u64>,
    account_state_refresh_nonce: RwSignal<u64>,
    journal: super::SettingsJournal,
) -> VenueCredentialMaintenanceAction {
    let state = RwSignal::new(ActionState::Idle);
    Effect::new(move |_| {
        journal.connection.track();
        state.set(journal.restored_state(&[
            ActionRunKind::VenueCredentialsClear,
            ActionRunKind::VenueCredentialsMigrate,
        ]));
    });
    let submit = Callback::new(move |request: VenueCredentialMaintenance| {
        let (kind, target) = match &request {
            VenueCredentialMaintenance::Clear { venue, .. } => {
                (ActionRunKind::VenueCredentialsClear, venue.clone())
            }
            VenueCredentialMaintenance::Migrate { venue } => {
                (ActionRunKind::VenueCredentialsMigrate, venue.clone())
            }
        };
        let Some(attempt) = journal.begin(kind, target) else {
            return;
        };
        let epoch = journal.epoch.get_untracked();
        let context = attempt.context.clone();
        let pending_evidence = context.evidence();
        let key = context.idempotency_key().unwrap_or_default().to_owned();
        state.set(
            ActionState::pending(request.pending_message()).with_evidence(pending_evidence.clone()),
        );
        let client = journal.client();
        spawn_local(async move {
            let result = credential_maintenance_task(client, request, context)
                .await
                .and_then(|response| {
                    super::validate_setting_response(&attempt, &response)?;
                    Ok(response)
                });
            if !journal.current(epoch) {
                return;
            }
            match result {
                Ok(response) => {
                    let evidence = maintenance_response_evidence(&response, &key, pending_evidence);
                    if journal.resolve(&attempt) {
                        bump_refresh(credentials_refresh_nonce);
                        bump_refresh(runtime_health_refresh_nonce);
                        bump_refresh(account_state_refresh_nonce);
                        state.set(
                            ActionState::succeeded(credential_maintenance_success_message(
                                &response, &key,
                            ))
                            .with_evidence(evidence),
                        );
                    } else {
                        state.set(
                            ActionState::accepted("处理结果已返回，恢复记录待清理")
                                .with_evidence(evidence),
                        );
                    }
                    journal.busy.set(false);
                }
                Err(error) => {
                    journal.failed(&attempt, &error);
                    bump_refresh(credentials_refresh_nonce);
                    state.set(
                        ActionState::failed("凭证维护结果待确认", error.problem)
                            .with_evidence(pending_evidence),
                    );
                }
            }
        });
    });
    VenueCredentialMaintenanceAction {
        state,
        submit,
        journal,
    }
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
