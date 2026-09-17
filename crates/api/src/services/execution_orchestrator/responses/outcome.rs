use super::*;

pub(super) struct PartialOutcomeInput {
    pub(super) cause: HedgeConfirmPartialCause,
    pub(super) status: HedgeConfirmStatus,
    pub(super) primary_message: Option<String>,
    pub(super) primary_problem: Option<ApiProblem>,
    pub(super) unwind_status: HedgeConfirmUnwindStatus,
    pub(super) unwind_target_leg: HedgeLegRole,
    pub(super) unwind_quantity: Option<f64>,
    pub(super) unwind_problem: Option<ApiProblem>,
}

pub(super) fn confirm_partial_outcome(
    run: &ExecutionRun,
    input: PartialOutcomeInput,
) -> HedgeConfirmPartialOutcome {
    let manual_review_required = matches!(run.recovery_action, Some(RecoveryAction::ManualReview))
        || input.unwind_status != HedgeConfirmUnwindStatus::Submitted
        || input.unwind_problem.is_some();
    HedgeConfirmPartialOutcome {
        context: shared_types::HedgeConfirmContext::default(),
        cause: input.cause,
        original_status: input.status,
        run_id: run.run_id.clone(),
        run_state: run.state,
        net_exposure_usd: run.net_exposure_usd,
        recovery_action: run.recovery_action,
        primary_message: input.primary_message,
        primary_problem: input.primary_problem,
        unwind_status: input.unwind_status,
        unwind_target_leg: Some(input.unwind_target_leg),
        unwind_quantity: input.unwind_quantity,
        unwind_problem: input.unwind_problem,
        manual_review_required,
    }
}

pub(super) fn unwind_outcome_status(error: &Option<ApiProblem>) -> HedgeConfirmUnwindStatus {
    if error.is_some() {
        HedgeConfirmUnwindStatus::SubmitFailed
    } else {
        HedgeConfirmUnwindStatus::Submitted
    }
}

pub(super) fn unwind_intended_quantity(record: &OrderRecord) -> Option<f64> {
    record
        .filled_quantity
        .filter(|quantity| quantity.is_finite() && *quantity > 0.0)
        .or_else(|| {
            let quantity = record.intent.quantity;
            (quantity.is_finite() && quantity > 0.0).then_some(quantity)
        })
}
