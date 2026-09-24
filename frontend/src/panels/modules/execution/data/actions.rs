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
    apply_run_update, clear_execution_run_context, restored_execution_run_evidence,
    restored_execution_run_matches, store_confirm_request_context, store_execution_run_context,
};
use super::submission::{request_matches_run, SubmissionRecovery};

#[path = "actions/outcome.rs"]
pub(super) mod outcome;
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
    pub recovery: SubmissionRecovery,
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
    let recovery = runtime.recovery;
    Effect::new(move |_| {
        let run = execution_run.get();
        let orders = orders.get();
        let pending = recovery.pending.get();
        let submitted_context = context.get();
        let Some(run) = run else {
            return;
        };
        let matches_request = pending
            .as_ref()
            .or(submitted_context.as_ref())
            .is_some_and(|request| request_matches_run(request, &run));
        if !matches_request
            && (!allows_execution_run_restore(&state.get_untracked())
                || !run_matches_action_scope(&run, &preview.get()))
        {
            return;
        }
        if let Some(recovered) = action_state_from_execution_run(&run) {
            if let Some(request) = pending
                .as_ref()
                .filter(|request| request_matches_run(request, &run))
            {
                store_execution_run_context(&run, &request.idempotency_key);
                recovery.resolve_run(&run);
            }
            let evidence = merge_order_evidence(restored_execution_run_evidence(&run), &orders);
            state.set(recovered.with_evidence(evidence));
        }
    });
    let submit = Callback::new(move |request: ConfirmHedgeRequest| {
        if state.get_untracked().is_pending()
            || !recovery.begin(&request.seed.context, &client.base_url())
        {
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
            let result = confirm_hedge_task(client.clone(), request.seed, request_transport).await;
            if recovery.sending.try_get_untracked().is_none() {
                return;
            }
            recovery.sending.set(false);
            if !recovery.matches_backend(&client.base_url())
                || !context
                    .get_untracked()
                    .as_ref()
                    .is_some_and(|current| current.idempotency_key == idempotency_key)
            {
                return;
            }
            match result {
                Ok(mut response) => {
                    let response_context = resolved_confirm_context(&response, &request_context);
                    if !outcome::response_matches_request(
                        &response,
                        &response_context,
                        &request_context,
                    ) {
                        state.set(
                            ActionState::failed(
                                "提交结果待核验",
                                shared_types::ApiProblem::new(
                                    "HEDGE_CONFIRM_IDENTITY_MISMATCH",
                                    "回执与原提交不一致；只查询原请求，不重复下单",
                                ),
                            )
                            .with_evidence(pending_evidence),
                        );
                        refresh_nonce.update(|value| *value = value.wrapping_add(1));
                        return;
                    }
                    context.set(Some(response_context));
                    if let Some(run) = response.execution_run.clone() {
                        store_execution_run_context(&run, &idempotency_key);
                        apply_run_update(execution_run, run.clone(), false);
                        recovery.resolve_run(&run);
                    }
                    if let Some(run) = execution_run
                        .get_untracked()
                        .filter(|run| request_matches_run(&request_context, run))
                    {
                        response.execution_run = Some(run.clone());
                        recovery.resolve_run(&run);
                    }
                    last_outcome.set(Some(response.clone()));
                    refresh_nonce.update(|value| *value = value.wrapping_add(1));
                    let outcome_evidence = confirm_response_evidence(&response, pending_evidence);
                    if let Some(recovered) = response
                        .execution_run
                        .as_ref()
                        .and_then(action_state_from_execution_run)
                    {
                        state.set(recovered.with_evidence(outcome_evidence));
                        return;
                    }
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
                    state.set(ActionState::accepted(label).with_evidence(outcome_evidence));
                }
                Err(error) => {
                    last_outcome.set(None);
                    let mut problem = error.problem;
                    let recovered_context = match confirm_context_from_problem(&problem) {
                        Ok(Some(context)) if outcome::same_request(&context, &request_context) => {
                            context
                        }
                        Ok(_) => request_context.clone(),
                        Err(decode_error) => {
                            attach_confirm_context_decode_problem(&mut problem, &decode_error);
                            request_context.clone()
                        }
                    };
                    context.set(Some(recovered_context));
                    refresh_nonce.update(|value| *value = value.wrapping_add(1));
                    if let Some(run) = execution_run
                        .get_untracked()
                        .filter(|run| request_matches_run(&request_context, run))
                    {
                        if let Some(recovered) = action_state_from_execution_run(&run) {
                            recovery.resolve_run(&run);
                            state.set(
                                recovered.with_evidence(restored_execution_run_evidence(&run)),
                            );
                            return;
                        }
                    }
                    let rejected = outcome::confirm_rejected_before_order(&problem);
                    if rejected {
                        clear_execution_run_context();
                        recovery.resolve(&idempotency_key);
                    }
                    state.set(
                        ActionState::failed(
                            if rejected {
                                submit_failed_label(mode_label)
                            } else {
                                "提交结果待核验 · 不重复下单"
                            },
                            problem,
                        )
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
        recovery,
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
