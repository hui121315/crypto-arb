use shared_types::{
    ApiProblem, CloseRun, CloseRunStatus, LiveOrderState, OrderRecord, OrderUpdateSource,
};

pub(in crate::panels::modules::positions) fn close_run_problem(run: &CloseRun) -> ApiProblem {
    let problem = single_failed_leg_problem(run)
        .or_else(|| run.problem.clone())
        .unwrap_or_else(|| {
            ApiProblem::new(
                shared_types::problem::codes::CLOSE_RUN_FAILED,
                run.message.clone(),
            )
            .with_source("positions.close_run")
        });
    close_run_problem_with_request(problem, run.request_id.clone())
}

pub(in crate::panels::modules::positions) fn close_run_retry_anchor(
    run: &CloseRun,
) -> Option<(String, i64)> {
    (run.status == CloseRunStatus::Failed && close_run_is_safe_for_explicit_retry(run))
        .then(|| (run.id.clone(), run.updated_at_ms))
}

fn close_run_is_safe_for_explicit_retry(run: &CloseRun) -> bool {
    if run.submitted_order_count == 0 {
        return run.legs.iter().any(|leg| {
            leg.problem
                .as_ref()
                .is_some_and(definitive_not_submitted_problem)
        });
    }
    !run.legs.is_empty() && run.legs.iter().all(close_leg_is_terminal_without_fill)
}

fn close_leg_is_terminal_without_fill(leg: &shared_types::CloseLeg) -> bool {
    leg.order
        .as_ref()
        .is_some_and(terminal_order_is_safe_for_explicit_retry)
}

fn terminal_order_is_safe_for_explicit_retry(order: &OrderRecord) -> bool {
    if order
        .filled_quantity
        .is_some_and(|quantity| !quantity.is_finite() || quantity > 0.0)
    {
        return false;
    }
    match order.state {
        LiveOrderState::Failed => bitget_order_was_confirmed_absent(order),
        _ => false,
    }
}

fn bitget_order_was_confirmed_absent(order: &OrderRecord) -> bool {
    order.intent.exchange.eq_ignore_ascii_case("bitget")
        && order.exchange_order_id.is_none()
        && order.last_update_source == OrderUpdateSource::Reconcile
        && order.message.as_deref().is_some_and(|message| {
            message.contains("order was not found by clientOid after the")
                && message.contains("ambiguity window; submission was not retried")
        })
}

fn definitive_not_submitted_problem(problem: &ApiProblem) -> bool {
    problem.code == shared_types::problem::codes::CREDENTIAL_PERMISSION_DENIED
        || problem.code == shared_types::problem::codes::CLOSE_RUN_PRE_TRADE_REJECTED
        || problem
            .details
            .as_ref()
            .and_then(|details| details.get("venueCode"))
            .and_then(serde_json::Value::as_str)
            == Some("-2015")
}

fn single_failed_leg_problem(run: &CloseRun) -> Option<ApiProblem> {
    (run.expected_leg_count == 1 && run.submitted_order_count == 0)
        .then(|| {
            run.legs
                .iter()
                .find_map(|leg| leg.problem.clone())
                .map(normalize_legacy_binance_permission_problem)
        })
        .flatten()
}

fn normalize_legacy_binance_permission_problem(mut problem: ApiProblem) -> ApiProblem {
    let is_binance_2015 = problem.code == shared_types::problem::codes::UPSTREAM_API
        && problem.message.contains("binance api error")
        && problem.message.contains("code=-2015");
    if !is_binance_2015 {
        return problem;
    }

    problem.code = shared_types::problem::codes::CREDENTIAL_PERMISSION_DENIED.to_owned();
    problem.message = "Binance 实盘写权限被拒绝（-2015）。请在 API Management 检查主网 Key、Futures 交易权限及后端公网 IP 白名单；无需开启提现权限。".to_owned();
    problem.status = Some(403);
    problem.recovery_action = Some(shared_types::ApiRecoveryAction::CheckPermissions);
    problem
}

fn close_run_problem_with_request(problem: ApiProblem, request_id: Option<String>) -> ApiProblem {
    if problem.request_id.is_some() {
        return problem;
    }
    problem.with_request_id(request_id)
}
