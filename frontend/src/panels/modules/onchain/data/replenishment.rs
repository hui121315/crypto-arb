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
use super::snapshot_state::{ReadStamp, SnapshotState};

#[derive(Clone, Copy)]
pub(in crate::panels::modules::onchain) struct OnchainReplenishmentData {
    pub loaded: RwSignal<bool>,
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
    loaded: RwSignal<bool>,
    revision: RwSignal<u64>,
    reading: RwSignal<bool>,
    retry_at_ms: RwSignal<u64>,
    plan_context: RwSignal<Option<ReadStamp>>,
    pending_authorization: RwSignal<Option<String>>,
    pending_submission: RwSignal<Option<String>>,
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

impl ReplenishmentSignals {
    fn busy(self) -> bool {
        self.building.try_get_untracked() != Some(false)
            || self.authorizing.get_untracked()
            || self.submitting.get_untracked()
            || self.rechecking.get_untracked()
    }

    fn can_act(self) -> bool {
        self.loaded.get_untracked()
            && self.recovery_problem.get_untracked().is_none()
            && self.pending_authorization.get_untracked().is_none()
            && self.pending_submission.get_untracked().is_none()
    }

    fn begin(self) {
        self.revision.update(|revision| *revision = revision.wrapping_add(1));
    }
}

pub(super) fn use_replenishment(client: &ApiClient, snapshots: SnapshotState) -> OnchainReplenishmentData {
    let signals = ReplenishmentSignals {
        loaded: RwSignal::new(false),
        revision: RwSignal::new(0),
        reading: RwSignal::new(false),
        retry_at_ms: RwSignal::new(0),
        plan_context: RwSignal::new(None),
        pending_authorization: RwSignal::new(None),
        pending_submission: RwSignal::new(None),
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
    Effect::new(move |_| {
        let _epoch = snapshots.config_epoch();
        signals.plan.set(None);
        signals.plan_context.set(None);
        signals.confirmation.set(String::new());
        // Pending writes belong to their original run, not the current market draft.
    });
    recover_active_run(client, signals);
    OnchainReplenishmentData {
        loaded: signals.loaded,
        runs: signals.runs,
        plan: signals.plan,
        building: signals.building,
        build: build_callback(client.clone(), signals, snapshots),
        confirmation: signals.confirmation,
        authorizing: signals.authorizing,
        authorize: authorize_callback(client.clone(), signals, snapshots),
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
    snapshots: SnapshotState,
) -> Callback<OnchainReplenishmentBuildRequest> {
    Callback::new(move |request| {
        if signals.busy() || signals.pending_authorization.get_untracked().is_some()
            || signals.pending_submission.get_untracked().is_some() {
            return;
        }
        let Some(stamp) = snapshots.read_stamp() else { return; };
        let client = client.clone();
        signals.building.set(true);
        signals.plan.set(None);
        signals.plan_context.set(None);
        // A new preview must not hide or stop polling an existing funds transfer.
        signals.confirmation.set(String::new());
        signals.authorization_key.set(None);
        spawn_local(async move {
            let result = client
                .build_onchain_replenishment(&request)
                .await
                .map_err(|error| error.to_string());
            signals.building.try_set(false);
            if !snapshots.accepts_read(stamp) { return; }
            signals.plan_context.set(Some(stamp));
            if let Ok(plan) = &result {
                signals.authorization_key.set(Some(format!(
                    "onchain-replenishment-{}-{}",
                    plan.plan_id,
                    crate::state::polling::now_ms()
                )));
            }
            signals.plan.set(Some(result));
        });
    })
}

fn authorize_callback(client: ApiClient, signals: ReplenishmentSignals, snapshots: SnapshotState) -> Callback<String> {
    Callback::new(move |confirmation| {
        if signals.busy() || !signals.can_act()
            || confirmation != shared_types::ONCHAIN_REPLENISHMENT_AUTHORIZATION_PHRASE
            || !signals.plan_context.get_untracked().is_some_and(|stamp| snapshots.accepts_read(stamp)) {
            return;
        }
        let Some((plan_id, idempotency_key)) = authorization_scope(signals) else {
            return;
        };
        let client = client.clone();
        signals.begin();
        signals.pending_authorization.set(Some(idempotency_key.clone()));
        signals.authorizing.set(true);
        spawn_local(async move {
            let request = OnchainReplenishmentAuthorizeRequest {
                plan_id,
                idempotency_key,
                confirmation,
            };
            let result = client
                .authorize_onchain_replenishment(&request)
                .await;
            if signals.authorizing.try_get_untracked().is_none() { return; }
            match result {
                Ok(run) => {
                    signals.pending_authorization.set(None);
                    accept_latest_run(signals, run);
                }
                Err(error) => {
                    if matches!(error.problem.code.as_str(),
                        "ONCHAIN_REPLENISHMENT_PLAN_MISSING" | "ONCHAIN_REPLENISHMENT_PLAN_EXPIRED"
                        | "ONCHAIN_REPLENISHMENT_PLAN_NOT_READY" | "ONCHAIN_REPLENISHMENT_CONFIRMATION_REQUIRED"
                        | "ONCHAIN_REPLENISHMENT_AUTHORIZATION_INVALID") {
                        signals.pending_authorization.set(None);
                    }
                    signals.recovery_problem.set(Some(format!("授权反馈未确认，正在核对原记录：{error}")));
                }
            }
            signals.confirmation.set(String::new());
            signals.authorizing.set(false);
        });
    })
}

fn authorization_scope(signals: ReplenishmentSignals) -> Option<(String, String)> {
    let plan_id = signals
        .plan
        .get_untracked()?
        .ok()
        .filter(|plan| plan.status == shared_types::OnchainReplenishmentPlanStatus::ReadyForAuthorization
            && plan.submit_ready && plan.requires_live_authorization && plan.blockers.is_empty()
            && plan.valid_until_ms > crate::state::polling::now_ms() as i64)
        .map(|plan| plan.plan_id)?;
    let key = signals.authorization_key.get_untracked()?;
    Some((plan_id, key))
}

fn submit_callback(client: ApiClient, signals: ReplenishmentSignals) -> Callback<String> {
    Callback::new(move |run_id: String| {
        if signals.busy() || !signals.can_act() {
            return;
        }
        if !signals.run.with_untracked(|current| current.as_ref().and_then(|r| r.as_ref().ok())
            .is_some_and(|run| run.run_id == run_id
                && run.status == OnchainReplenishmentRunStatus::AuthorizedAwaitingSubmit
                && !run.read_only_recovery
                && run.authorization.valid_until_ms > crate::state::polling::now_ms() as i64)) { return; }
        let client = client.clone();
        signals.begin();
        signals.pending_submission.set(Some(run_id.clone()));
        signals.submitting.set(true);
        spawn_local(async move {
            let result = client
                .submit_onchain_replenishment(&OnchainReplenishmentSubmitRequest { run_id })
                .await;
            if signals.submitting.try_get_untracked().is_none() { return; }
            match result {
                Ok(run) => {
                    signals.pending_submission.set(None);
                    accept_latest_run(signals, run);
                }
                Err(error) => signals.recovery_problem.set(Some(format!("提交反馈未确认，保留原记录；不重复转账：{error}"))),
            }
            signals.submitting.set(false);
        });
    })
}

fn recheck_callback(client: ApiClient, signals: ReplenishmentSignals) -> Callback<shared_types::OnchainReplenishmentRecheckRequest> {
    Callback::new(move |request| {
        if signals.busy() || !signals.can_act() { return; }
        let current = signals.run.get_untracked().and_then(Result::ok);
        if current.as_ref().and_then(|r| r.recheck_request()).as_ref() != Some(&request) { return; }
        signals.begin();
        signals.rechecking.set(true);
        let client = client.clone();
        spawn_local(async move {
            let result = client.recheck_onchain_replenishment(&request).await;
            if signals.rechecking.try_get_untracked().is_none() { return; }
            match result {
                Ok(run) => accept_latest_run(signals, run),
                Err(error) => signals.recovery_problem.set(Some(format!("重新核验反馈未确认，保留原记录并刷新：{error}"))),
            }
            signals.rechecking.set(false);
        });
    })
}

fn accept_latest_run(signals: ReplenishmentSignals, latest: OnchainReplenishmentRun) {
    signals.runs.update(|rows| {
        if let Some(old) = rows.iter_mut().find(|old| old.run_id == latest.run_id) {
            if latest.updated_at_ms >= old.updated_at_ms { *old = latest.clone(); }
        } else {
            rows.push(latest.clone());
        }
        rows.sort_by_key(|row| std::cmp::Reverse(row.updated_at_ms));
    });
    let accept = signals.run.with_untracked(|current| match current {
        Some(Ok(current)) if current.run_id == latest.run_id => latest.updated_at_ms >= current.updated_at_ms,
        _ => true,
    });
    if accept && signals.run.get_untracked().as_ref() != Some(&Ok(latest.clone())) {
        signals.run.set(Some(Ok(latest)));
    }
}

fn accept_snapshot(signals: ReplenishmentSignals, snapshot: shared_types::OnchainReplenishmentRunsResponse) {
    let pending_key = signals.pending_authorization.get_untracked();
    let current_id = signals.run.get_untracked().and_then(Result::ok).map(|run| run.run_id);
    let recovered = snapshot.rows.iter().find(|row| pending_key.as_ref() == Some(&row.idempotency_key)).cloned();
    let selected = recovered.clone().or_else(|| snapshot.rows.iter().find(|row| current_id.as_ref() == Some(&row.run_id)).cloned())
        .or_else(|| current_id.is_none().then(|| snapshot.rows.first().cloned()).flatten());
    if recovered.is_some() { signals.pending_authorization.set(None); }
    if let Some(latest) = selected {
        let old = signals.run.get_untracked().and_then(Result::ok);
        if signals.pending_submission.get_untracked().as_ref() == Some(&latest.run_id)
            && old.as_ref().is_none_or(|old| latest.updated_at_ms >= old.updated_at_ms)
            && (latest.status != OnchainReplenishmentRunStatus::AuthorizedAwaitingSubmit || !latest.transfers.is_empty()) {
            signals.pending_submission.set(None);
        }
        accept_latest_run(signals, latest);
    }
    let selected_id = signals.run.get_untracked().and_then(Result::ok).map(|run| run.run_id);
    signals.runs.update(|rows| {
        rows.retain(|old| selected_id.as_ref() == Some(&old.run_id)
            || snapshot.rows.iter().any(|row| row.run_id == old.run_id));
        for row in snapshot.rows {
            if let Some(old) = rows.iter_mut().find(|old| old.run_id == row.run_id) {
                if row.updated_at_ms >= old.updated_at_ms { *old = row; }
            } else { rows.push(row); }
        }
        rows.sort_by_key(|row| std::cmp::Reverse(row.updated_at_ms));
    });
    signals.loaded.set(true);
    let pending = signals.pending_authorization.get_untracked().is_some() || signals.pending_submission.get_untracked().is_some();
    signals.recovery_problem.set(snapshot.recovery_problem.or_else(|| pending.then(|| "请求结果仍待核对；保留原记录，不重复提交".to_owned())));
}

fn recover_active_run(client: &ApiClient, signals: ReplenishmentSignals) {
    let run = signals.run;
    let poll = use_conditional_polling_result(
        Duration::from_secs(2),
        move || {
            !signals.busy() && !signals.reading.get_untracked()
                && crate::state::polling::now_ms() >= signals.retry_at_ms.get_untracked()
                && (!signals.loaded.get_untracked()
                || pending_run_id(run.try_get_untracked().flatten()).is_some()
                || signals
                    .recovery_problem
                    .try_get_untracked()
                    .flatten()
                    .is_some())
        },
        {
            let client = client.clone();
            move || {
                let client = client.clone();
                let revision = signals.revision.get_untracked();
                signals.reading.set(true);
                async move {
                    let result = client.onchain_replenishment_runs(20).await;
                    signals.reading.try_set(false);
                    result.map(|snapshot| (revision, snapshot)).map_err(|error| (revision, error))
                }
            }
        },
    );
    Effect::new(move |_| {
        let Some(result) = poll.get().and_then(|event| event.take().into_fetched()) else {
            return;
        };
        let snapshot = match result {
            Ok((revision, snapshot)) if signals.revision.get_untracked() == revision => snapshot,
            Ok(_) => return,
            Err((revision, _)) if signals.revision.get_untracked() != revision => return,
            Err((_, error)) => {
                signals.retry_at_ms.set(crate::state::polling::now_ms().saturating_add(5_000));
                signals
                    .recovery_problem
                    .set(Some(format!("补仓运行记录读取失败，保留上次结果：{error}")));
                return;
            }
        };
        accept_snapshot(signals, snapshot);
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
