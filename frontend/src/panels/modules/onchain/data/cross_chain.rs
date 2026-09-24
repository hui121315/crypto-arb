use crate::api::rest::ApiClient;
use gloo_timers::callback::Interval;
use leptos::prelude::*;
use leptos::task::spawn_local;
use super::snapshot_state::{ReadStamp, SnapshotState};
use shared_types::{
    ApiProblem, OnchainCrossChainAuthorizeRequest, OnchainCrossChainBuildRequest,
    OnchainCrossChainBuildResponse, OnchainCrossChainRecheckRequest,
    OnchainCrossChainSubmitRequest, ONCHAIN_CROSS_CHAIN_AUTHORIZATION_PHRASE,
    OnchainCrossChainRecoveryPreview, OnchainCrossChainRecoveryPreviewRequest,
    OnchainCrossChainRecoveryAuthorizeRequest,
};

#[path = "cross_chain/recovery.rs"]
mod recovery;
pub(in crate::panels::modules::onchain) use recovery::{
    next_recheck_position, next_submit_position, CrossChainRecovery,
};

#[derive(Clone, Copy)]
pub(in crate::panels::modules::onchain) struct OnchainCrossChainData {
    pub selected_approvals: RwSignal<Vec<String>>,
    pub selected_replenishments: RwSignal<Vec<String>>,
    pub build: RwSignal<Option<Result<OnchainCrossChainBuildResponse, ApiProblem>>>,
    pub building: RwSignal<bool>,
    pub build_preview: Callback<OnchainCrossChainBuildRequest>,
    pub confirmation: RwSignal<String>,
    pub authorizing: RwSignal<bool>,
    pub authorize: Callback<String>,
    pub recovery: RwSignal<CrossChainRecovery>,
    pub refreshing: RwSignal<bool>,
    pub refresh: Callback<()>,
    pub submitting: RwSignal<bool>,
    pub submit_next_leg: Callback<OnchainCrossChainSubmitRequest>,
    pub rechecking: RwSignal<bool>,
    pub recheck: Callback<OnchainCrossChainRecheckRequest>,
    pub recovery_preview: RwSignal<Option<Result<OnchainCrossChainRecoveryPreview, ApiProblem>>>,
    pub recovery_preview_request: RwSignal<Option<OnchainCrossChainRecoveryPreviewRequest>>,
    pub recovery_previewing: RwSignal<bool>,
    pub preview_recovery: Callback<OnchainCrossChainRecoveryPreviewRequest>,
    pub recovery_mutating: RwSignal<bool>,
    pub reserve_recovery: Callback<OnchainCrossChainRecoveryAuthorizeRequest>,
    pub cancel_recovery: Callback<String>,
}

#[derive(Clone, Copy)]
struct Signals {
    build_context: RwSignal<Option<ReadStamp>>,
    build: RwSignal<Option<Result<OnchainCrossChainBuildResponse, ApiProblem>>>,
    building: RwSignal<bool>,
    confirmation: RwSignal<String>,
    authorizing: RwSignal<bool>,
    recovery: RwSignal<CrossChainRecovery>,
    refreshing: RwSignal<bool>,
    submitting: RwSignal<bool>,
    rechecking: RwSignal<bool>,
    revision: RwSignal<u64>,
    recovery_preview: RwSignal<Option<Result<OnchainCrossChainRecoveryPreview, ApiProblem>>>,
    recovery_preview_request: RwSignal<Option<OnchainCrossChainRecoveryPreviewRequest>>,
    recovery_previewing: RwSignal<bool>,
    recovery_mutating: RwSignal<bool>,
}
impl Signals {
    fn busy(self) -> bool {
        self.building.try_get_untracked() != Some(false)
            || self.authorizing.get_untracked()
            || self.submitting.get_untracked()
            || self.rechecking.get_untracked()
            || self.recovery_previewing.get_untracked()
            || self.recovery_mutating.get_untracked()
    }
    fn begin(self) {
        self.revision.update(|revision| *revision += 1);
        self.recovery.update(|state| state.problem = None);
    }
}

pub(super) fn use_cross_chain(client: &ApiClient, snapshots: SnapshotState) -> OnchainCrossChainData {
    let s = Signals {
        build_context: RwSignal::new(None),
        build: RwSignal::new(None),
        building: RwSignal::new(false),
        confirmation: RwSignal::new(String::new()),
        authorizing: RwSignal::new(false),
        recovery: RwSignal::new(CrossChainRecovery::default()),
        refreshing: RwSignal::new(false),
        submitting: RwSignal::new(false),
        rechecking: RwSignal::new(false),
        revision: RwSignal::new(0),
        recovery_preview: RwSignal::new(None),
        recovery_preview_request: RwSignal::new(None),
        recovery_previewing: RwSignal::new(false),
        recovery_mutating: RwSignal::new(false),
    };
    Effect::new(move |_| {
        let _epoch = snapshots.config_epoch();
        s.build.set(None);
        s.build_context.set(None);
        s.confirmation.set(String::new());
        s.recovery_preview.set(None);
        s.recovery_preview_request.set(None);
    });
    let refresh = refresh_callback(client.clone(), s);
    let timer = StoredValue::new_local(None::<Interval>);
    Effect::new(move |_| {
        refresh.run(());
        timer.set_value(Some(Interval::new(2_000, move || {
            if s.recovery
                .try_with_untracked(CrossChainRecovery::needs_poll)
                == Some(true)
            {
                refresh.run(());
            }
        })));
    });
    on_cleanup(move || {
        timer.update_value(|timer| {
            timer.take();
        })
    });
    OnchainCrossChainData {
        selected_approvals: RwSignal::new(Vec::new()),
        selected_replenishments: RwSignal::new(Vec::new()),
        build: s.build,
        building: s.building,
        build_preview: build_callback(client.clone(), s, snapshots),
        confirmation: s.confirmation,
        authorizing: s.authorizing,
        authorize: authorize_callback(client.clone(), s, snapshots, refresh),
        recovery: s.recovery,
        refreshing: s.refreshing,
        refresh,
        submitting: s.submitting,
        submit_next_leg: submit_callback(client.clone(), s, refresh),
        rechecking: s.rechecking,
        recheck: recheck_callback(client.clone(), s, refresh),
        recovery_preview: s.recovery_preview,
        recovery_preview_request: s.recovery_preview_request,
        recovery_previewing: s.recovery_previewing,
        preview_recovery: recovery_preview_callback(client.clone(), s, snapshots, refresh),
        recovery_mutating: s.recovery_mutating,
        reserve_recovery: reserve_recovery_callback(client.clone(), s, refresh),
        cancel_recovery: cancel_recovery_callback(client.clone(), s, refresh),
    }
}

fn recovery_preview_callback(client: ApiClient, s: Signals, snapshots: SnapshotState, refresh: Callback<()>) -> Callback<OnchainCrossChainRecoveryPreviewRequest> {
    Callback::new(move |request: OnchainCrossChainRecoveryPreviewRequest| {
        if s.busy() || !s.recovery.with_untracked(|state| state.recovery_actions_ready() && state.selected().is_some_and(|run|
                run.run_id == request.run_id && run.updated_at_ms == request.expected_run_updated_at_ms)) { return; }
        let Some(stamp) = snapshots.read_stamp() else { return; };
        s.begin();
        s.recovery_preview_request.set(Some(request.clone()));
        s.recovery_preview.set(None);
        s.recovery_previewing.set(true);
        let client = client.clone();
        spawn_local(async move {
            let result = client.preview_onchain_cross_chain_recovery(&request).await.map_err(|e| e.problem);
            if snapshots.accepts_read(stamp) && s.recovery.try_with_untracked(|state| state.selected().is_some_and(|run|
                run.run_id == request.run_id && run.updated_at_ms == request.expected_run_updated_at_ms)) == Some(true) {
                s.recovery_preview.try_set(Some(result));
            }
            s.recovery_previewing.try_set(false);
            if s.recovery.try_get_untracked().is_some() { refresh.run(()); }
        });
    })
}

fn reserve_recovery_callback(client: ApiClient, s: Signals, refresh: Callback<()>) -> Callback<OnchainCrossChainRecoveryAuthorizeRequest> {
    Callback::new(move |request: OnchainCrossChainRecoveryAuthorizeRequest| {
        if s.busy() || !s.recovery.with_untracked(|state| state.can_reserve_recovery(&request.plan_id, now_ms())) { return; }
        s.begin(); s.recovery_mutating.set(true);
        let client = client.clone();
        spawn_local(async move {
            let result = client.reserve_onchain_cross_chain_recovery(&request).await;
            s.recovery.try_update(|state| match result {
                Ok(plan) => { state.accept_recovery_plan(plan); state.problem = None; },
                Err(error) => state.problem = Some(error.to_string()),
            });
            s.recovery_mutating.try_set(false);
            if s.recovery.try_get_untracked().is_some() { refresh.run(()); }
        });
    })
}

fn cancel_recovery_callback(client: ApiClient, s: Signals, refresh: Callback<()>) -> Callback<String> {
    Callback::new(move |plan_id: String| {
        if s.busy() || !s.recovery.with_untracked(|state| state.can_cancel_recovery(&plan_id, now_ms())) { return; }
        s.begin(); s.recovery_mutating.set(true);
        let client = client.clone();
        spawn_local(async move {
            let result = client.cancel_onchain_cross_chain_recovery(&plan_id).await;
            s.recovery.try_update(|state| match result {
                Ok(plan) => { state.accept_recovery_plan(plan); state.problem = None; },
                Err(error) => state.problem = Some(error.to_string()),
            });
            s.recovery_mutating.try_set(false);
            if s.recovery.try_get_untracked().is_some() { refresh.run(()); }
        });
    })
}

fn build_callback(client: ApiClient, s: Signals, snapshots: SnapshotState) -> Callback<OnchainCrossChainBuildRequest> {
    Callback::new(move |request| {
        if s.busy() || !s.recovery.with_untracked(|state| state.can_build(now_ms())) {
            return;
        }
        let Some(stamp) = snapshots.read_stamp() else { return; };
        s.begin();
        s.building.set(true);
        s.build.set(None);
        s.build_context.set(None);
        s.confirmation.set(String::new());
        let client = client.clone();
        spawn_local(async move {
            let result = client
                .build_onchain_cross_chain_preview(&request)
                .await
                .map_err(|error| error.problem);
            s.building.try_set(false);
            if snapshots.accepts_read(stamp) {
                s.build_context.set(Some(stamp));
                s.build.set(Some(result));
            }
        });
    })
}

pub(in crate::panels::modules::onchain) fn authorization_key(build_id: &str) -> String {
    format!("onchain-cross-chain-{build_id}")
}

fn authorize_callback(client: ApiClient, s: Signals, snapshots: SnapshotState, refresh: Callback<()>) -> Callback<String> {
    Callback::new(move |confirmation: String| {
        if s.busy() || confirmation != ONCHAIN_CROSS_CHAIN_AUTHORIZATION_PHRASE {
            return;
        }
        if !s.build_context.get_untracked().is_some_and(|stamp| snapshots.accepts_read(stamp)) {
            return;
        }
        let Some(Ok(build)) = s.build.get_untracked() else {
            return;
        };
        let key = authorization_key(&build.build_id);
        let retrying = s
            .recovery
            .with_untracked(|state| state.pending_authorization.as_ref() == Some(&key));
        if !s
            .recovery
            .with_untracked(|state| state.can_authorize(&key, now_ms()))
            || !retrying
                && (!build.submit_ready
                    || !build.blockers.is_empty()
                    || now_ms() >= build.valid_until_ms)
        {
            return;
        }
        s.begin();
        s.authorizing.set(true);
        s.recovery
            .update(|state| state.pending_authorization = Some(key.clone()));
        let client = client.clone();
        spawn_local(async move {
            let result = client
                .authorize_onchain_cross_chain(&OnchainCrossChainAuthorizeRequest {
                    build_id: build.build_id,
                    idempotency_key: key,
                    confirmation,
                })
                .await;
            s.recovery.try_update(|state| match result {
                Ok(run) => {
                    state.pending_authorization = None;
                    state.accept_run(run, true);
                }
                Err(error) => {
                    // Only explicit contract rejections prove no authorization was created.
                    if matches!(
                        error.problem.code.as_str(),
                        "ONCHAIN_CROSS_CHAIN_BUILD_EXPIRED"
                            | "ONCHAIN_CROSS_CHAIN_BUILD_MISSING"
                            | "ONCHAIN_CROSS_CHAIN_BUILD_NOT_READY"
                            | "ONCHAIN_CROSS_CHAIN_CONFIRMATION_REQUIRED"
                    ) {
                        state.pending_authorization = None;
                    }
                    state.problem = Some(error.to_string());
                }
            });
            s.confirmation.try_set(String::new());
            s.authorizing.try_set(false);
            if s.recovery.try_get_untracked().is_some() {
                refresh.run(());
            }
        });
    })
}

fn submit_callback(
    client: ApiClient,
    s: Signals,
    refresh: Callback<()>,
) -> Callback<OnchainCrossChainSubmitRequest> {
    Callback::new(move |request: OnchainCrossChainSubmitRequest| {
        if s.busy()
            || !s
                .recovery
                .with_untracked(|state| state.can_submit(&request, now_ms()))
        {
            return;
        }
        s.begin();
        s.submitting.set(true);
        s.recovery
            .update(|state| state.pending_submission = Some(request.run_id.clone()));
        let client = client.clone();
        spawn_local(async move {
            let result = client.submit_onchain_cross_chain(&request).await;
            s.recovery.try_update(|state| match result {
                Ok(run) => {
                    state.pending_submission = None;
                    state.accept_run(run, true);
                }
                Err(error) => state.problem = Some(error.to_string()),
            });
            s.submitting.try_set(false);
            if s.recovery.try_get_untracked().is_some() {
                refresh.run(());
            }
        });
    })
}

fn refresh_callback(client: ApiClient, s: Signals) -> Callback<()> {
    Callback::new(move |_| {
        if s.busy() || s.refreshing.get_untracked() {
            return;
        }
        s.refreshing.set(true);
        let revision = s.revision.get_untracked();
        let client = client.clone();
        spawn_local(async move {
            let result = client.onchain_cross_chain_runs(128).await;
            // Discard a read started before a write, even if it finishes after that write.
            if s.revision.try_get_untracked() == Some(revision) {
                s.recovery.try_update(|state| match result {
                    Ok(snapshot) => {
                        state.accept_snapshot(snapshot.rows, now_ms());
                        state.plans.retain(|old| snapshot.recovery_plans.iter().any(|plan| plan.plan_id == old.plan_id));
                        for plan in snapshot.recovery_plans { state.accept_recovery_plan(plan); }
                        state.recovery_problem = snapshot.recovery_problem;
                    },
                    Err(error) => state.read_problem = Some(error.to_string()),
                });
            }
            s.refreshing.try_set(false);
        });
    })
}

fn recheck_callback(
    client: ApiClient,
    s: Signals,
    refresh: Callback<()>,
) -> Callback<OnchainCrossChainRecheckRequest> {
    Callback::new(move |request: OnchainCrossChainRecheckRequest| {
        if s.busy()
            || !s
                .recovery
                .with_untracked(|state| state.can_recheck(&request))
        {
            return;
        }
        s.begin();
        s.rechecking.set(true);
        let client = client.clone();
        spawn_local(async move {
            let result = client.recheck_onchain_cross_chain(&request).await;
            s.recovery.try_update(|state| match result {
                Ok(run) => state.accept_run(run, true),
                Err(error) => state.problem = Some(error.to_string()),
            });
            s.rechecking.try_set(false);
            if s.recovery.try_get_untracked().is_some() {
                refresh.run(());
            }
        });
    })
}
fn now_ms() -> i64 {
    crate::state::polling::now_ms() as i64
}
