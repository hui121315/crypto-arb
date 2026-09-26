use crate::api::rest::{with_mutation_timeout, ApiError};
use crate::panels::shared::operation_journal::{
    lookup_operation, OperationJournal, PendingOperation,
};
use crate::state::{action_state::ActionState, load_state::LoadState};
use leptos::{prelude::*, task::spawn_local};
use shared_types::{
    ActionRunKind, ActionRunStatus, CloseRun, CloseRunScope, CloseRunStatus,
    PortfolioSnapshot,
};

use super::super::{
    runs::{close_run_action_state, close_run_next_attempt_anchor},
    snapshot::merge_close_run_receipt,
};

#[derive(Clone, Copy)]
pub(in crate::panels::modules::positions) struct CloseRecovery {
    pub journal: OperationJournal,
    pub state: RwSignal<ActionState>,
    pub active_key: RwSignal<Option<String>>,
    receipt: RwSignal<Option<CloseRun>>,
    snapshot: RwSignal<LoadState<PortfolioSnapshot>>,
}

impl CloseRecovery {
    pub(in crate::panels::modules::positions) fn new(
        snapshot: RwSignal<LoadState<PortfolioSnapshot>>,
    ) -> Self {
        let journal = OperationJournal::new("position-close");
        let recovery = Self {
            journal,
            snapshot,
            receipt: RwSignal::new(None),
            state: RwSignal::new(ActionState::Idle),
            active_key: RwSignal::new(None),
        };
        Effect::new(move |_| {
            journal.connection.track();
            recovery.receipt.set(None);
            recovery.state.set(journal.restored_state(&[
                ActionRunKind::PortfolioClosePosition,
                ActionRunKind::PortfolioClosePair,
                ActionRunKind::PortfolioCloseAll,
            ]));
        });
        Effect::new(move |_| {
            let locked = journal.locked();
            recovery.active_key.update(|key| {
                if locked {
                    key.get_or_insert_with(|| "pending-close".into());
                } else {
                    *key = None;
                }
            });
        });
        Effect::new(move |_| {
            let snapshot = snapshot.get();
            let Some(attempt) = journal.pending.get_untracked() else {
                return;
            };
            if let Some(run) = snapshot.value().and_then(|snapshot| {
                snapshot
                    .recent_close_runs
                    .iter()
                    .find(|run| identity_matches(&attempt, run))
            }) {
                // Snapshot already contains this receipt; do not write it back and loop.
                recovery.accept(&attempt, run.clone(), false);
            }
        });
        Effect::new(move |_| {
            let snapshot = snapshot.get();
            if let (Some(snapshot), Some(receipt)) = (snapshot.value(), recovery.receipt.get()) {
                if !snapshot
                    .recent_close_runs
                    .iter()
                    .any(|run| run.id == receipt.id && run.updated_at_ms >= receipt.updated_at_ms)
                {
                    merge_close_run_receipt(recovery.snapshot, receipt);
                }
            }
        });
        recovery
    }

    pub(super) fn failed(self, attempt: &PendingOperation, error: ApiError) {
        if preflight_rejected(&error.problem) {
            self.state.set(
                ActionState::failed("平仓未提交，请刷新持仓后重试", error.problem)
                    .with_evidence(attempt.context.evidence()),
            );
            self.resolve(attempt);
            self.journal.busy.set(false);
            return;
        }
        self.state.set(
            ActionState::failed("平仓结果待核对", error.problem).with_evidence(
                attempt
                    .context
                    .evidence()
                    .with_action_run_id(attempt.run_id.clone()),
            ),
        );
        self.journal.problem.set(Some(
            "请求返回异常，不能认定未下单；请核对原平仓结果。".into(),
        ));
        self.journal.busy.set(false);
    }

    pub(super) fn accept(
        self,
        attempt: &PendingOperation,
        run: CloseRun,
        merge: bool,
    ) -> Option<CloseRun> {
        if let Err(error) = validate_receipt(attempt, &run) {
            self.failed(attempt, error);
            return None;
        }
        let run = self
            .receipt
            .get_untracked()
            .filter(|known| known.id == run.id && known.updated_at_ms > run.updated_at_ms)
            .unwrap_or(run);
        let run = if merge {
            merge_close_run_receipt(self.snapshot, run)
        } else {
            run
        };
        if let Err(error) = validate_receipt(attempt, &run) {
            self.failed(attempt, error);
            return None;
        }
        self.journal.remember_run(
            attempt,
            run.action_run_id.clone().expect("validated action id"),
        );
        self.receipt.set(Some(run.clone()));
        self.state.set(close_run_action_state("原平仓", &run));
        let known = self
            .journal
            .pending
            .get_untracked()
            .expect("remembered operation");
        if close_run_next_attempt_anchor(&run).is_some() {
            self.resolve(&known);
        }
        Some(run)
    }

    fn resolve(self, attempt: &PendingOperation) {
        if self.journal.resolve(attempt) {
            // A WS terminal receipt can finish before HTTP. Invalidate its late reply.
            self.journal
                .epoch
                .update(|epoch| *epoch = epoch.wrapping_add(1));
            self.journal.busy.set(false);
        }
    }

    fn recheck(self) -> Callback<()> {
        Callback::new(move |()| {
            let journal = self.journal;
            let epoch = journal.epoch.get_untracked();
            if journal.busy.get_untracked() || !journal.current(epoch) {
                return;
            }
            let Some(attempt) = journal.pending.get_untracked() else {
                journal.restore();
                return;
            };
            let client = journal.client();
            journal.busy.set(true);
            journal.problem.set(None);
            spawn_local(async move {
                let result =
                    with_mutation_timeout("核对原平仓", lookup_operation(&client, &attempt)).await;
                if !journal.current(epoch) {
                    return;
                }
                match result {
                    Ok(action) => {
                        journal.remember_run(&attempt, action.id.clone());
                        let known = journal
                            .pending
                            .get_untracked()
                            .expect("remembered operation");
                        if action.status == ActionRunStatus::Failed && action.result.is_none() {
                            if let Some(problem) = action.problem.filter(preflight_rejected) {
                                self.failed(&known, ApiError::from_problem(problem));
                                return;
                            }
                        }
                        match action
                            .result
                            .and_then(|value| serde_json::from_value::<CloseRun>(value).ok())
                        {
                            Some(run) if action.status == action_status(run.status) => {
                                self.accept(&known, run, true);
                            }
                            _ => {
                                journal.problem.set(Some(
                                    if action.status == ActionRunStatus::Accepted {
                                        "后端已受理，尚无订单最终结果；请稍后核对。"
                                    } else {
                                        "原动作缺少一致的平仓结果，仍需核对，不能重新提交。"
                                    }
                                    .into(),
                                ));
                            }
                        }
                    }
                    Err(error) => self.failed(&attempt, error),
                }
                journal.busy.set(false);
            });
        })
    }
}

fn preflight_rejected(problem: &shared_types::ApiProblem) -> bool {
    // These codes are emitted before submit_close_leg in the backend request validator.
    matches!(
        problem.code.as_str(),
        shared_types::problem::codes::CLOSE_RUN_REQUEST_INVALID
            | shared_types::problem::codes::CLOSE_RUN_STALE_SNAPSHOT
            | shared_types::problem::codes::CLOSE_RUN_EXPECTED_LEG_MISMATCH
    )
}

fn identity_matches(attempt: &PendingOperation, run: &CloseRun) -> bool {
    run.request_id.as_deref() == Some(attempt.context.request_id())
        && run.idempotency_key.as_deref() == attempt.context.idempotency_key()
        && run.action_run_id.as_ref().is_some_and(|id| {
            !id.is_empty()
                && attempt
                    .run_id
                    .as_ref()
                    .is_none_or(|expected| expected == id)
        })
}

fn validate_receipt(attempt: &PendingOperation, run: &CloseRun) -> Result<(), ApiError> {
    let scope_matches = match attempt.kind {
        ActionRunKind::PortfolioClosePosition => run.scope == CloseRunScope::Single,
        ActionRunKind::PortfolioClosePair => run.scope == CloseRunScope::Pair,
        ActionRunKind::PortfolioCloseAll => {
            run.scope == CloseRunScope::All && attempt.target == "all-positions"
        }
        _ => false,
    };
    let target_matches = run.scope == CloseRunScope::All
        || run
            .legs
            .iter()
            .any(|leg| format!("{}:{}", leg.venue, leg.symbol) == attempt.target);
    let filled = run.status != CloseRunStatus::Succeeded || run.has_complete_fills();
    if !identity_matches(attempt, run)
        || !scope_matches
        || !target_matches
        || !filled
        || run.id.is_empty()
    {
        return Err(ApiError::client(
            "CLOSE_RECEIPT_MISMATCH",
            "平仓结果身份或订单状态不一致，保留原请求待核对",
        ));
    }
    Ok(())
}

fn action_status(status: CloseRunStatus) -> ActionRunStatus {
    match status {
        CloseRunStatus::Submitted | CloseRunStatus::CompensationSubmitted => {
            ActionRunStatus::Accepted
        }
        CloseRunStatus::Succeeded
        | CloseRunStatus::Compensated
        | CloseRunStatus::ManuallyResolved => ActionRunStatus::Succeeded,
        _ => ActionRunStatus::Failed,
    }
}

pub(in crate::panels::modules::positions) fn close_recovery_panel(
    recovery: CloseRecovery,
) -> impl IntoView {
    let journal = recovery.journal;
    let recheck = recovery.recheck();
    view! {
        <Show when=move || journal.locked()>
            <div class="provider-credentials-feedback provider-credentials-recovery has-action" role="alert" aria-label="平仓操作待核对">
                <span class="provider-credentials-recovery-copy">
                    <strong>{move || journal.pending.with(|pending| pending.as_ref().map_or_else(
                        || "平仓恢复记录不可用，已暂停重复提交".into(),
                        |attempt| format!("{} · {}", if attempt.kind == ActionRunKind::PortfolioCloseAll {
                            "全部持仓" } else { &attempt.target }, if journal.busy.get() { "平仓处理中" } else { "平仓待核对" })
                    ))}</strong>
                    <span>"原请求未确认前，不重复平仓。"</span>
                    {move || journal.problem.get().map(|message| view! { <span>{message}</span> })}
                    <details class="positions-action-evidence">
                        <summary>"原平仓详情"</summary>
                        <span>{move || recovery.state.get().message("")}</span>
                    </details>
                </span>
                <button type="button" class="row-action" disabled=move || journal.busy.get() on:click=move |_| recheck.run(())>
                    {move || if journal.busy.get() { "处理中…" } else { "核对原平仓" }}
                </button>
            </div>
        </Show>
    }
}
