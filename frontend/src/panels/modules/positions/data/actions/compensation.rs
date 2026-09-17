//! Close-run compensation, compensation cancellation, and manual-terminal actions.

use crate::api::rest::MutationRequestContext;
use crate::state::action_state::ActionState;
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{ActionEvidence, ActionRunKind, CloseRun, PortfolioSnapshot};

use super::super::requests::*;
use super::super::runs::*;
use super::close_request_context;

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
    pub active_key: RwSignal<Option<String>>,
    pub submit: Callback<CloseRunCompensationInput>,
    pub cancel: Callback<CloseRunCompensationCancelInput>,
    pub manual_terminal: Callback<CloseRunManualTerminalInput>,
}

pub(in crate::panels::modules::positions) fn use_close_run_compensation_action(
    snapshot_state: RwSignal<LoadState<PortfolioSnapshot>>,
) -> CloseRunCompensationAction {
    let client = use_global().client;
    let cancel_client = client.clone();
    let manual_client = client.clone();
    let state = RwSignal::new(ActionState::Idle);
    let active_key = RwSignal::new(None::<String>);
    recover_compensation_state(state, snapshot_state);
    let submit = Callback::new(move |input: CloseRunCompensationInput| {
        if state.get_untracked().is_pending() {
            return;
        }
        let request = match close_run_compensation_request(&input) {
            Ok(request) => request,
            Err(problem) => {
                state.set(ActionState::failed("补偿单失败", *problem));
                return;
            }
        };
        let key = compensation_key(&input.run, input.candidate_index);
        let context = close_request_context(
            "compensation",
            &key,
            Some(input.run.snapshot_version.as_str()),
        );
        let pending_evidence = context
            .evidence()
            .merged(ActionEvidence::from_close_run(&input.run))
            .with_action_kind(ActionRunKind::PortfolioCloseCompensation);
        state.set(ActionState::pending("正在提交补偿单").with_evidence(pending_evidence.clone()));
        active_key.set(Some(key));
        let client = client.clone();
        spawn_local(async move {
            let result =
                submit_close_run_compensation_task(client, &input.run.id, request, context).await;
            active_key.set(None);
            match result {
                Ok(run) => apply_close_run_result(state, "补偿单", &run, pending_evidence),
                Err(error) => state.set(
                    ActionState::failed("补偿单失败", error.problem)
                        .with_evidence(pending_evidence),
                ),
            }
        });
    });
    let cancel = Callback::new(move |input: CloseRunCompensationCancelInput| {
        if state.get_untracked().is_pending() {
            return;
        }
        let order_id = match close_run_compensation_cancel_order_id(&input) {
            Ok(order_id) => order_id,
            Err(problem) => {
                state.set(ActionState::failed("补偿撤单失败", *problem));
                return;
            }
        };
        let key = cancel_compensation_key(&input.run, &order_id);
        let context = MutationRequestContext::new_idempotent_attempt(format!(
            "positions-compensation-cancel:{}:{key}",
            input.run.snapshot_version
        ));
        let pending_evidence = context
            .evidence()
            .merged(ActionEvidence::from_close_run(&input.run))
            .with_action_kind(ActionRunKind::TradingOrderCancel);
        state.set(ActionState::pending("正在撤销补偿单").with_evidence(pending_evidence.clone()));
        active_key.set(Some(key));
        let client = cancel_client.clone();
        spawn_local(async move {
            let result = cancel_close_run_compensation_task(client, &order_id, context).await;
            active_key.set(None);
            match result {
                Ok(order) => apply_cancel_order_result(state, &order, pending_evidence),
                Err(error) => state.set(
                    ActionState::failed("补偿撤单失败", error.problem)
                        .with_evidence(pending_evidence),
                ),
            }
        });
    });
    let manual_terminal = Callback::new(move |input: CloseRunManualTerminalInput| {
        if state.get_untracked().is_pending() {
            return;
        }
        let request = match close_run_manual_terminal_request(&input) {
            Ok(request) => request,
            Err(problem) => {
                state.set(ActionState::failed("人工终结失败", *problem));
                return;
            }
        };
        let key = manual_terminal_key(&input.run);
        let context = close_request_context(
            "manual-terminal",
            &key,
            Some(input.run.snapshot_version.as_str()),
        );
        let pending_evidence = context
            .evidence()
            .merged(ActionEvidence::from_close_run(&input.run))
            .with_action_kind(ActionRunKind::PortfolioCloseManualTerminal);
        state.set(ActionState::pending("正在记录人工终结").with_evidence(pending_evidence.clone()));
        active_key.set(Some(key));
        let client = manual_client.clone();
        spawn_local(async move {
            let result =
                submit_close_run_manual_terminal_task(client, &input.run.id, request, context)
                    .await;
            active_key.set(None);
            match result {
                Ok(run) => apply_close_run_result(state, "人工终结", &run, pending_evidence),
                Err(error) => state.set(
                    ActionState::failed("人工终结失败", error.problem)
                        .with_evidence(pending_evidence),
                ),
            }
        });
    });
    CloseRunCompensationAction {
        state,
        active_key,
        submit,
        cancel,
        manual_terminal,
    }
}
