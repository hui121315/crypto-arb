//! Recover compensation actions independently from the original close request.

use super::super::{requests::*, runs::*};
use super::close_request_context;
use crate::api::rest::{with_mutation_timeout, MutationRequestContext};
use crate::state::{action_state::ActionState, load_state::LoadState};
use leptos::{prelude::*, task::spawn_local};
use shared_types::{ActionRunKind, CloseRun, PortfolioSnapshot};

#[path = "compensation_recovery.rs"]
mod recovery;
use recovery::RemedyRecovery;

#[derive(Clone)]
pub(in crate::panels::modules::positions) struct CloseRunCompensationInput {
    pub run: CloseRun,
    pub candidate_index: usize,
    pub confirmation_phrase: String,
}
#[derive(Clone)]
pub(in crate::panels::modules::positions) struct CloseRunCompensationCancelInput {
    pub run: CloseRun,
    pub order_id: String,
}
#[derive(Clone)]
pub(in crate::panels::modules::positions) struct CloseRunManualTerminalInput {
    pub run: CloseRun,
    pub confirmation_phrase: String,
    pub reason: String,
    pub evidence: String,
    pub manual_handling_cost_usd: String,
}

#[derive(Clone, Copy)]
pub(in crate::panels::modules::positions) struct CloseRunCompensationAction {
    pub state: RwSignal<ActionState>,
    pub cancel_state: RwSignal<ActionState>,
    pub active_key: RwSignal<Option<String>>,
    pub submit: Callback<CloseRunCompensationInput>,
    pub cancel: Callback<CloseRunCompensationCancelInput>,
    pub manual_terminal: Callback<CloseRunManualTerminalInput>,
    recovery: RemedyRecovery,
    cancellation: RemedyRecovery,
}

impl CloseRunCompensationAction {
    pub(in crate::panels::modules::positions) fn locked(self) -> bool {
        self.recovery.journal.locked() || self.cancellation.journal.locked()
    }
    pub(in crate::panels::modules::positions) fn cancel_locked(self) -> bool {
        self.cancellation.journal.locked()
    }
    pub(in crate::panels::modules::positions) fn cancel_finished(self, id: &str) -> bool {
        self.cancellation.order_finished(id)
    }
    pub(in crate::panels::modules::positions) fn recovery_panel(self) -> impl IntoView {
        view! { {self.recovery.panel()} {self.cancellation.panel()} }
    }
}

pub(in crate::panels::modules::positions) fn use_close_run_compensation_action(
    snapshot: RwSignal<LoadState<PortfolioSnapshot>>,
) -> CloseRunCompensationAction {
    let recovery = RemedyRecovery::new("position-remedy", "补救 / 人工终结", snapshot);
    // An acknowledged compensation order must remain cancellable while awaiting fills.
    let cancellation = RemedyRecovery::new("position-remedy-cancel", "补救撤单", snapshot);
    let state = recovery.state;
    let active_key = RwSignal::new(None);
    Effect::new(move |_| {
        if !recovery.journal.busy.get() {
            active_key.set(None);
        }
    });
    let submit = Callback::new(move |input: CloseRunCompensationInput| {
        if recovery.journal.locked() || cancellation.journal.locked() {
            return;
        }
        let request = match close_run_compensation_request(&input) {
            Ok(request) => request,
            Err(problem) => {
                state.set(ActionState::failed("补救单未提交", *problem));
                return;
            }
        };
        let key = compensation_key(&input.run, input.candidate_index);
        let context =
            close_request_context("compensation", &key, Some(&input.run.snapshot_version));
        let Some(attempt) = recovery.begin(
            ActionRunKind::PortfolioCloseCompensation,
            input.run.id.clone(),
            context,
        ) else {
            return;
        };
        active_key.set(Some(key));
        let epoch = recovery.journal.epoch.get_untracked();
        let client = recovery.journal.client();
        spawn_local(async move {
            let result = with_mutation_timeout(
                "提交补救单",
                submit_close_run_compensation_task(
                    client,
                    &input.run.id,
                    request,
                    attempt.context.clone(),
                ),
            )
            .await;
            if !recovery.journal.current(epoch) {
                return;
            }
            match result {
                Ok(_) => recovery.lookup(attempt, epoch).await,
                Err(error) => recovery.failed(&attempt, error),
            }
        });
    });
    let cancel = Callback::new(move |input: CloseRunCompensationCancelInput| {
        if cancellation.journal.locked() {
            return;
        }
        let order_id = match close_run_compensation_cancel_order_id(&input) {
            Ok(id) => id,
            Err(problem) => {
                cancellation
                    .state
                    .set(ActionState::failed("补救撤单未提交", *problem));
                return;
            }
        };
        if cancellation.order_finished(&order_id) {
            return;
        }
        let key = cancel_compensation_key(&input.run, &order_id);
        let context = MutationRequestContext::new_idempotent_attempt(format!(
            "positions-compensation-cancel:{}:{key}",
            input.run.snapshot_version
        ));
        let Some(attempt) =
            cancellation.begin(ActionRunKind::TradingOrderCancel, order_id.clone(), context)
        else {
            return;
        };
        let epoch = cancellation.journal.epoch.get_untracked();
        let client = cancellation.journal.client();
        spawn_local(async move {
            let result = with_mutation_timeout(
                "撤销补救单",
                cancel_close_run_compensation_task(client, &order_id, attempt.context.clone()),
            )
            .await;
            if !cancellation.journal.current(epoch) {
                return;
            }
            match result {
                Ok(_) => cancellation.lookup(attempt, epoch).await,
                Err(error) => cancellation.failed(&attempt, error),
            }
        });
    });
    let manual_terminal = Callback::new(move |input: CloseRunManualTerminalInput| {
        if recovery.journal.locked() || cancellation.journal.locked() {
            return;
        }
        let request = match close_run_manual_terminal_request(&input) {
            Ok(request) => request,
            Err(problem) => {
                state.set(ActionState::failed("人工终结未提交", *problem));
                return;
            }
        };
        let context = close_request_context(
            "manual-terminal",
            &manual_terminal_key(&input.run),
            Some(&input.run.snapshot_version),
        );
        let Some(attempt) = recovery.begin(
            ActionRunKind::PortfolioCloseManualTerminal,
            input.run.id.clone(),
            context,
        ) else {
            return;
        };
        let epoch = recovery.journal.epoch.get_untracked();
        let client = recovery.journal.client();
        spawn_local(async move {
            let result = with_mutation_timeout(
                "记录人工终结",
                submit_close_run_manual_terminal_task(
                    client,
                    &input.run.id,
                    request,
                    attempt.context.clone(),
                ),
            )
            .await;
            if !recovery.journal.current(epoch) {
                return;
            }
            match result {
                Ok(_) => recovery.lookup(attempt, epoch).await,
                Err(error) => recovery.failed(&attempt, error),
            }
        });
    });
    CloseRunCompensationAction {
        state,
        cancel_state: cancellation.state,
        active_key,
        submit,
        cancel,
        manual_terminal,
        recovery,
        cancellation,
    }
}
