use super::*;

mod orders;
mod problems;

use orders::*;
pub(super) use problems::*;

pub(super) const MAX_COMPENSATION_ATTEMPTS_PER_CANDIDATE: usize = 2;

pub(super) fn close_run_unwind_plan(
    legs: &[CloseLeg],
    previous: Option<&CloseRunUnwindPlan>,
) -> Option<CloseRunUnwindPlan> {
    let filled_legs = unwind_leg_evidence(legs, close_leg_has_fill, true);
    let failed_legs = unwind_leg_evidence(legs, |leg| close_leg_failed(leg.status), false);
    let compensation_attempts = previous
        .map(|plan| plan.compensation_attempts.clone())
        .unwrap_or_default();
    let manual_terminal_evidence = previous.and_then(|plan| plan.manual_terminal_evidence.clone());
    if compensation_attempts.is_empty() && (filled_legs.is_empty() || failed_legs.is_empty()) {
        return None;
    }
    let status = if manual_terminal_evidence.is_some() {
        CloseRunUnwindPlanStatus::ManualTerminalRecorded
    } else {
        compensation_plan_status(&filled_legs, &compensation_attempts)
    };
    let required_evidence = unwind_required_evidence();
    let remaining_positions =
        remaining_position_evidence(status, &failed_legs, manual_terminal_evidence.as_ref());
    let next_actions = close_run_next_actions(
        status,
        &filled_legs,
        &compensation_attempts,
        &required_evidence,
    );
    Some(CloseRunUnwindPlan {
        status,
        compensation_candidates: filled_legs.clone(),
        filled_legs,
        failed_legs,
        remaining_positions,
        compensation_attempts,
        manual_terminal_evidence,
        next_actions,
        required_evidence,
    })
}

pub(super) fn compensation_plan_status(
    candidates: &[CloseRunUnwindLegEvidence],
    attempts: &[CloseRunCompensationAttempt],
) -> CloseRunUnwindPlanStatus {
    if attempts.is_empty() {
        return CloseRunUnwindPlanStatus::BlockedPendingManualRecheck;
    }
    if attempts.iter().any(compensation_attempt_active) {
        return CloseRunUnwindPlanStatus::CompensationSubmitted;
    }
    if compensation_candidates_compensated(candidates, attempts) {
        return CloseRunUnwindPlanStatus::Compensated;
    }
    if attempts.iter().any(compensation_attempt_failed) {
        return CloseRunUnwindPlanStatus::CompensationFailed;
    }
    CloseRunUnwindPlanStatus::CompensationSubmitted
}

pub(super) fn compensation_candidates_compensated(
    candidates: &[CloseRunUnwindLegEvidence],
    attempts: &[CloseRunCompensationAttempt],
) -> bool {
    if candidates.is_empty() {
        return attempts.iter().all(compensation_attempt_filled);
    }
    candidates.iter().all(|candidate| {
        attempts.iter().any(|attempt| {
            compensation_attempt_filled(attempt)
                && compensation_attempt_matches_candidate(attempt, candidate)
        })
    })
}

pub(super) fn compensation_attempt_active(attempt: &CloseRunCompensationAttempt) -> bool {
    !compensation_attempt_failed(attempt) && !compensation_attempt_filled(attempt)
}

pub(super) fn compensation_attempt_failed(attempt: &CloseRunCompensationAttempt) -> bool {
    close_leg_failed(attempt.status)
}

pub(super) fn compensation_attempt_filled(attempt: &CloseRunCompensationAttempt) -> bool {
    attempt.status == CloseLegStatus::Filled
        && attempt.order.as_ref().is_some_and(has_fill_evidence)
}

pub(super) fn compensation_attempt_matches_candidate(
    attempt: &CloseRunCompensationAttempt,
    candidate: &CloseRunUnwindLegEvidence,
) -> bool {
    let Some(compensation_order_side) = candidate.compensation_order_side else {
        return false;
    };
    venue_names_equal(&attempt.venue, &candidate.venue)
        && attempt.symbol.eq_ignore_ascii_case(&candidate.symbol)
        && attempt.side == candidate.side
        && attempt.compensation_order_side == compensation_order_side
        && compensation_attempt_quantity_matches(attempt, candidate)
}

pub(super) fn compensation_attempt_quantity_matches(
    attempt: &CloseRunCompensationAttempt,
    candidate: &CloseRunUnwindLegEvidence,
) -> bool {
    let target = candidate
        .confirmed_quantity
        .unwrap_or(candidate.target_quantity)
        .abs();
    if !target.is_finite() || target <= 0.0 {
        return false;
    }
    let tolerance = (target.abs() * 1e-9).max(1e-9);
    (attempt.target_quantity - target).abs() <= tolerance
}

pub(super) fn compensation_retry_allowed_for_candidate(
    candidate: &CloseRunUnwindLegEvidence,
    attempts: &[CloseRunCompensationAttempt],
) -> bool {
    let mut matched_count = 0usize;
    let mut all_matched_failed = true;
    for attempt in attempts
        .iter()
        .filter(|attempt| compensation_attempt_matches_candidate(attempt, candidate))
    {
        matched_count += 1;
        all_matched_failed &= compensation_attempt_failed(attempt);
    }
    matched_count > 0
        && all_matched_failed
        && matched_count < MAX_COMPENSATION_ATTEMPTS_PER_CANDIDATE
}

pub(super) fn unwind_leg_evidence<F>(
    legs: &[CloseLeg],
    include: F,
    include_compensation_side: bool,
) -> Vec<CloseRunUnwindLegEvidence>
where
    F: Fn(&CloseLeg) -> bool,
{
    legs.iter()
        .filter(|leg| include(leg))
        .map(|leg| unwind_leg(leg, include_compensation_side))
        .collect()
}

pub(super) fn unwind_leg(
    leg: &CloseLeg,
    include_compensation_side: bool,
) -> CloseRunUnwindLegEvidence {
    let confirmed_quantity = leg_confirmed_quantity(leg);
    let confirmed_price = leg_confirmed_price(leg);
    let notional = unwind_leg_notional(leg, confirmed_quantity, confirmed_price);
    CloseRunUnwindLegEvidence {
        venue: leg.venue.clone(),
        symbol: leg.symbol.clone(),
        side: leg.side,
        status: leg.status,
        target_quantity: leg.quantity,
        confirmed_quantity,
        confirmed_price,
        mark_price: leg.mark_price,
        notional_usd: notional.amount_usd,
        notional_quality: notional.quality,
        notional_source: notional.source,
        notional_missing_fields: notional.missing_fields,
        compensation_order_side: include_compensation_side.then(|| reopen_order_side(leg.side)),
        order_id: leg.order.as_ref().map(|order| order.intent.id.clone()),
        client_order_id: leg
            .order
            .as_ref()
            .map(|order| order.intent.client_order_id.clone()),
        exchange_order_id: leg
            .order
            .as_ref()
            .and_then(|order| order.exchange_order_id.clone()),
        finality_source: leg.finality_source,
        confirmed_filled_at_ms: leg.confirmed_filled_at_ms,
        problem: leg.problem.clone(),
    }
}

pub(super) fn leg_confirmed_quantity(leg: &CloseLeg) -> Option<f64> {
    let quantity = leg.order.as_ref()?.filled_quantity?;
    (quantity.is_finite() && quantity > 0.0).then_some(quantity)
}

pub(super) fn reopen_order_side(side: PositionSide) -> OrderSide {
    match side {
        PositionSide::Long => OrderSide::Buy,
        PositionSide::Short => OrderSide::Sell,
    }
}

pub(super) fn unwind_required_evidence() -> Vec<String> {
    [
        "fresh_position_snapshot",
        "fresh_orderbook",
        "credential_probe:order_permission",
        "private_ws_order_stream",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

pub(super) fn remaining_position_evidence(
    status: CloseRunUnwindPlanStatus,
    failed_legs: &[CloseRunUnwindLegEvidence],
    manual_terminal_evidence: Option<&CloseRunManualTerminalEvidence>,
) -> Vec<CloseRunUnwindLegEvidence> {
    match status {
        CloseRunUnwindPlanStatus::Compensated => Vec::new(),
        CloseRunUnwindPlanStatus::ManualTerminalRecorded => manual_terminal_evidence
            .map(|evidence| evidence.remaining_positions.clone())
            .unwrap_or_default(),
        CloseRunUnwindPlanStatus::BlockedPendingManualRecheck
        | CloseRunUnwindPlanStatus::CompensationSubmitted
        | CloseRunUnwindPlanStatus::CompensationFailed => failed_legs.to_vec(),
    }
}

pub(super) fn close_run_next_actions(
    status: CloseRunUnwindPlanStatus,
    candidates: &[CloseRunUnwindLegEvidence],
    attempts: &[CloseRunCompensationAttempt],
    required_evidence: &[String],
) -> Vec<CloseRunNextAction> {
    match status {
        CloseRunUnwindPlanStatus::BlockedPendingManualRecheck => {
            submit_compensation_actions(candidates, required_evidence)
        }
        CloseRunUnwindPlanStatus::CompensationSubmitted => {
            let mut actions = cancel_compensation_actions(candidates, attempts);
            actions.push(close_run_next_action(
                CloseRunNextActionKind::WaitForCompensationFinality,
                "等待补偿终态",
                None,
                false,
                Vec::new(),
                Some("补偿订单已提交，继续等待交易所成交/取消终态".to_owned()),
            ));
            actions
        }
        CloseRunUnwindPlanStatus::Compensated => Vec::new(),
        CloseRunUnwindPlanStatus::ManualTerminalRecorded => Vec::new(),
        CloseRunUnwindPlanStatus::CompensationFailed => {
            let mut actions = vec![close_run_next_action(
                CloseRunNextActionKind::ManualIncidentReview,
                "人工复核事故",
                None,
                false,
                Vec::new(),
                Some("补偿订单失败/取消，需人工复核后重新处理裸露风险".to_owned()),
            )];
            actions.extend(retry_compensation_actions(
                candidates,
                attempts,
                required_evidence,
            ));
            actions
        }
    }
}
