use super::*;

pub(super) fn cancel_compensation_actions(
    candidates: &[CloseRunUnwindLegEvidence],
    attempts: &[CloseRunCompensationAttempt],
) -> Vec<CloseRunNextAction> {
    attempts
        .iter()
        .filter(|attempt| compensation_attempt_cancellable(attempt))
        .map(|attempt| {
            close_run_next_action(
                CloseRunNextActionKind::CancelCompensationOrder,
                cancel_compensation_label(attempt),
                compensation_attempt_candidate_index(attempt, candidates),
                false,
                Vec::new(),
                Some("补偿订单仍在途，可走交易撤单 ActionRun 并等待撤单/成交终态".to_owned()),
            )
        })
        .collect()
}

fn compensation_attempt_cancellable(attempt: &CloseRunCompensationAttempt) -> bool {
    matches!(
        attempt.status,
        CloseLegStatus::Submitted | CloseLegStatus::Accepted
    ) && attempt.order.as_ref().is_some_and(|order| {
        order.intent.source == OrderSource::CloseRunCompensation
            && matches!(
                order.state,
                LiveOrderState::Submitted | LiveOrderState::Accepted | LiveOrderState::Unknown
            )
            && !order.intent.id.trim().is_empty()
    })
}

fn compensation_attempt_candidate_index(
    attempt: &CloseRunCompensationAttempt,
    candidates: &[CloseRunUnwindLegEvidence],
) -> Option<usize> {
    candidates
        .iter()
        .position(|candidate| compensation_attempt_matches_candidate(attempt, candidate))
}

pub(super) fn submit_compensation_actions(
    candidates: &[CloseRunUnwindLegEvidence],
    required_evidence: &[String],
) -> Vec<CloseRunNextAction> {
    candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            close_run_next_action(
                CloseRunNextActionKind::SubmitCompensationOrder,
                submit_compensation_label(candidate),
                Some(index),
                true,
                required_evidence.to_vec(),
                Some("已成交腿需要提交反向补偿单，避免平仓事故留下裸露方向".to_owned()),
            )
        })
        .collect()
}

pub(super) fn retry_compensation_actions(
    candidates: &[CloseRunUnwindLegEvidence],
    attempts: &[CloseRunCompensationAttempt],
    required_evidence: &[String],
) -> Vec<CloseRunNextAction> {
    candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| compensation_retry_allowed_for_candidate(candidate, attempts))
        .map(|(index, candidate)| {
            close_run_next_action(
                CloseRunNextActionKind::SubmitCompensationOrder,
                retry_compensation_label(candidate),
                Some(index),
                true,
                required_evidence.to_vec(),
                Some("上一笔补偿已到失败/取消终态，可在重新校验运行态证据后重试一次".to_owned()),
            )
        })
        .collect()
}

pub(super) fn submit_compensation_label(candidate: &CloseRunUnwindLegEvidence) -> String {
    match candidate.compensation_order_side {
        Some(OrderSide::Buy) => "提交补买补偿单".to_owned(),
        Some(OrderSide::Sell) => "提交补卖补偿单".to_owned(),
        None => "提交补偿单".to_owned(),
    }
}

pub(super) fn cancel_compensation_label(attempt: &CloseRunCompensationAttempt) -> String {
    match attempt.compensation_order_side {
        OrderSide::Buy => "撤销补买补偿单".to_owned(),
        OrderSide::Sell => "撤销补卖补偿单".to_owned(),
    }
}

pub(super) fn retry_compensation_label(candidate: &CloseRunUnwindLegEvidence) -> String {
    match candidate.compensation_order_side {
        Some(OrderSide::Buy) => "重试补买补偿单".to_owned(),
        Some(OrderSide::Sell) => "重试补卖补偿单".to_owned(),
        None => "重试补偿单".to_owned(),
    }
}

pub(super) fn close_run_next_action(
    kind: CloseRunNextActionKind,
    label: impl Into<String>,
    candidate_index: Option<usize>,
    requires_confirmation: bool,
    required_evidence: Vec<String>,
    reason: Option<String>,
) -> CloseRunNextAction {
    CloseRunNextAction {
        kind,
        label: label.into(),
        candidate_index,
        requires_confirmation,
        required_evidence,
        reason,
    }
}
