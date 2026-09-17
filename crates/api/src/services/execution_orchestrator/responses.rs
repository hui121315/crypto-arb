use super::events::{combine_errors, problem_message};
use super::legs::{records_by_role, recovery_action_for_role};
use super::run_model::{exact_fill_quantity, update_exposure, update_run};
use super::unwind::{mark_unwind_result, submit_unwind, unwind_skipped_problem, unwind_status};
use super::*;

mod outcome;
use outcome::*;

pub(super) async fn first_leg_partial_response(
    state: &AppState,
    idempotency_key: String,
    mut run: ExecutionRun,
    first_record: OrderRecord,
    first_role: HedgeLegRole,
) -> HedgeConfirmResponse {
    run.state = ExecutionRunState::FirstLegPartial;
    update_exposure(&mut run);
    if exact_fill_quantity(&first_record).is_none() {
        return partial_fill_manual_response(state, idempotency_key, run, first_record, first_role);
    }

    run.recovery_action = Some(recovery_action_for_role(first_role));
    update_run(
        state,
        &mut run,
        "第一腿部分成交，正在反向 unwind 已成交数量",
    );
    let unwind_quantity = exact_fill_quantity(&first_record);
    let unwind_result =
        submit_unwind(state, &first_record, &idempotency_key, &run, first_role).await;
    let (unwind_record, unwind_error) = mark_unwind_result(
        state,
        &mut run,
        unwind_result,
        first_role,
        "第一腿部分成交，已提交反向 unwind",
        "第一腿部分成交，反向 unwind 提交失败",
    );
    let status = unwind_status(
        HedgeConfirmStatus::FirstLegPartialUnwindAttempted,
        HedgeConfirmStatus::FirstLegPartialUnwindFailed,
        &unwind_error,
    );
    let partial_outcome = confirm_partial_outcome(
        &run,
        PartialOutcomeInput {
            cause: HedgeConfirmPartialCause::FirstLegPartial,
            status,
            primary_message: Some("第一腿部分成交".into()),
            primary_problem: None,
            unwind_status: unwind_outcome_status(&unwind_error),
            unwind_target_leg: first_role,
            unwind_quantity,
            unwind_problem: unwind_error.clone(),
        },
    );
    let (long_record, short_record) = records_by_role(first_role, Some(first_record), None);
    HedgeConfirmResponse {
        idempotency_key,
        status,
        context: shared_types::HedgeConfirmContext::default(),
        execution_run: Some(run),
        long_record,
        short_record,
        unwind_record,
        problem: unwind_error.clone(),
        partial_outcome: Some(partial_outcome),
        error: problem_message(&unwind_error),
    }
}

pub(super) fn first_leg_not_filled_response(
    state: &AppState,
    idempotency_key: String,
    mut run: ExecutionRun,
    first_record: OrderRecord,
    first_role: HedgeLegRole,
    cause: Option<ApiProblem>,
) -> HedgeConfirmResponse {
    run.state = ExecutionRunState::FailedSafe;
    run.recovery_action = (!matches!(
        first_record.state,
        LiveOrderState::Cancelled | LiveOrderState::Rejected | LiveOrderState::Failed
    ))
    .then_some(RecoveryAction::ManualReview);
    update_exposure(&mut run);
    let problem = cause.unwrap_or_else(|| {
        ApiProblem::new(
            codes::HEDGE_ORDER_FINALITY_FAILED,
            format!(
                "第一腿未确认完全成交，未提交第二腿：state={:?}",
                first_record.state
            ),
        )
        .with_status(409)
        .with_source("execution_orchestrator.first_leg_finality")
    });
    run.finality_problem = Some(problem.clone());
    run.finality_checked_at_ms = Some(common::time::now_ms());
    update_run(state, &mut run, &problem.message);
    let message = problem.message.clone();
    let records = records_by_role(first_role, Some(first_record), None);
    confirm_response(
        idempotency_key,
        HedgeConfirmStatus::LongLegFailed,
        run,
        records,
        Some(problem),
        Some(message),
    )
}

fn partial_fill_manual_response(
    state: &AppState,
    idempotency_key: String,
    mut run: ExecutionRun,
    first_record: OrderRecord,
    first_role: HedgeLegRole,
) -> HedgeConfirmResponse {
    run.recovery_action = Some(RecoveryAction::ManualReview);
    let problem = unwind_skipped_problem(&run, "第一腿部分成交但成交数量未知，未自动 unwind");
    run.unwind_problem = Some(problem.clone());
    update_run(
        state,
        &mut run,
        "第一腿部分成交，但缺少精确成交数量，需要人工复核",
    );
    let error = problem.message.clone();
    let partial_outcome = confirm_partial_outcome(
        &run,
        PartialOutcomeInput {
            cause: HedgeConfirmPartialCause::FirstLegPartial,
            status: HedgeConfirmStatus::FirstLegPartialWaitingFillQty,
            primary_message: Some(error.clone()),
            primary_problem: Some(problem.clone()),
            unwind_status: HedgeConfirmUnwindStatus::AwaitingFillQuantity,
            unwind_target_leg: first_role,
            unwind_quantity: None,
            unwind_problem: Some(problem.clone()),
        },
    );
    let (long_record, short_record) = records_by_role(first_role, Some(first_record), None);
    HedgeConfirmResponse {
        idempotency_key,
        status: HedgeConfirmStatus::FirstLegPartialWaitingFillQty,
        context: shared_types::HedgeConfirmContext::default(),
        execution_run: Some(run),
        long_record,
        short_record,
        unwind_record: None,
        problem: Some(problem),
        partial_outcome: Some(partial_outcome),
        error: Some(error),
    }
}

pub(super) async fn recheck_blocked_response(
    state: &AppState,
    idempotency_key: String,
    mut run: ExecutionRun,
    first_record: OrderRecord,
    first_role: HedgeLegRole,
    error: String,
) -> HedgeConfirmResponse {
    run.state = ExecutionRunState::UnwindRequired;
    run.recovery_action = Some(recovery_action_for_role(first_role));
    update_run(state, &mut run, "二次风控拒绝第二腿，需要反向处理第一腿");
    let unwind_result =
        submit_unwind(state, &first_record, &idempotency_key, &run, first_role).await;
    let (unwind_record, unwind_error) = mark_unwind_result(
        state,
        &mut run,
        unwind_result,
        first_role,
        "二次风控后已提交第一腿反向 unwind",
        "二次风控后第一腿反向 unwind 提交失败",
    );
    let status = unwind_status(
        HedgeConfirmStatus::HedgeRecheckBlockedUnwindAttempted,
        HedgeConfirmStatus::HedgeRecheckBlockedUnwindFailed,
        &unwind_error,
    );
    let partial_outcome = confirm_partial_outcome(
        &run,
        PartialOutcomeInput {
            cause: HedgeConfirmPartialCause::HedgeRecheckBlocked,
            status,
            primary_message: Some(error.clone()),
            primary_problem: None,
            unwind_status: unwind_outcome_status(&unwind_error),
            unwind_target_leg: first_role,
            unwind_quantity: unwind_intended_quantity(&first_record),
            unwind_problem: unwind_error.clone(),
        },
    );
    let (long_record, short_record) = records_by_role(first_role, Some(first_record), None);
    HedgeConfirmResponse {
        idempotency_key,
        status,
        context: shared_types::HedgeConfirmContext::default(),
        execution_run: Some(run),
        long_record,
        short_record,
        unwind_record,
        problem: unwind_error.clone(),
        partial_outcome: Some(partial_outcome),
        error: combine_errors(Some(error), problem_message(&unwind_error)),
    }
}

pub(super) async fn second_leg_failed_response(
    state: &AppState,
    idempotency_key: String,
    mut run: ExecutionRun,
    first_record: OrderRecord,
    first_role: HedgeLegRole,
    error: ApiProblem,
) -> HedgeConfirmResponse {
    run.state = ExecutionRunState::UnwindRequired;
    run.recovery_action = Some(recovery_action_for_role(first_role));
    update_exposure(&mut run);
    update_run(state, &mut run, "第二腿失败，需要反向处理第一腿");
    let unwind_result =
        submit_unwind(state, &first_record, &idempotency_key, &run, first_role).await;
    let (unwind_record, unwind_error) = mark_unwind_result(
        state,
        &mut run,
        unwind_result,
        first_role,
        "已提交第一腿反向 unwind",
        "第二腿失败后第一腿反向 unwind 提交失败",
    );
    let status = unwind_status(
        HedgeConfirmStatus::HedgeBrokenUnwindAttempted,
        HedgeConfirmStatus::HedgeBrokenUnwindFailed,
        &unwind_error,
    );
    let partial_outcome = confirm_partial_outcome(
        &run,
        PartialOutcomeInput {
            cause: HedgeConfirmPartialCause::HedgeBroken,
            status,
            primary_message: Some(error.message.clone()),
            primary_problem: Some(error.clone()),
            unwind_status: unwind_outcome_status(&unwind_error),
            unwind_target_leg: first_role,
            unwind_quantity: unwind_intended_quantity(&first_record),
            unwind_problem: unwind_error.clone(),
        },
    );
    let problem = unwind_error.clone().or_else(|| Some(error.clone()));
    let (long_record, short_record) = records_by_role(first_role, Some(first_record), None);
    HedgeConfirmResponse {
        idempotency_key,
        status,
        context: shared_types::HedgeConfirmContext::default(),
        execution_run: Some(run),
        long_record,
        short_record,
        unwind_record,
        problem,
        partial_outcome: Some(partial_outcome),
        error: combine_errors(Some(error.message), problem_message(&unwind_error)),
    }
}
