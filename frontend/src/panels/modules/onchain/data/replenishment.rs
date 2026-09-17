use crate::api::rest::ApiClient;
use crate::state::polling::use_conditional_polling_result;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{
    OnchainReplenishmentAuthorizeRequest, OnchainReplenishmentBuildRequest,
    OnchainReplenishmentPlanResponse, OnchainReplenishmentRun, OnchainReplenishmentRunStatus,
    OnchainReplenishmentSubmitRequest,
};
use std::time::Duration;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::onchain) struct OnchainReplenishmentData {
    pub runs: RwSignal<Vec<OnchainReplenishmentRun>>,
    pub plan: RwSignal<Option<Result<OnchainReplenishmentPlanResponse, String>>>,
    pub building: RwSignal<bool>,
    pub build: Callback<OnchainReplenishmentBuildRequest>,
    pub confirmation: RwSignal<String>,
    pub authorizing: RwSignal<bool>,
    pub authorize: Callback<String>,
    pub run: RwSignal<Option<Result<OnchainReplenishmentRun, String>>>,
    pub submitting: RwSignal<bool>,
    pub submit: Callback<String>,
    pub recovery_problem: RwSignal<Option<String>>,
    pub rechecking: RwSignal<bool>,
    pub recheck: Callback<shared_types::OnchainReplenishmentRecheckRequest>,
}

#[derive(Clone, Copy)]
struct ReplenishmentSignals {
    runs: RwSignal<Vec<OnchainReplenishmentRun>>,
    plan: RwSignal<Option<Result<OnchainReplenishmentPlanResponse, String>>>,
    building: RwSignal<bool>,
    confirmation: RwSignal<String>,
    authorization_key: RwSignal<Option<String>>,
    authorizing: RwSignal<bool>,
    run: RwSignal<Option<Result<OnchainReplenishmentRun, String>>>,
    submitting: RwSignal<bool>,
    recovery_problem: RwSignal<Option<String>>,
    rechecking: RwSignal<bool>,
}

pub(super) fn use_replenishment(client: &ApiClient) -> OnchainReplenishmentData {
    let signals = ReplenishmentSignals {
        runs: RwSignal::new(Vec::new()),
        plan: RwSignal::new(None),
        building: RwSignal::new(false),
        confirmation: RwSignal::new(String::new()),
        authorization_key: RwSignal::new(None),
        authorizing: RwSignal::new(false),
        run: RwSignal::new(None),
        submitting: RwSignal::new(false),
        recovery_problem: RwSignal::new(None),
        rechecking: RwSignal::new(false),
    };
    restore_latest_run(client, signals);
    recover_active_run(client, signals);
    OnchainReplenishmentData {
        runs: signals.runs,
        plan: signals.plan,
        building: signals.building,
        build: build_callback(client.clone(), signals),
        confirmation: signals.confirmation,
        authorizing: signals.authorizing,
        authorize: authorize_callback(client.clone(), signals),
        run: signals.run,
        submitting: signals.submitting,
        submit: submit_callback(client.clone(), signals),
        recovery_problem: signals.recovery_problem,
        rechecking: signals.rechecking,
        recheck: recheck_callback(client.clone(), signals),
    }
}

fn build_callback(
    client: ApiClient,
    signals: ReplenishmentSignals,
) -> Callback<OnchainReplenishmentBuildRequest> {
    Callback::new(move |request| {
        if signals.building.get_untracked() {
            return;
        }
        let client = client.clone();
        signals.building.set(true);
        signals.plan.set(None);
        // A new preview must not hide or stop polling an existing funds transfer.
        signals.confirmation.set(String::new());
        signals.authorization_key.set(None);
        spawn_local(async move {
            let result = client
                .build_onchain_replenishment(&request)
                .await
                .map_err(|error| error.to_string());
            if let Ok(plan) = &result {
                signals.authorization_key.set(Some(format!(
                    "onchain-replenishment-{}-{}",
                    plan.plan_id,
                    crate::state::polling::now_ms()
                )));
            }
            signals.plan.set(Some(result));
            signals.building.set(false);
        });
    })
}

fn authorize_callback(client: ApiClient, signals: ReplenishmentSignals) -> Callback<String> {
    Callback::new(move |confirmation| {
        if signals.authorizing.get_untracked() || signals.submitting.get_untracked()
            || signals.rechecking.get_untracked() || signals.recovery_problem.get_untracked().is_some()
        {
            return;
        }
        let Some((plan_id, idempotency_key)) = authorization_scope(signals) else {
            signals.run.set(Some(Err(
                "补仓计划或幂等标识不存在，请重新生成计划".to_owned()
            )));
            return;
        };
        let client = client.clone();
        signals.authorizing.set(true);
        spawn_local(async move {
            let request = OnchainReplenishmentAuthorizeRequest {
                plan_id,
                idempotency_key,
                confirmation,
            };
            let result = client
                .authorize_onchain_replenishment(&request)
                .await
                .map_err(|error| error.to_string());
            signals.run.set(Some(result));
            signals.authorizing.set(false);
        });
    })
}

fn authorization_scope(signals: ReplenishmentSignals) -> Option<(String, String)> {
    let plan_id = signals
        .plan
        .get_untracked()?
        .ok()
        .map(|plan| plan.plan_id)?;
    let key = signals.authorization_key.get_untracked()?;
    Some((plan_id, key))
}

fn submit_callback(client: ApiClient, signals: ReplenishmentSignals) -> Callback<String> {
    Callback::new(move |run_id| {
        if signals.submitting.get_untracked() || signals.authorizing.get_untracked()
            || signals.rechecking.get_untracked() || signals.recovery_problem.get_untracked().is_some()
        {
            return;
        }
        let client = client.clone();
        signals.submitting.set(true);
        spawn_local(async move {
            let result = client
                .submit_onchain_replenishment(&OnchainReplenishmentSubmitRequest { run_id })
                .await
                .map_err(|error| error.to_string());
            signals.run.set(Some(result));
            signals.submitting.set(false);
        });
    })
}

fn recheck_callback(client: ApiClient, signals: ReplenishmentSignals) -> Callback<shared_types::OnchainReplenishmentRecheckRequest> {
    Callback::new(move |request| {
        if signals.rechecking.get_untracked() || signals.submitting.get_untracked()
            || signals.authorizing.get_untracked() || signals.recovery_problem.get_untracked().is_some() { return; }
        let current = signals.run.get_untracked().and_then(Result::ok);
        if current.as_ref().and_then(|r| r.recheck_request()).as_ref() != Some(&request) { return; }
        signals.rechecking.set(true);
        let client = client.clone();
        spawn_local(async move {
            match client.recheck_onchain_replenishment(&request).await {
                Ok(run) => accept_latest_run(signals, run),
                Err(error) => signals.recovery_problem.set(Some(format!("重新核验反馈未确认，保留原记录并刷新：{error}"))),
            }
            signals.rechecking.set(false);
        });
    })
}

fn accept_latest_run(signals: ReplenishmentSignals, latest: OnchainReplenishmentRun) {
    let accept = signals.run.with_untracked(|current| match current {
        Some(Ok(current)) if current.run_id == latest.run_id => latest.updated_at_ms >= current.updated_at_ms,
        _ => true,
    });
    if accept { signals.run.set(Some(Ok(latest))); }
}

fn restore_latest_run(client: &ApiClient, signals: ReplenishmentSignals) {
    let client = client.clone();
    Effect::new(move |_| {
        let client = client.clone();
        spawn_local(async move {
            let snapshot = match client.onchain_replenishment_runs(20).await {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    signals
                        .recovery_problem
                        .set(Some(format!("补仓运行记录读取失败：{error}")));
                    return;
                }
            };
            signals.recovery_problem.set(snapshot.recovery_problem);
            signals.runs.set(snapshot.rows.clone());
            if signals.run.get_untracked().is_none() {
                signals.run.set(snapshot.rows.into_iter().next().map(Ok));
            }
        });
    });
}

fn recover_active_run(client: &ApiClient, signals: ReplenishmentSignals) {
    let run = signals.run;
    let poll = use_conditional_polling_result(
        Duration::from_secs(2),
        move || {
            pending_run_id(run.try_get_untracked().flatten()).is_some()
                || signals
                    .recovery_problem
                    .try_get_untracked()
                    .flatten()
                    .is_some()
        },
        {
            let client = client.clone();
            move || {
                let client = client.clone();
                async move { client.onchain_replenishment_runs(20).await }
            }
        },
    );
    Effect::new(move |_| {
        let Some(result) = poll.get().and_then(|event| event.take().into_fetched()) else {
            return;
        };
        let snapshot = match result {
            Ok(snapshot) => snapshot,
            Err(error) => {
                signals
                    .recovery_problem
                    .set(Some(format!("补仓运行记录读取失败，保留上次结果：{error}")));
                return;
            }
        };
        signals.recovery_problem.set(snapshot.recovery_problem);
        signals.runs.set(snapshot.rows.clone());
        if run.get_untracked().is_none() {
            run.set(snapshot.rows.into_iter().next().map(Ok));
            return;
        }
        let Some(run_id) = run.get_untracked().and_then(Result::ok).map(|run| run.run_id) else {
            return;
        };
        if let Some(latest) = snapshot.rows.into_iter().find(|row| row.run_id == run_id) {
            accept_latest_run(signals, latest);
        }
    });
}

fn pending_run_id(result: Option<Result<OnchainReplenishmentRun, String>>) -> Option<String> {
    result.and_then(|result| match result {
        Ok(run) if status_needs_poll(run.status) => Some(run.run_id),
        Ok(_) | Err(_) => None,
    })
}

fn status_needs_poll(status: OnchainReplenishmentRunStatus) -> bool {
    matches!(
        status,
        OnchainReplenishmentRunStatus::AuthorizedAwaitingSubmit
            | OnchainReplenishmentRunStatus::ReadyForNextTransfer
            | OnchainReplenishmentRunStatus::Submitting
            | OnchainReplenishmentRunStatus::AwaitingSourceFinality
            | OnchainReplenishmentRunStatus::AwaitingDestinationCredit
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_non_terminal_replenishment_runs_keep_polling() {
        assert!(status_needs_poll(
            OnchainReplenishmentRunStatus::AuthorizedAwaitingSubmit
        ));
        assert!(status_needs_poll(
            OnchainReplenishmentRunStatus::AwaitingDestinationCredit
        ));
        assert!(status_needs_poll(
            OnchainReplenishmentRunStatus::ReadyForNextTransfer
        ));
        assert!(!status_needs_poll(OnchainReplenishmentRunStatus::Paused));
        assert!(!status_needs_poll(OnchainReplenishmentRunStatus::Completed));
    }
}
