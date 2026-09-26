use crate::state::load_state::LoadState;
use super::connection::ExecutionConnection;
use gloo_timers::callback::Interval;
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
    pub rebuild: Callback<()>,
    pub preview: Memo<ExecutionPreview>,
    pub clock: RwSignal<i64>,
    pub ready: Memo<bool>,
    pub validated: Memo<bool>,
    pub expired: Memo<bool>,
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

    fn matches(&self, artifact: &DeterministicExecutionArtifact) -> bool {
        self.idempotency_key == artifact.idempotency_key
            && self.ticket_id == artifact.ticket_id
            && self.opportunity_snapshot_id == artifact.opportunity_snapshot_id
    }
}

pub(in crate::panels::modules::execution) fn use_execution_artifact(
    preview: Memo<ExecutionPreview>,
) -> ExecutionArtifactRuntime {
    let connection = expect_context::<ExecutionConnection>();
    let client = connection.client();
    let state = RwSignal::new(LoadState::Ready(None));
    let validation = RwSignal::new(LoadState::Ready(None));
    let current_key = RwSignal::new(None::<(ArtifactKey, u64)>);
    let rebuild_nonce = RwSignal::new(0_u64);
    let request_version = RwSignal::new(0_u64);
    let clock = RwSignal::new(preview.get_untracked().current_time_ms());
    Effect::new(move |_| clock.set(preview.get().current_time_ms()));
    let interval = StoredValue::new_local(Some(Interval::new(1_000, move || {
        clock.set(preview.get_untracked().current_time_ms());
    })));
    on_cleanup(move || {
        interval.update_value(|slot| {
            slot.take();
        })
    });
    let ready = Memo::new(move |_| artifact_is_ready(&state.get(), &preview.get(), clock.get()));
    let expired = Memo::new(move |_| {
        let now = clock.get();
        preview.get().expires_at_ms.is_some_and(|until| now >= until)
            || state.get().value().and_then(Option::as_ref)
                .and_then(artifact_valid_until).is_some_and(|until| now >= until)
    });
    let validated = Memo::new(move |_| {
        artifact_validation_is_ready(&validation.get(), &state.get(), &preview.get(), clock.get())
    });
    let build_client = client.clone();
    Effect::new(move |_| {
        if !connection.available() { return; }
        let key = ArtifactKey::from_preview(&preview.get());
        let request_key = key.clone().map(|key| (key, rebuild_nonce.get()));
        if current_key.get_untracked() == request_key {
            return;
        }
        current_key.set(request_key);
        validation.set(LoadState::Ready(None));
        request_version.update(|value| *value = value.wrapping_add(1));
        let version = request_version.get_untracked();
        let Some(key) = key else {
            state.set(LoadState::Ready(None));
            return;
        };
        state.set(LoadState::Loading);
        let client = build_client.clone();
        let opportunity_id = preview.get_untracked().opportunity_id;
        spawn_local(async move {
            let result = super::await_execution_check(
                "生成提交前检查信息", client.build_execution_artifact(&key.build_request()),
            )
                .await
                .and_then(|artifact| {
                    if key.matches(&artifact) && artifact.opportunity_id == opportunity_id {
                        Ok(Some(artifact))
                    } else {
                        Err(binding_problem())
                    }
                });
            if connection.current() && request_version.try_get_untracked() == Some(version)
                && ArtifactKey::from_preview(&preview.get_untracked()).as_ref() == Some(&key)
            {
                state.update(|current| current.apply_result(result));
            }
        });
    });
    let validate = Callback::new(move |()| {
        if !connection.current() { return; }
        let current_preview = preview.get_untracked();
        let now_ms = current_preview.current_time_ms();
        clock.set(now_ms);
        if matches!(validation.get_untracked(), LoadState::Loading)
            || !artifact_is_ready(&state.get_untracked(), &current_preview, now_ms)
        {
            return;
        }
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
        let version = request_version.get_untracked();
        let key = ArtifactKey::from_preview(&preview.get_untracked());
        validation.set(LoadState::Loading);
        spawn_local(async move {
            let request = ExecutionArtifactValidationRequest {
                idempotency_key: artifact.idempotency_key,
                ticket_id: artifact.ticket_id,
                opportunity_snapshot_id: artifact.opportunity_snapshot_id,
                checksum: artifact.checksum,
            };
            let result = super::await_execution_check(
                "核对提交条件", client.validate_execution_artifact(&request),
            )
                .await
                .map(Some);
            if connection.current() && request_version.try_get_untracked() == Some(version)
                && ArtifactKey::from_preview(&preview.get_untracked()) == key
            {
                validation.update(|current| current.apply_result(result));
            }
        });
    });
    let rebuild = Callback::new(move |()| {
        if connection.current()
            && state.get_untracked().problem().is_some()
            && preview.get_untracked().can_submit()
        {
            rebuild_nonce.update(|value| *value = value.wrapping_add(1));
        }
    });
    ExecutionArtifactRuntime {
        state,
        validation,
        validate,
        rebuild,
        preview,
        clock,
        ready,
        validated,
        expired,
    }
}

pub(in crate::panels::modules::execution) fn artifact_is_ready(
    state: &LoadState<Option<DeterministicExecutionArtifact>>,
    preview: &ExecutionPreview,
    now_ms: i64,
) -> bool {
    let LoadState::Ready(Some(artifact)) = state else {
        return false;
    };
    preview.can_submit_at(now_ms)
        && ArtifactKey::from_preview(preview).is_some_and(|key| key.matches(artifact))
        && artifact.opportunity_id == preview.opportunity_id
        && crate::panels::shared::execution_environment_label(artifact.environment)
            == preview.execution_mode_label
        && artifact.status.is_ready()
        && artifact.blockers.is_empty()
        && !artifact.checksum.trim().is_empty()
        && artifact_valid_until(artifact).is_some_and(|expires| now_ms < expires)
        && artifact.legs.iter().all(|leg| {
            leg.market_observed_at_ms
                .is_some_and(|observed| observed <= now_ms)
        })
}

pub(in crate::panels::modules::execution) fn artifact_valid_until(
    artifact: &DeterministicExecutionArtifact,
) -> Option<i64> {
    if artifact.legs.len() != 2 {
        return None;
    }
    artifact
        .legs
        .iter()
        .try_fold(artifact.expires_at_ms, |expires, leg| {
            let observed = leg.market_observed_at_ms.filter(|time| *time > 0)?;
            Some(
                expires.min(observed.saturating_add(shared_types::HEDGE_PREVIEW_MARKET_MAX_AGE_MS)),
            )
        })
}

pub(in crate::panels::modules::execution) fn artifact_validation_is_ready(
    state: &LoadState<Option<ExecutionArtifactValidationResponse>>,
    artifact_state: &LoadState<Option<DeterministicExecutionArtifact>>,
    preview: &ExecutionPreview,
    now_ms: i64,
) -> bool {
    if !artifact_is_ready(artifact_state, preview, now_ms) {
        return false;
    }
    let LoadState::Ready(Some(result)) = state else {
        return false;
    };
    let Some(artifact) = artifact_state.value().and_then(Option::as_ref) else {
        return false;
    };
    validation_matches_artifact(result, artifact, now_ms)
}

fn validation_matches_artifact(
    result: &ExecutionArtifactValidationResponse,
    artifact: &DeterministicExecutionArtifact,
    now_ms: i64,
) -> bool {
    result.valid
        && result.checked_at_ms > 0
        && result.checked_at_ms >= artifact.generated_at_ms
        && result.checked_at_ms <= now_ms
        && result.status.is_ready()
        && result.blockers.is_empty()
        && result.expires_at_ms.is_some_and(|expires| now_ms < expires)
        && result.artifact.as_ref().is_some_and(|verified| {
            verified.artifact_id == artifact.artifact_id
                && verified.checksum == artifact.checksum
                && verified.opportunity_id == artifact.opportunity_id
                && verified.ticket_id == artifact.ticket_id
                && verified.opportunity_snapshot_id == artifact.opportunity_snapshot_id
                && verified.idempotency_key == artifact.idempotency_key
                && verified.environment == artifact.environment
                && verified.status.is_ready()
                && verified.blockers.is_empty()
                && artifact_valid_until(verified).is_some_and(|expires| now_ms < expires)
        })
}

fn binding_problem() -> shared_types::ApiProblem {
    shared_types::ApiProblem::new(
        "EXECUTION_ARTIFACT_MISMATCH",
        "检查结果不属于当前交易计划，请重新检查",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::ExecutionArtifactStatus;

    fn artifact() -> DeterministicExecutionArtifact {
        serde_json::from_value(serde_json::json!({
            "schemaVersion": "crossline.execution-artifact.v1", "artifactId": "artifact-1",
            "opportunityId": "opportunity-1", "opportunitySnapshotId": "snapshot-1",
            "ticketId": "ticket-1", "idempotencyKey": "key-1", "environment": "paper",
            "symbol": "BTC", "generatedAtMs": 1, "expiresAtMs": 100,
            "status": "ready", "expectedGrossEdgeUsd": 2, "expectedTotalCostUsd": 1,
            "expectedNetEdgeUsd": 1, "capitalUsd": 10, "maxLossUsd": 1, "legs": [
                {"role":"long","venue":"paper-a","symbol":"BTC","side":"buy","targetNotionalUsd":10,"marketObservedAtMs":1},
                {"role":"short","venue":"paper-b","symbol":"BTC","side":"sell","targetNotionalUsd":10,"marketObservedAtMs":1}
            ],
            "evidence": [], "invalidationConditions": [], "checksum": "checksum-1",
            "validationCommand": "fixture-read-only"
        }))
        .unwrap()
    }

    fn validation(
        artifact: &DeterministicExecutionArtifact,
    ) -> ExecutionArtifactValidationResponse {
        ExecutionArtifactValidationResponse {
            valid: true,
            status: ExecutionArtifactStatus::Ready,
            checked_at_ms: 2,
            expires_at_ms: Some(100),
            blockers: Vec::new(),
            artifact: Some(artifact.clone()),
        }
    }

    #[test]
    fn freshness_expires_with_the_older_leg_not_only_ticket_ttl() {
        let mut artifact = artifact();
        artifact.expires_at_ms = 100_000;
        artifact.legs[0].market_observed_at_ms = Some(1_000);
        artifact.legs[1].market_observed_at_ms = Some(2_000);
        assert_eq!(artifact_valid_until(&artifact), Some(31_000));
        artifact.legs[0].market_observed_at_ms = None;
        assert_eq!(artifact_valid_until(&artifact), None);
    }

    #[test]
    fn validation_requires_bound_artifact_and_both_expiry_windows() {
        let artifact = artifact();
        let mut result = validation(&artifact);
        assert!(validation_matches_artifact(&result, &artifact, 99));
        assert!(!validation_matches_artifact(&result, &artifact, 100));
        result.expires_at_ms = None;
        assert!(!validation_matches_artifact(&result, &artifact, 3));
        result.expires_at_ms = Some(50);
        assert!(!validation_matches_artifact(&result, &artifact, 50));
        result.artifact = None;
        assert!(!validation_matches_artifact(&result, &artifact, 3));
    }

    #[test]
    fn validation_rejects_wrong_ticket_snapshot_checksum_or_environment() {
        let artifact = artifact();
        for field in [
            "ticketId",
            "opportunityId",
            "opportunitySnapshotId",
            "idempotencyKey",
            "checksum",
            "artifactId",
            "environment",
        ] {
            let mut value = serde_json::to_value(&artifact).unwrap();
            value[field] = serde_json::json!(if field == "environment" {
                "live"
            } else {
                "wrong"
            });
            let mut result = validation(&artifact);
            result.artifact = Some(serde_json::from_value(value).unwrap());
            assert!(
                !validation_matches_artifact(&result, &artifact, 3),
                "{field}"
            );
        }
    }

    #[test]
    fn a_valid_flag_never_overrides_blockers_or_nonready_status() {
        let artifact = artifact();
        let mut result = validation(&artifact);
        result.blockers.push("blocked".into());
        assert!(!validation_matches_artifact(&result, &artifact, 3));
        result.blockers.clear();
        result.status = ExecutionArtifactStatus::Tampered;
        assert!(!validation_matches_artifact(&result, &artifact, 3));
        result.status = ExecutionArtifactStatus::Ready;
        result
            .artifact
            .as_mut()
            .unwrap()
            .blockers
            .push("blocked".into());
        assert!(!validation_matches_artifact(&result, &artifact, 3));
    }
}
