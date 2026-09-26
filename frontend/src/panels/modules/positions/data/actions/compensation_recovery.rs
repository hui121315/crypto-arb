use crate::api::rest::{with_mutation_timeout, ApiError, MutationRequestContext};
use crate::panels::modules::positions::data::snapshot::merge_close_run_receipt;
use crate::panels::shared::operation_journal::{
    lookup_operation, OperationJournal, PendingOperation,
};
use crate::state::{action_state::ActionState, load_state::LoadState};
use leptos::{prelude::*, task::spawn_local};
use shared_types::{ActionRunKind, ActionRunStatus, CloseRun, OrderRecord, PortfolioSnapshot};

#[path = "compensation_receipt.rs"]
mod receipt;

#[derive(Clone, Copy)]
pub(super) struct RemedyRecovery {
    pub journal: OperationJournal,
    pub state: RwSignal<ActionState>,
    label: &'static str,
    snapshot: RwSignal<LoadState<PortfolioSnapshot>>,
    receipt: RwSignal<Option<CloseRun>>,
    order: RwSignal<Option<OrderRecord>>,
}

impl RemedyRecovery {
    pub(super) fn new(
        domain: &'static str,
        label: &'static str,
        snapshot: RwSignal<LoadState<PortfolioSnapshot>>,
    ) -> Self {
        let journal = OperationJournal::new(domain);
        let recovery = Self {
            journal,
            label,
            snapshot,
            state: RwSignal::new(ActionState::Idle),
            receipt: RwSignal::new(None),
            order: RwSignal::new(None),
        };
        Effect::new(move |_| {
            journal.connection.track();
            recovery.receipt.set(None);
            recovery.order.set(None);
            recovery.state.set(journal.restored_state(&[
                ActionRunKind::PortfolioCloseCompensation,
                ActionRunKind::PortfolioCloseManualTerminal,
                ActionRunKind::TradingOrderCancel,
            ]));
        });
        Effect::new(move |_| {
            let snapshot = snapshot.get();
            let Some(snapshot) = snapshot.value() else {
                return;
            };
            let Some(attempt) = journal
                .pending
                .get_untracked()
                .filter(|p| p.run_id.is_some())
            else {
                return;
            };
            if attempt.kind == ActionRunKind::TradingOrderCancel {
                let orders = snapshot
                    .recent_close_runs
                    .iter()
                    .filter_map(|run| run.unwind_plan.as_ref())
                    .flat_map(|plan| &plan.compensation_attempts)
                    .filter_map(|a| a.order.as_ref())
                    .filter(|order| order.intent.id == attempt.target)
                    .collect::<Vec<_>>();
                if orders.len() == 1 {
                    recovery.accept_order(&attempt, orders[0].clone());
                }
            } else if let Some(run) = snapshot
                .recent_close_runs
                .iter()
                .find(|run| run.id == attempt.target)
            {
                // Only an attempt identified by the action ledger may advance via WS.
                if receipt::run_outcome(&attempt, run).is_ok() {
                    recovery.accept_run(&attempt, run.clone(), false);
                }
            }
        });
        Effect::new(move |_| {
            let snapshot = snapshot.get();
            if let (Some(snapshot), Some(run)) = (snapshot.value(), recovery.receipt.get()) {
                if !snapshot
                    .recent_close_runs
                    .iter()
                    .any(|r| r.id == run.id && r.updated_at_ms >= run.updated_at_ms)
                {
                    merge_close_run_receipt(recovery.snapshot, run);
                }
            }
        });
        recovery
    }

    pub(super) fn begin(
        self,
        kind: ActionRunKind,
        target: String,
        context: MutationRequestContext,
    ) -> Option<PendingOperation> {
        let attempt = self.journal.begin_with_context(kind, target, context)?;
        self.receipt.set(None);
        self.order.set(None);
        self.state.set(
            ActionState::pending(format!("{}处理中", self.label))
                .with_evidence(receipt::evidence(&attempt)),
        );
        Some(attempt)
    }

    pub(super) fn failed(self, attempt: &PendingOperation, error: ApiError) {
        let rejected = attempt.kind != ActionRunKind::TradingOrderCancel
            && matches!(
                error.problem.code.as_str(),
                "CLOSE_RUN_REQUEST_INVALID" | "CLOSE_RUN_STALE_SNAPSHOT"
            );
        self.state.set(
            ActionState::failed(
                if rejected {
                    "操作未提交，请刷新记录后重试"
                } else {
                    "操作结果待核对"
                },
                error.problem,
            )
            .with_evidence(receipt::evidence(attempt)),
        );
        if rejected {
            self.resolve(attempt);
        } else {
            self.journal.problem.set(Some(
                "不能据此认定未执行；请查询原处理结果，不要重复提交。".into(),
            ));
        }
        self.journal.busy.set(false);
    }

    fn resolve(self, attempt: &PendingOperation) {
        if self.journal.resolve(attempt) {
            self.journal
                .epoch
                .update(|epoch| *epoch = epoch.wrapping_add(1));
        }
    }

    pub(super) fn order_finished(self, id: &str) -> bool {
        self.order.with(|order| {
            order
                .as_ref()
                .is_some_and(|order| order.intent.id == id && receipt::terminal_order(order))
        })
    }

    fn accept_run(self, attempt: &PendingOperation, run: CloseRun, merge: bool) {
        let run = self
            .receipt
            .get_untracked()
            .filter(|known| known.id == run.id && known.updated_at_ms > run.updated_at_ms)
            .unwrap_or(run);
        let (state, terminal) = match receipt::run_outcome(attempt, &run) {
            Ok(result) => result,
            Err(error) => {
                self.failed(attempt, error);
                return;
            }
        };
        self.state.set(state);
        self.receipt.set(Some(run.clone()));
        if merge {
            merge_close_run_receipt(self.snapshot, run);
        }
        if terminal {
            self.resolve(attempt);
        }
        self.journal.busy.set(false);
    }

    fn accept_order(self, attempt: &PendingOperation, order: OrderRecord) {
        let order = self
            .order
            .get_untracked()
            .filter(|known| {
                known.intent.id == order.intent.id && known.updated_at_ms > order.updated_at_ms
            })
            .unwrap_or(order);
        let state = match receipt::cancel_outcome(attempt, &order) {
            Ok(state) => state,
            Err(error) => {
                self.failed(attempt, error);
                return;
            }
        };
        self.state.set(state);
        let terminal = receipt::terminal_order(&order);
        self.order.set(Some(order));
        if terminal {
            self.resolve(attempt);
        }
        self.journal.busy.set(false);
    }

    pub(super) async fn lookup(self, attempt: PendingOperation, epoch: u64) {
        let client = self.journal.client();
        let result =
            with_mutation_timeout("查询原处理处理结果", lookup_operation(&client, &attempt)).await;
        if !self.journal.current(epoch) {
            return;
        }
        let action = match result {
            Ok(action) => action,
            Err(error) => {
                self.failed(&attempt, error);
                return;
            }
        };
        self.journal.remember_run(&attempt, action.id.clone());
        let mut known = attempt;
        known.run_id = Some(action.id);
        let Some(payload) = action.result else {
            if action.status == ActionRunStatus::Failed {
                if let Some(problem) = action.problem {
                    self.failed(&known, ApiError { problem });
                    return;
                }
            }
            self.failed(
                &known,
                ApiError::client("REMEDY_RESULT_PENDING", "原请求已找到，结果尚不可核对"),
            );
            return;
        };
        if known.kind == ActionRunKind::TradingOrderCancel {
            // Action Succeeded means the cancel handler returned, not exchange finality.
            let order = serde_json::from_value::<OrderRecord>(payload)
                .map_err(|_| receipt::mismatch())
                .and_then(|order| {
                    receipt::cancel_outcome(&known, &order)?;
                    Ok(order)
                });
            if let Err(error) = order {
                self.failed(&known, error);
                return;
            }
            let result =
                with_mutation_timeout("查询原补救订单", client.trading_order(&known.target)).await;
            if !self.journal.current(epoch) {
                return;
            }
            match result {
                Ok(order) => self.accept_order(&known, order),
                Err(error) => self.failed(&known, error),
            }
        } else {
            match serde_json::from_value::<CloseRun>(payload) {
                Ok(run) => self.accept_run(&known, run, true),
                Err(_) => self.failed(&known, receipt::mismatch()),
            }
        }
    }

    pub(super) fn panel(self) -> impl IntoView {
        let journal = self.journal;
        view! {
            <Show when=move || journal.locked()>
                <div class="provider-credentials-feedback provider-credentials-recovery has-action" role="alert" aria-label=self.label>
                    <span class="provider-credentials-recovery-copy">
                        <strong>{move || format!("{} · {}", self.label, if journal.busy.get() { "处理中" } else { "待核对" })}</strong>
                        <span>{move || self.state.get().label().unwrap_or("原请求结果待核对").to_owned()}</span>
                        {move || journal.problem.get().map(|message| view! { <span>{message}</span> })}
                        <details class="positions-action-evidence">
                            <summary>"原请求详情"</summary>
                            <span>{move || self.state.get().message("")}</span>
                        </details>
                    </span>
                    <button type="button" class="row-action" disabled=move || journal.busy.get()
                        on:click=move |_| {
                            if journal.busy.get_untracked() { return; }
                            let Some(attempt) = journal.pending.get_untracked() else { journal.restore(); return; };
                            let epoch = journal.epoch.get_untracked();
                            journal.busy.set(true);
                            journal.problem.set(None);
                            spawn_local(async move { self.lookup(attempt, epoch).await; });
                        }>"核对原处理"</button>
                </div>
            </Show>
        }
    }
}
