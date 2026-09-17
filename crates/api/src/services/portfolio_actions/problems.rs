use super::*;

pub(super) fn close_run_problem(
    status: CloseRunStatus,
    message: &str,
    legs: &[CloseLeg],
) -> Option<ApiProblem> {
    let mut problem = match status {
        CloseRunStatus::Submitted
        | CloseRunStatus::Succeeded
        | CloseRunStatus::Compensated
        | CloseRunStatus::ManuallyResolved => None,
        CloseRunStatus::UnwindRequired | CloseRunStatus::CompensationSubmitted => Some(
            ApiProblem::new(
                shared_types::problem::codes::CLOSE_RUN_UNWIND_REQUIRED,
                message,
            )
            .with_source("portfolio.close_run.unwind")
            .with_request_id(common::request_id::current()),
        ),
        CloseRunStatus::PartiallySubmitted => Some(
            ApiProblem::new(shared_types::problem::codes::CLOSE_RUN_PARTIAL, message)
                .with_source("portfolio.close_run")
                .with_request_id(common::request_id::current()),
        ),
        CloseRunStatus::CompensationFailed => Some(
            ApiProblem::new(
                shared_types::problem::codes::CLOSE_RUN_COMPENSATION_FAILED,
                message,
            )
            .with_source("portfolio.close_run.compensation")
            .with_request_id(common::request_id::current()),
        ),
        CloseRunStatus::Failed => Some(
            ApiProblem::new(shared_types::problem::codes::CLOSE_RUN_FAILED, message)
                .with_source("portfolio.close_run")
                .with_request_id(common::request_id::current()),
        ),
    }?;
    problem.details = Some(close_problem_details(legs));
    Some(problem)
}

fn close_problem_details(legs: &[CloseLeg]) -> serde_json::Value {
    json!({
        "submittedLegs": leg_summaries(legs, |leg| matches!(leg.status, CloseLegStatus::Submitted)),
        "failedLegs": leg_summaries(legs, |leg| matches!(leg.status, CloseLegStatus::Failed)),
        "compensationCandidates": Vec::<serde_json::Value>::new(),
        "autoUnwindStatus": "blocked_pending_manual_recheck",
        "autoUnwindRequiredEvidence": [
            "fresh_position_snapshot",
            "fresh_orderbook",
            "credential_probe:order_permission",
            "private_ws_order_stream"
        ],
    })
}

fn leg_summaries<F>(legs: &[CloseLeg], include: F) -> Vec<serde_json::Value>
where
    F: Fn(&CloseLeg) -> bool,
{
    legs.iter()
        .filter(|leg| include(leg))
        .map(leg_summary)
        .collect()
}

fn leg_summary(leg: &CloseLeg) -> serde_json::Value {
    json!({
        "venue": &leg.venue,
        "symbol": &leg.symbol,
        "side": leg.side,
        "status": leg.status,
        "quantity": leg.quantity,
        "notionalUsd": leg.notional_usd,
        "problem": leg.problem.clone(),
    })
}

pub(super) fn problem_from_error(error: &AppError) -> ApiProblem {
    error.to_api_problem()
}
