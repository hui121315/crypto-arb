mod artifact;
mod monitor;
mod transfer;

use super::{emit_value, Cursor};
use crate::services::instrument_registry::CandidateTransferStatus;
use crate::state::AppState;
use artifact::{
    deterministic_artifact_ready_at, opportunity_artifact_payload, transfer_is_relevant,
};
use monitor::emit_transfer_monitors;
use shared_types::{DeterministicExecutionArtifact, WebhookEventKind};
use transfer::status_allows_deterministic_delivery;

pub(super) async fn emit_opportunity(state: &AppState, cursor: &mut Cursor) {
    let status = state.automation().snapshot();
    let now_ms = common::time::now_ms();
    let ready_artifact = status
        .recent_decisions
        .iter()
        .find_map(|decision| decision.execution_artifact.as_ref())
        .filter(|artifact| deterministic_artifact_ready_at(artifact, now_ms));
    let current_transfer =
        ready_artifact.and_then(|artifact| current_transfer_status(state, artifact, now_ms));
    let artifact = ready_artifact.filter(|artifact| {
        !transfer_is_relevant(artifact.strategy)
            || current_transfer
                .as_ref()
                .is_some_and(status_allows_deterministic_delivery)
    });
    if let Some(artifact) = artifact {
        if cursor.opportunity_artifact_id.as_deref() != Some(artifact.artifact_id.as_str()) {
            let payload = opportunity_artifact_payload(artifact, current_transfer.as_ref(), now_ms);
            if emit_value(
                state,
                WebhookEventKind::Opportunity,
                format!("opportunity-{}", artifact.artifact_id),
                &payload,
            )
            .await
            {
                cursor.opportunity_artifact_id = Some(artifact.artifact_id.clone());
                return;
            }
        }
    }
    emit_transfer_monitors(
        state,
        cursor,
        artifact.map(|artifact| artifact.opportunity_id.as_str()),
    )
    .await;
}

fn current_transfer_status(
    state: &AppState,
    artifact: &DeterministicExecutionArtifact,
    now_ms: i64,
) -> Option<CandidateTransferStatus> {
    if !transfer_is_relevant(artifact.strategy) {
        return None;
    }
    let row = state
        .opportunity_index()
        .current(&artifact.opportunity_id)?;
    if row.strategy_kind != artifact.strategy
        || !row.symbol.eq_ignore_ascii_case(&artifact.symbol)
        || shared_types::market_monitor_net_bps_at(&row, now_ms).is_none()
    {
        return None;
    }
    Some(
        state
            .instrument_registry()
            .candidate_transfer_status(&row, now_ms),
    )
}

pub(super) fn current_monitor_event_keys(
    state: &AppState,
    excluded_opportunity_id: Option<&str>,
) -> std::collections::HashMap<String, i64> {
    monitor::current_monitor_event_keys(state, excluded_opportunity_id)
}
