use super::*;

pub(super) fn auto_compensation_decision(
    run: &CloseRun,
    mode: ExecutionMode,
) -> AutoCompensationDecision {
    let Some(plan) = run.unwind_plan.as_ref() else {
        return AutoCompensationDecision::Skip(AutoCompensationSkip::NotEligible);
    };
    if !auto_compensation_state_eligible(run, plan) {
        return AutoCompensationDecision::Skip(AutoCompensationSkip::NotEligible);
    }
    if plan.compensation_candidates.len() != 1 {
        return AutoCompensationDecision::Skip(AutoCompensationSkip::MultipleCandidates);
    }
    if mode == ExecutionMode::Live {
        return AutoCompensationDecision::Skip(AutoCompensationSkip::LiveMode);
    }
    let candidate = &plan.compensation_candidates[0];
    let target = candidate
        .confirmed_quantity
        .unwrap_or(candidate.target_quantity)
        .abs();
    if !target.is_finite()
        || target <= 0.0
        || !candidate.mark_price.is_finite()
        || candidate.mark_price <= 0.0
        || candidate.compensation_order_side.is_none()
    {
        return AutoCompensationDecision::Skip(AutoCompensationSkip::InvalidCandidate);
    }
    let attempt_index = plan
        .compensation_attempts
        .iter()
        .filter(|attempt| compensation_attempt_matches_candidate(attempt, candidate))
        .count();
    AutoCompensationDecision::Submit(AutoCompensationSubmit {
        close_run_id: run.id.clone(),
        request: CloseRunCompensationRequest {
            confirmation_phrase: CLOSE_RUN_COMPENSATION_CONFIRMATION_PHRASE.to_owned(),
            snapshot_version: Some(run.snapshot_version.clone()),
            candidate_index: Some(0),
            target_quantity: Some(target),
            limit_price: Some(candidate.mark_price),
            reason: Some(auto_compensation_reason(attempt_index).to_owned()),
        },
        attempt_index,
    })
}

pub(super) fn auto_compensation_state_eligible(run: &CloseRun, plan: &CloseRunUnwindPlan) -> bool {
    initial_compensation_submit_allowed(run, plan)
        || (run.status == CloseRunStatus::CompensationFailed
            && plan.status == CloseRunUnwindPlanStatus::CompensationFailed
            && plan.compensation_candidates.len() == 1
            && plan
                .compensation_candidates
                .first()
                .is_some_and(|candidate| {
                    compensation_retry_allowed_for_candidate(candidate, &plan.compensation_attempts)
                }))
}

pub(super) fn auto_compensation_reason(attempt_index: usize) -> &'static str {
    if attempt_index == 0 {
        AUTO_COMPENSATION_REASON
    } else {
        AUTO_COMPENSATION_RETRY_REASON
    }
}
