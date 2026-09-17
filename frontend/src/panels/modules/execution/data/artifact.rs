use crate::state::{context::use_global, load_state::LoadState};
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{
    DeterministicExecutionArtifact, ExecutionArtifactBuildRequest,
    ExecutionArtifactValidationRequest, ExecutionArtifactValidationResponse,
};

use super::preview::ExecutionPreview;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::execution) struct ExecutionArtifactRuntime {
    pub state: RwSignal<LoadState<Option<DeterministicExecutionArtifact>>>,
    pub validation: RwSignal<LoadState<Option<ExecutionArtifactValidationResponse>>>,
    pub validate: Callback<()>,
}

#[derive(Clone, PartialEq, Eq)]
struct ArtifactKey {
    idempotency_key: String,
    ticket_id: String,
    opportunity_snapshot_id: String,
}

impl ArtifactKey {
    fn from_preview(preview: &ExecutionPreview) -> Option<Self> {
        if !preview.can_submit() || preview.opportunity_snapshot_id.trim().is_empty() {
            return None;
        }
        Some(Self {
            idempotency_key: preview.idempotency_key.clone()?,
            ticket_id: preview.ticket_id.clone()?,
            opportunity_snapshot_id: preview.opportunity_snapshot_id.clone(),
        })
    }

    fn build_request(&self) -> ExecutionArtifactBuildRequest {
        ExecutionArtifactBuildRequest {
            idempotency_key: self.idempotency_key.clone(),
            ticket_id: self.ticket_id.clone(),
            opportunity_snapshot_id: self.opportunity_snapshot_id.clone(),
        }
    }
}

pub(in crate::panels::modules::execution) fn use_execution_artifact(
    preview: Memo<ExecutionPreview>,
) -> ExecutionArtifactRuntime {
    let client = use_global().client;
    let state = RwSignal::new(LoadState::Ready(None));
    let validation = RwSignal::new(LoadState::Ready(None));
    let current_key = RwSignal::new(None::<ArtifactKey>);
    let request_version = RwSignal::new(0_u64);
    let build_client = client.clone();
    Effect::new(move |_| {
        let key = ArtifactKey::from_preview(&preview.get());
        if current_key.get_untracked() == key {
            return;
        }
        current_key.set(key.clone());
        validation.set(LoadState::Ready(None));
        request_version.update(|value| *value = value.wrapping_add(1));
        let version = request_version.get_untracked();
        let Some(key) = key else {
            state.set(LoadState::Ready(None));
            return;
        };
        state.set(LoadState::Loading);
        let client = build_client.clone();
        spawn_local(async move {
            let result = client
                .build_execution_artifact(&key.build_request())
                .await
                .map(Some)
                .map_err(|error| error.problem);
            if request_version.get_untracked() == version {
                state.update(|current| current.apply_result(result));
            }
        });
    });
    let validate = Callback::new(move |()| {
        let Some(artifact) = state
            .get_untracked()
            .value()
            .and_then(Option::as_ref)
            .cloned()
        else {
            validation.set(LoadState::Ready(None));
            return;
        };
        let client = client.clone();
        validation.set(LoadState::Loading);
        spawn_local(async move {
            let request = ExecutionArtifactValidationRequest {
                idempotency_key: artifact.idempotency_key,
                ticket_id: artifact.ticket_id,
                opportunity_snapshot_id: artifact.opportunity_snapshot_id,
                checksum: artifact.checksum,
            };
            let result = client
                .validate_execution_artifact(&request)
                .await
                .map(Some)
                .map_err(|error| error.problem);
            validation.update(|current| current.apply_result(result));
        });
    });
    ExecutionArtifactRuntime {
        state,
        validation,
        validate,
    }
}

pub(in crate::panels::modules::execution) fn artifact_is_ready(
    state: &LoadState<Option<DeterministicExecutionArtifact>>,
) -> bool {
    state
        .value()
        .and_then(Option::as_ref)
        .is_some_and(|artifact| artifact.status.is_ready() && artifact.blockers.is_empty())
}

pub(in crate::panels::modules::execution) fn artifact_validation_is_ready(
    state: &LoadState<Option<ExecutionArtifactValidationResponse>>,
) -> bool {
    matches!(
        state,
        LoadState::Ready(Some(result)) if result.valid && result.status.is_ready()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::ExecutionArtifactStatus;

    #[test]
    fn submit_readiness_requires_a_valid_ready_server_response() {
        let response = |valid, status| {
            LoadState::Ready(Some(ExecutionArtifactValidationResponse {
                valid,
                status,
                checked_at_ms: 1,
                expires_at_ms: Some(2),
                blockers: Vec::new(),
                artifact: None,
            }))
        };

        assert!(artifact_validation_is_ready(&response(
            true,
            ExecutionArtifactStatus::Ready
        )));
        assert!(!artifact_validation_is_ready(&response(
            false,
            ExecutionArtifactStatus::Ready
        )));
        assert!(!artifact_validation_is_ready(&response(
            false,
            ExecutionArtifactStatus::Tampered
        )));
        assert!(!artifact_validation_is_ready(&LoadState::Stale {
            value: response(true, ExecutionArtifactStatus::Ready)
                .value()
                .cloned()
                .unwrap_or(None),
            problem: shared_types::ApiProblem::new("VALIDATION_STALE", "latest check failed"),
        }));
    }
}
