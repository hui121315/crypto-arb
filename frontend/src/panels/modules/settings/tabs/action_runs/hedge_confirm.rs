use shared_types::{
    HedgeConfirmPartialCause, HedgeConfirmResponse, HedgeConfirmStatus, HedgeConfirmUnwindStatus,
    RecoveryAction,
};

pub(in crate::panels::modules::settings::tabs::action_runs) fn hedge_confirm_result_summary(
    result: Option<&serde_json::Value>,
) -> Option<String> {
    let response = match hedge_confirm_response(result?) {
        Ok(response) => response,
        Err(error) => return Some(format!("提交结果解码失败 · {error}")),
    };
    let outcome = response.partial_outcome.as_ref()?;
    let mut parts = vec![
        format!("事故 {}", hedge_confirm_cause_label(outcome.cause)),
        format!(
            "unwind {}",
            hedge_confirm_unwind_label(outcome.unwind_status)
        ),
    ];
    if let Some(action) = outcome.recovery_action {
        parts.push(format!("恢复 {}", recovery_action_label(action)));
    }
    if outcome.manual_review_required {
        parts.push("需要人工复核".into());
    }
    if let Some(message) = outcome.primary_message.as_deref() {
        parts.push(message.to_owned());
    }
    if let Some(problem) = outcome
        .unwind_problem
        .as_ref()
        .or(outcome.primary_problem.as_ref())
        .or(response.problem.as_ref())
    {
        parts.push(format!("code {}", problem.code));
    }
    Some(parts.join(" · "))
}

pub(super) fn hedge_confirm_payload_label(result: &serde_json::Value) -> &'static str {
    let Ok(response) = hedge_confirm_response(result) else {
        return "提交结果解码失败";
    };
    let status = response
        .partial_outcome
        .as_ref()
        .map(|outcome| outcome.original_status)
        .unwrap_or(response.status);
    hedge_confirm_status_label(status)
}

fn hedge_confirm_response(
    result: &serde_json::Value,
) -> Result<HedgeConfirmResponse, serde_json::Error> {
    serde_json::from_value(result.clone())
}

fn hedge_confirm_status_label(status: HedgeConfirmStatus) -> &'static str {
    match status {
        HedgeConfirmStatus::Submitted => "已提交",
        HedgeConfirmStatus::Replayed => "已回放",
        HedgeConfirmStatus::LongLegFailed => "第一执行腿失败",
        HedgeConfirmStatus::ValuationMissing => "估值缺失",
        HedgeConfirmStatus::FirstLegPartialUnwindAttempted => "部分成交已反向",
        HedgeConfirmStatus::FirstLegPartialUnwindFailed => "部分成交反向失败",
        HedgeConfirmStatus::FirstLegPartialWaitingFillQty => "部分成交待数量",
        HedgeConfirmStatus::HedgeRecheckBlockedUnwindAttempted => "复检拒绝已反向",
        HedgeConfirmStatus::HedgeRecheckBlockedUnwindFailed => "复检拒绝反向失败",
        HedgeConfirmStatus::HedgeBrokenUnwindAttempted => "对冲破裂已反向",
        HedgeConfirmStatus::HedgeBrokenUnwindFailed => "对冲破裂反向失败",
        HedgeConfirmStatus::Unknown => "提交状态异常",
    }
}

fn hedge_confirm_cause_label(cause: HedgeConfirmPartialCause) -> &'static str {
    match cause {
        HedgeConfirmPartialCause::FirstLegPartial => "第一腿部分成交",
        HedgeConfirmPartialCause::HedgeRecheckBlocked => "二次复检拒绝",
        HedgeConfirmPartialCause::HedgeBroken => "第二腿失败",
        HedgeConfirmPartialCause::Unknown => "未知",
    }
}

fn hedge_confirm_unwind_label(status: HedgeConfirmUnwindStatus) -> &'static str {
    match status {
        HedgeConfirmUnwindStatus::Submitted => "已提交",
        HedgeConfirmUnwindStatus::SubmitFailed => "提交失败",
        HedgeConfirmUnwindStatus::AwaitingFillQuantity => "等待成交数量",
        HedgeConfirmUnwindStatus::Unknown => "未知",
    }
}

fn recovery_action_label(action: RecoveryAction) -> &'static str {
    match action {
        RecoveryAction::CancelOpenOrders => "撤销挂单",
        RecoveryAction::UnwindLongLeg => "反向长腿",
        RecoveryAction::UnwindShortLeg => "反向短腿",
        RecoveryAction::ManualReview => "人工复核",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hedge_confirm_status_uses_payload_partial_status() {
        let result = hedge_confirm_result();

        assert_eq!(hedge_confirm_payload_label(&result), "部分成交反向失败");

        let summary = hedge_confirm_result_summary(Some(&result)).unwrap_or_default();
        assert!(summary.contains("事故 第一腿部分成交"));
        assert!(summary.contains("unwind 提交失败"));
        assert!(summary.contains("需要人工复核"));
        assert!(summary.contains("code HEDGE_UNWIND_SUBMIT_FAILED"));
    }

    #[test]
    fn hedge_confirm_status_handles_malformed_payload() {
        let malformed = serde_json::json!({ "status": "unknown_success" });
        assert_eq!(hedge_confirm_payload_label(&malformed), "提交结果解码失败");
        assert!(hedge_confirm_result_summary(Some(&malformed))
            .is_some_and(|summary| summary.contains("提交结果解码失败")));
    }

    fn hedge_confirm_result() -> serde_json::Value {
        serde_json::json!({
            "idempotencyKey": "idem-1",
            "status": "replayed",
            "executionRun": null,
            "longRecord": null,
            "shortRecord": null,
            "unwindRecord": null,
            "problem": {
                "code": "HEDGE_UNWIND_SUBMIT_FAILED",
                "message": "route down"
            },
            "partialOutcome": {
                "cause": "first_leg_partial",
                "originalStatus": "first_leg_partial_unwind_failed",
                "runId": "run-1",
                "runState": "unwind_required",
                "netExposureUsd": 42.0,
                "recoveryAction": "manual_review",
                "primaryMessage": "第一腿部分成交",
                "unwindStatus": "submit_failed",
                "unwindTargetLeg": "long",
                "unwindQuantity": 0.4,
                "unwindProblem": {
                    "code": "HEDGE_UNWIND_SUBMIT_FAILED",
                    "message": "route down"
                },
                "manualReviewRequired": true
            },
            "error": "route down"
        })
    }
}
