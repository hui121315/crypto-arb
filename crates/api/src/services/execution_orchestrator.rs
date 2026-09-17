use crate::{
    services::execution_valuation::{self, refresh_cost_reconciliation},
    services::hedge_ticket::HedgeExecutionOrder,
    state::AppState,
};
use shared_types::{
    problem::codes, ApiProblem, ExecutionCostReconciliation, ExecutionRun, ExecutionRunLeg,
    ExecutionRunState, HedgeConfirmPartialCause, HedgeConfirmPartialOutcome, HedgeConfirmResponse,
    HedgeConfirmStatus, HedgeConfirmUnwindStatus, HedgeLegRole, HedgePreviewResponse,
    LiveOrderState, OrderIntent, OrderRecord, OrderSide, OrderType, OrderUpdateSource,
    RecoveryAction, VenueOrderIdentity,
};
use trading::ExecutionLedgerOrderContext;

pub(crate) async fn confirm_preview(
    state: &AppState,
    preview: HedgePreviewResponse,
    idempotency_key: String,
) -> HedgeConfirmResponse {
    let execution_order = crate::services::hedge_ticket::execution_order(&preview.ticket);
    let context = context::from_preview(&preview, &idempotency_key);
    let response = confirm_preview_inner(state, preview, idempotency_key, execution_order).await;
    context::attach_response(response, context, execution_order.first)
}

async fn confirm_preview_inner(
    state: &AppState,
    preview: HedgePreviewResponse,
    idempotency_key: String,
    execution_order: HedgeExecutionOrder,
) -> HedgeConfirmResponse {
    let mut run = start_run(state, &preview, &idempotency_key, execution_order);
    if let Some(problem) = run.valuation_problem.clone() {
        return valuation_failed_response(state, idempotency_key, run, None, None, problem);
    }

    let first_record =
        match submit_first_leg(state, &preview, &mut run, execution_order.first).await {
            Ok(record) => record,
            Err(error) => {
                let message = error.message.clone();
                return confirm_response(
                    idempotency_key,
                    HedgeConfirmStatus::LongLegFailed,
                    run,
                    (None, None),
                    Some(error),
                    Some(message),
                );
            }
        };

    let first_record = match settle_first_leg(state, first_record).await {
        Ok(record) => record,
        Err((record, problem)) => {
            let _ = apply_leg_record(run_leg_mut(&mut run, execution_order.first), &record);
            crate::services::execution_runs::append_order_update_evidence(&mut run, &record);
            return first_leg_not_filled_response(
                state,
                idempotency_key,
                run,
                record,
                execution_order.first,
                Some(problem),
            );
        }
    };

    if let Some(problem) =
        apply_leg_record(run_leg_mut(&mut run, execution_order.first), &first_record)
    {
        let (long_record, short_record) =
            records_by_role(execution_order.first, Some(first_record), None);
        return valuation_failed_response(
            state,
            idempotency_key,
            run,
            long_record,
            short_record,
            problem,
        );
    }
    crate::services::execution_runs::append_order_update_evidence(&mut run, &first_record);
    refresh_cost_reconciliation(&mut run);
    // This branch owns the large ticket/run payload once per confirmed hedge.
    // Boxing it keeps the async state machine off the task stack.
    Box::pin(continue_after_first_leg(
        state,
        preview,
        idempotency_key,
        run,
        first_record,
        execution_order,
    ))
    .await
}

async fn continue_after_first_leg(
    state: &AppState,
    mut preview: HedgePreviewResponse,
    idempotency_key: String,
    mut run: ExecutionRun,
    first_record: OrderRecord,
    execution_order: HedgeExecutionOrder,
) -> HedgeConfirmResponse {
    if first_record.state == LiveOrderState::PartiallyFilled {
        return first_leg_partial_response(
            state,
            idempotency_key,
            run,
            first_record,
            execution_order.first,
        )
        .await;
    }
    if first_record.state != LiveOrderState::Filled {
        return first_leg_not_filled_response(
            state,
            idempotency_key,
            run,
            first_record,
            execution_order.first,
            None,
        );
    }

    prepare_second_leg(state, &mut run);
    if let Some(error) = crate::services::hedge_recheck::before_second_leg_rejection(
        state,
        &mut preview,
        &first_record,
        execution_order.first,
    )
    .await
    {
        return recheck_blocked_response(
            state,
            idempotency_key,
            run,
            first_record,
            execution_order.first,
            error,
        )
        .await;
    }
    if let Some(problem) = refresh_second_leg_plan(&mut run, &preview, execution_order.second) {
        return recheck_blocked_response(
            state,
            idempotency_key,
            run,
            first_record,
            execution_order.first,
            problem.message,
        )
        .await;
    }

    update_run(state, &mut run, "提交第二腿");
    submit_second_leg(
        state,
        &preview,
        idempotency_key,
        run,
        first_record,
        execution_order,
    )
    .await
}

fn start_run(
    state: &AppState,
    preview: &HedgePreviewResponse,
    idempotency_key: &str,
    execution_order: HedgeExecutionOrder,
) -> ExecutionRun {
    let mut run = new_run(preview, idempotency_key);
    run = crate::services::execution_runs::record(state, run);
    run.state = ExecutionRunState::SubmittingFirstLeg;
    update_run(
        state,
        &mut run,
        &format!(
            "提交第一腿（{}，第二腿 {}）",
            crate::services::hedge_ticket::role_label(execution_order.first),
            crate::services::hedge_ticket::role_label(execution_order.second),
        ),
    );
    run
}

fn prepare_second_leg(state: &AppState, run: &mut ExecutionRun) {
    run.state = ExecutionRunState::SubmittingSecondLeg;
    update_exposure(run);
    update_run(state, run, "二次风控复核");
}

mod context;
mod events;
mod evidence;
mod leg_submission;
mod legs;
mod responses;
mod run_model;
mod submission;
#[cfg(test)]
mod tests;
mod unwind;

use events::{confirm_response, settle_first_leg, valuation_failed_response};
use leg_submission::{submit_first_leg, submit_second_leg};
use legs::{records_by_role, run_leg_mut};
use responses::{
    first_leg_not_filled_response, first_leg_partial_response, recheck_blocked_response,
};
use run_model::{apply_leg_record, new_run, refresh_second_leg_plan, update_exposure, update_run};
