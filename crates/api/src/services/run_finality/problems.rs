use super::*;

pub(super) fn finality_remote_missing_problem(
    target: &PendingOrderTarget,
    checked_at_ms: i64,
) -> ApiProblem {
    let mut problem = ApiProblem::new(
        codes::HEDGE_ORDER_FINALITY_FAILED,
        "订单终态回查未找到远端订单，需要等待私有 WS 或后续回查确认",
    )
    .with_status(409)
    .with_source(FINALITY_PROBLEM_SOURCE)
    .with_request_id(Some(common::request_id::normalize(None)));
    problem.details = Some(finality_problem_details(target, None, checked_at_ms));
    problem
}

pub(super) fn finality_refresh_failure_problem(
    target: &PendingOrderTarget,
    error: &trading::TradingError,
    checked_at_ms: i64,
) -> ApiProblem {
    let mut problem = ApiProblem::new(
        codes::HEDGE_ORDER_FINALITY_FAILED,
        format!("订单终态回查失败: {error}"),
    )
    .with_status(502)
    .with_source(FINALITY_PROBLEM_SOURCE)
    .with_request_id(Some(common::request_id::normalize(None)));
    let error_message = error.to_string();
    problem.details = Some(finality_problem_details(
        target,
        Some(error_message.as_str()),
        checked_at_ms,
    ));
    problem
}

fn finality_problem_details(
    target: &PendingOrderTarget,
    error: Option<&str>,
    checked_at_ms: i64,
) -> serde_json::Value {
    serde_json::json!({
        "rawOrderId": target.raw_order_id,
        "internalOrderId": target.internal_order_id,
        "venue": target.venue.as_str(),
        "source": format!("{:?}", target.source),
        "orderState": target.state,
        "error": error,
        "checkedAtMs": checked_at_ms,
    })
}
