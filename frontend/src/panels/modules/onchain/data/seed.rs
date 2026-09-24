use crate::api::rest::ApiClient;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{
    OnchainExecutionRunStatus, OnchainExecutionSubmitResponse,
    OnchainTokenApprovalRunStatus, OnchainTokenApprovalSubmitResponse, WebhookRuntimeStatus,
};
use super::snapshot_state::SnapshotState;

pub(super) fn start_seed_reads(
    client: &ApiClient,
    snapshots: SnapshotState,
    webhook_status: RwSignal<LoadState<WebhookRuntimeStatus>>,
    execution_submit: RwSignal<Option<Result<OnchainExecutionSubmitResponse, String>>>,
    approval_submit: RwSignal<Option<Result<OnchainTokenApprovalSubmitResponse, String>>>,
    approval_history: RwSignal<
        Option<Result<shared_types::OnchainTokenApprovalRunsResponse, String>>,
    >,
    recovery_problem: RwSignal<Option<String>>,
) {
    let seed_client = client.clone();
    Effect::new(move |_| {
        let Some(stamp) = snapshots.read_stamp() else { return; };
        let client = seed_client.clone();
        spawn_local(async move {
            let result = client
                .onchain_comparison()
                .await
                .map_err(|error| error.problem);
            snapshots.apply_read(stamp, result);
        });
    });
    let webhook_client = client.clone();
    Effect::new(move |_| {
        let client = webhook_client.clone();
        spawn_local(async move {
            let result = client.webhook_status().await.map_err(|error| error.problem);
            match result {
                Ok(status) => apply_webhook_status(webhook_status, status),
                Err(problem) => { let _ = webhook_status.try_update(|current| {
                    if current.value().is_none() { current.apply_result(Err(problem)); }
                }); }
            }
        });
    });
    let runs_client = client.clone();
    Effect::new(move |_| {
        let client = runs_client.clone();
        spawn_local(async move {
            let Ok(snapshot) = client.onchain_execution_runs(20).await else {
                return;
            };
            if recovery_problem.try_get_untracked().is_none() { return; }
            recovery_problem.set(snapshot.recovery_problem);
            if execution_submit.get_untracked().is_none() {
                execution_submit.set(
                    snapshot
                        .rows
                        .into_iter()
                        .find(|run| restoreable_execution_status(run.status))
                        .map(Ok),
                );
            }
        });
    });
    let approval_runs_client = client.clone();
    Effect::new(move |_| {
        let client = approval_runs_client.clone();
        spawn_local(async move {
            let result = client
                .onchain_token_approval_runs(20)
                .await
                .map_err(|e| e.to_string());
            if approval_history.try_get_untracked().is_none() { return; }
            approval_history.set(Some(result.clone()));
            let Ok(snapshot) = result else {
                return;
            };
            if approval_submit.get_untracked().is_none() {
                approval_submit.set(
                    snapshot
                        .rows
                        .iter()
                        .find(|run| restoreable_approval_status(run.status))
                        .or_else(|| snapshot.rows.first())
                        .cloned()
                        .map(Ok),
                );
            }
        });
    });
}

fn restoreable_execution_status(status: OnchainExecutionRunStatus) -> bool {
    matches!(
        status,
        OnchainExecutionRunStatus::Executing
            | OnchainExecutionRunStatus::AwaitingChainFinality
            | OnchainExecutionRunStatus::FinalityUnresolved
            | OnchainExecutionRunStatus::Exposed
    )
}

fn restoreable_approval_status(status: OnchainTokenApprovalRunStatus) -> bool {
    matches!(
        status,
        OnchainTokenApprovalRunStatus::AwaitingFinality
            | OnchainTokenApprovalRunStatus::FinalityUnresolved
    )
}

pub(super) fn apply_webhook_status(
    state: RwSignal<LoadState<WebhookRuntimeStatus>>,
    next: WebhookRuntimeStatus,
) {
    let _ = state.try_update(|current| {
        let should_replace = current
            .value()
            .is_none_or(|existing| next.updated_at_ms >= existing.updated_at_ms);
        if should_replace {
            *current = LoadState::Ready(next);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_older_rest_snapshot_cannot_overwrite_newer_webhook_ws_state() {
        Owner::new().with(|| {
            let state = RwSignal::new(LoadState::Ready(WebhookRuntimeStatus {
                updated_at_ms: 200,
                ..WebhookRuntimeStatus::default()
            }));

            apply_webhook_status(
                state,
                WebhookRuntimeStatus {
                    updated_at_ms: 100,
                    ..WebhookRuntimeStatus::default()
                },
            );
            assert_eq!(state.get_untracked().value().unwrap().updated_at_ms, 200);

            apply_webhook_status(
                state,
                WebhookRuntimeStatus {
                    updated_at_ms: 300,
                    ..WebhookRuntimeStatus::default()
                },
            );
            assert_eq!(state.get_untracked().value().unwrap().updated_at_ms, 300);
        });
    }

    #[test]
    fn seed_only_restores_runs_that_still_need_attention() {
        assert!(restoreable_execution_status(
            OnchainExecutionRunStatus::Executing
        ));
        assert!(restoreable_execution_status(
            OnchainExecutionRunStatus::AwaitingChainFinality
        ));
        assert!(restoreable_execution_status(
            OnchainExecutionRunStatus::Exposed
        ));
        assert!(!restoreable_execution_status(
            OnchainExecutionRunStatus::Completed
        ));
        assert!(!restoreable_execution_status(
            OnchainExecutionRunStatus::Compensated
        ));

        assert!(restoreable_approval_status(
            OnchainTokenApprovalRunStatus::AwaitingFinality
        ));
        assert!(!restoreable_approval_status(
            OnchainTokenApprovalRunStatus::Completed
        ));
    }
}
