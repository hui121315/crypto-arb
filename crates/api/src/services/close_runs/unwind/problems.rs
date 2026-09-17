use super::*;

pub(in crate::services::close_runs) fn close_run_message(run: &CloseRun) -> String {
    match run.status {
        CloseRunStatus::Submitted => format!(
            "已提交 {} 条平仓订单，等待交易所成交终态",
            run.submitted_order_count
        ),
        CloseRunStatus::Succeeded => {
            format!("平仓已完成：{} 条订单已确认成交", run.submitted_order_count)
        }
        CloseRunStatus::PartiallySubmitted => {
            format!(
                "平仓终态异常：已提交 {} 条，失败/取消 {} 条",
                run.submitted_order_count, run.failed_leg_count
            )
        }
        CloseRunStatus::UnwindRequired => {
            format!(
                "平仓事故需补偿：已有成交腿，失败/取消 {} 条",
                run.failed_leg_count
            )
        }
        CloseRunStatus::CompensationSubmitted => {
            let count = compensation_attempt_count(run);
            format!("平仓事故补偿订单已提交：{count} 条，等待交易所成交终态")
        }
        CloseRunStatus::Compensated => {
            let count = compensation_attempt_count(run);
            format!("平仓事故补偿已完成：{count} 条补偿订单已确认成交")
        }
        CloseRunStatus::CompensationFailed => {
            let count = compensation_failed_attempt_count(run);
            format!("平仓事故补偿失败：{count} 条补偿订单失败/取消")
        }
        CloseRunStatus::ManuallyResolved => "平仓事故已人工复核并记录终结证据".to_owned(),
        CloseRunStatus::Failed => format!("平仓终态失败：失败/取消 {} 条", run.failed_leg_count),
    }
}

pub(super) fn compensation_attempt_count(run: &CloseRun) -> usize {
    run.unwind_plan
        .as_ref()
        .map(|plan| plan.compensation_attempts.len())
        .unwrap_or(0)
}

pub(super) fn compensation_failed_attempt_count(run: &CloseRun) -> usize {
    run.unwind_plan
        .as_ref()
        .map(|plan| {
            plan.compensation_attempts
                .iter()
                .filter(|attempt| compensation_attempt_failed(attempt))
                .count()
        })
        .unwrap_or(0)
}

pub(in crate::services::close_runs) fn close_run_problem(run: &CloseRun) -> Option<ApiProblem> {
    match run.status {
        CloseRunStatus::Submitted
        | CloseRunStatus::Succeeded
        | CloseRunStatus::Compensated
        | CloseRunStatus::ManuallyResolved => None,
        CloseRunStatus::UnwindRequired | CloseRunStatus::CompensationSubmitted => {
            let mut problem =
                ApiProblem::new(codes::CLOSE_RUN_UNWIND_REQUIRED, run.message.clone())
                    .with_status(409)
                    .with_source("portfolio.close_run.unwind");
            problem.details = Some(serde_json::json!({
                "closeRunId": run.id,
                "submittedOrderCount": run.submitted_order_count,
                "failedLegCount": run.failed_leg_count,
                "nakedExposureUsd": run.naked_exposure_usd,
                "filledLegs": unwind_plan_filled_legs(run),
                "failedLegs": unwind_plan_failed_legs(run),
                "compensationCandidates": unwind_plan_candidates(run),
                "compensationAttempts": unwind_plan_attempts(run),
                "remainingPositions": unwind_plan_remaining_positions(run),
                "nextActions": unwind_plan_next_actions(run),
                "unwindPlan": run.unwind_plan.clone(),
                "autoUnwindStatus": unwind_plan_status_label(run),
                "autoUnwindRequiredEvidence": unwind_required_evidence(),
            }));
            Some(problem)
        }
        CloseRunStatus::CompensationFailed => {
            let mut problem =
                ApiProblem::new(codes::CLOSE_RUN_COMPENSATION_FAILED, run.message.clone())
                    .with_status(409)
                    .with_source("portfolio.close_run.compensation");
            problem.details = Some(serde_json::json!({
                "closeRunId": run.id,
                "submittedOrderCount": run.submitted_order_count,
                "failedLegCount": run.failed_leg_count,
                "nakedExposureUsd": run.naked_exposure_usd,
                "filledLegs": unwind_plan_filled_legs(run),
                "failedLegs": unwind_plan_failed_legs(run),
                "compensationCandidates": unwind_plan_candidates(run),
                "compensationAttempts": unwind_plan_attempts(run),
                "remainingPositions": unwind_plan_remaining_positions(run),
                "nextActions": unwind_plan_next_actions(run),
                "unwindPlan": run.unwind_plan.clone(),
                "autoUnwindStatus": unwind_plan_status_label(run),
            }));
            Some(problem)
        }
        CloseRunStatus::PartiallySubmitted | CloseRunStatus::Failed => run
            .legs
            .iter()
            .find_map(|leg| leg.problem.clone())
            .or_else(|| {
                Some(ApiProblem::new(
                    codes::CLOSE_RUN_FAILED,
                    run.message.clone(),
                ))
            }),
    }
}
