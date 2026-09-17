use crate::api::rest::{with_mutation_timeout, ApiClient, ApiError, MutationRequestContext};
use crate::state::action_state::{
    action_state_from_execution_run, merge_order_evidence, ActionState,
};
use crate::state::context::use_global;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{
    ExecutionEnvironment, ExecutionRun, HedgeConfirmContext,
    HedgeConfirmRequest as HedgeConfirmApiRequest, HedgeConfirmResponse, OrderRecord,
};

use super::preview::ExecutionPreview;
use super::run::{
    restored_execution_run_evidence, restored_execution_run_matches, store_confirm_request_context,
    store_execution_run_context,
};

#[path = "actions/outcome.rs"]
mod outcome;
use super::runtime::ConfirmActionRuntime;
use outcome::{
    attach_confirm_context_decode_problem, confirm_context_from_problem, confirm_outcome_label,
    confirm_response_evidence, confirm_response_problem, resolved_confirm_context,
    submit_failed_label, submitting_label,
};

type ConfirmResult = Result<HedgeConfirmResponse, ApiError>;

pub(in crate::panels::modules::execution) struct ConfirmHedgeRequest {
    pub seed: ConfirmHedgeSeed,
    pub mode_label: &'static str,
    pub client_order_ids: Vec<String>,
}

pub(in crate::panels::modules::execution) struct ConfirmHedgeSeed {
    pub opportunity_id: String,
    pub request: HedgeConfirmApiRequest,
    pub context: HedgeConfirmContext,
}

impl ConfirmHedgeSeed {
    pub(in crate::panels::modules::execution) fn new(
        opportunity_id: String,
        idempotency_key: String,
        ticket_id: Option<String>,
        environment: ExecutionEnvironment,
        long_venue: Option<String>,
        short_venue: Option<String>,
    ) -> Self {
        let context = HedgeConfirmContext {
            opportunity_id: opportunity_id.clone(),
            idempotency_key: idempotency_key.clone(),
            ticket_id: ticket_id.clone(),
            environment: Some(environment),
            long_venue,
            short_venue,
            ..HedgeConfirmContext::default()
        };
        Self {
            opportunity_id,
            request: HedgeConfirmApiRequest {
                idempotency_key,
                ticket_id,
            },
            context,
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::panels::modules::execution) struct ConfirmHedgeAction {
    pub state: RwSignal<ActionState>,
    pub submit: Callback<ConfirmHedgeRequest>,
    pub last_outcome: RwSignal<Option<HedgeConfirmResponse>>,
    pub context: RwSignal<Option<HedgeConfirmContext>>,
}

pub(crate) fn use_confirm_hedge_action(
    preview: Memo<ExecutionPreview>,
    execution_run: RwSignal<Option<ExecutionRun>>,
    orders: Memo<Vec<OrderRecord>>,
    refresh_nonce: RwSignal<u64>,
    runtime: ConfirmActionRuntime,
) -> ConfirmHedgeAction {
    let client = use_global().client;
    let state = runtime.state;
    let last_outcome = runtime.last_outcome;
    let context = runtime.context;
    Effect::new(move |_| {
        let run = execution_run.get();
        let orders = orders.get();
        if !allows_execution_run_restore(&state.get_untracked()) {
            return;
        }
        let Some(run) = run else {
            return;
        };
        if !run_matches_action_scope(&run, &preview.get()) {
            return;
        }
        if let Some(recovered) = action_state_from_execution_run(&run) {
            let evidence = merge_order_evidence(restored_execution_run_evidence(&run), &orders);
            state.set(recovered.with_evidence(evidence));
        }
    });
    let submit = Callback::new(move |request: ConfirmHedgeRequest| {
        if state.get_untracked().is_pending() {
            return;
        }
        let request_transport = MutationRequestContext::with_idempotency_key(
            request.seed.request.idempotency_key.clone(),
        );
        let pending_evidence = request_transport
            .evidence()
            .with_ticket_id(request.seed.request.ticket_id.clone())
            .with_client_order_ids(request.client_order_ids);
        state.set(
            ActionState::pending(submitting_label(request.mode_label))
                .with_evidence(pending_evidence.clone()),
        );
        context.set(Some(request.seed.context.clone()));
        store_confirm_request_context(&request.seed.context);
        let client = client.clone();
        spawn_local(async move {
            let mode_label = request.mode_label;
            let request_context = request.seed.context.clone();
            let idempotency_key = request.seed.request.idempotency_key.clone();
            let result = confirm_hedge_task(client, request.seed, request_transport).await;
            match result {
                Ok(response) => {
                    let response_context = resolved_confirm_context(&response, &request_context);
                    context.set(Some(response_context));
                    last_outcome.set(Some(response.clone()));
                    if let Some(run) = response.execution_run.clone() {
                        store_execution_run_context(&run, &idempotency_key);
                        execution_run.set(Some(run));
                    }
                    refresh_nonce.update(|value| *value = value.wrapping_add(1));
                    let outcome_evidence = confirm_response_evidence(&response, pending_evidence);
                    if let Some(problem) = confirm_response_problem(&response) {
                        state.set(
                            ActionState::failed(
                                confirm_outcome_label(&response, mode_label),
                                problem,
                            )
                            .with_evidence(outcome_evidence),
                        );
                        return;
                    }
                    let label = confirm_outcome_label(&response, mode_label);
                    state.set(ActionState::succeeded(label).with_evidence(outcome_evidence));
                }
                Err(error) => {
                    last_outcome.set(None);
                    let mut problem = error.problem;
                    let recovered_context = match confirm_context_from_problem(&problem) {
                        Ok(Some(context)) => context,
                        Ok(None) => request_context,
                        Err(decode_error) => {
                            attach_confirm_context_decode_problem(&mut problem, &decode_error);
                            request_context
                        }
                    };
                    context.set(Some(recovered_context));
                    refresh_nonce.update(|value| *value = value.wrapping_add(1));
                    state.set(
                        ActionState::failed(submit_failed_label(mode_label), problem)
                            .with_evidence(pending_evidence),
                    );
                }
            }
        });
    });
    ConfirmHedgeAction {
        state,
        submit,
        last_outcome,
        context,
    }
}

fn run_matches_action_scope(run: &ExecutionRun, preview: &ExecutionPreview) -> bool {
    explicit_preview_run_match(run, preview.ticket_id.as_deref(), &preview.opportunity_id)
        .unwrap_or_else(|| restored_execution_run_matches(run))
}

fn explicit_preview_run_match(
    run: &ExecutionRun,
    preview_ticket_id: Option<&str>,
    preview_opportunity_id: &str,
) -> Option<bool> {
    preview_ticket_id
        .map(|ticket_id| ticket_id == run.ticket_id && preview_opportunity_id == run.opportunity_id)
}

const fn allows_execution_run_restore(state: &ActionState) -> bool {
    matches!(
        state,
        ActionState::Idle | ActionState::Accepted { .. } | ActionState::Succeeded { .. }
    )
}

async fn confirm_hedge_task(
    client: ApiClient,
    seed: ConfirmHedgeSeed,
    request_context: MutationRequestContext,
) -> ConfirmResult {
    with_mutation_timeout(
        "对冲提交",
        client.confirm_hedge_with_context(&seed.opportunity_id, &seed.request, &request_context),
    )
    .await
}

#[cfg(test)]
#[path = "actions_tests.rs"]
mod tests;
