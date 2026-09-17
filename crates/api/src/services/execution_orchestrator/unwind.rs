use super::events::{publish_order_event, refresh_order_status};
use super::run_model::{ledger_context, register_order_ids, update_run};
use super::*;

pub(super) fn mark_unwind_result(
    state: &AppState,
    run: &mut ExecutionRun,
    unwind_result: Result<OrderRecord, ApiProblem>,
    target_role: HedgeLegRole,
    submitted_reason: &str,
    failed_reason: &str,
) -> (Option<OrderRecord>, Option<ApiProblem>) {
    match unwind_result {
        Ok(record) => {
            mark_unwind_submitted(state, run, &record, target_role, submitted_reason);
            (Some(record), None)
        }
        Err(error) => {
            let problem = mark_unwind_failed(run, failed_reason, &error);
            update_run(state, run, &problem.message);
            (None, Some(problem))
        }
    }
}

fn mark_unwind_submitted(
    state: &AppState,
    run: &mut ExecutionRun,
    record: &OrderRecord,
    target_role: HedgeLegRole,
    reason: &str,
) {
    register_order_ids(super::legs::run_leg_mut(run, target_role), record);
    run.state = ExecutionRunState::Unwinding;
    run.unwind_problem = None;
    update_run(state, run, reason);
}

pub(super) fn mark_unwind_failed(
    run: &mut ExecutionRun,
    reason: &str,
    error: &ApiProblem,
) -> ApiProblem {
    let problem = unwind_submit_problem(run, reason, error);
    run.state = ExecutionRunState::UnwindRequired;
    run.recovery_action = Some(RecoveryAction::ManualReview);
    run.unwind_problem = Some(problem.clone());
    problem
}

fn unwind_submit_problem(run: &ExecutionRun, reason: &str, error: &ApiProblem) -> ApiProblem {
    unwind_problem(run, codes::HEDGE_UNWIND_SUBMIT_FAILED, reason, Some(error))
}

pub(super) fn unwind_skipped_problem(run: &ExecutionRun, reason: &str) -> ApiProblem {
    unwind_problem(run, codes::HEDGE_UNWIND_SKIPPED, reason, None)
}

fn unwind_problem(
    run: &ExecutionRun,
    code: &str,
    reason: &str,
    error: Option<&ApiProblem>,
) -> ApiProblem {
    let message = error.as_ref().map_or_else(
        || reason.to_owned(),
        |error| format!("{reason}: {}", error.message),
    );
    let mut problem = ApiProblem::new(code, message)
        .with_status(error.as_ref().and_then(|error| error.status).unwrap_or(409))
        .with_source(
            error
                .as_ref()
                .and_then(|error| error.source.clone())
                .unwrap_or_else(|| "execution_orchestrator".to_owned()),
        )
        .with_request_id(error.as_ref().and_then(|error| error.request_id.clone()))
        .with_retry_after_ms(error.as_ref().and_then(|error| error.retry_after_ms));
    problem.details = Some(serde_json::json!({
        "runId": run.run_id,
        "ticketId": run.ticket_id,
        "opportunityId": run.opportunity_id,
        "reason": reason,
        "submitProblem": error,
        "recoveryAction": run.recovery_action,
        "longLegState": run.long_leg.state,
        "shortLegState": run.short_leg.state,
        "netExposureUsd": run.net_exposure_usd,
    }));
    problem
}

pub(super) async fn submit_unwind(
    state: &AppState,
    open_record: &OrderRecord,
    key: &str,
    run: &ExecutionRun,
    target_role: HedgeLegRole,
) -> Result<OrderRecord, ApiProblem> {
    let record = state
        .trading_service()
        .submit_unwind_with_ledger_context(
            unwind_intent(open_record, key),
            ledger_context(run, target_role),
        )
        .await
        .map_err(|error| {
            crate::trading_errors::trading_error_problem(
                &error,
                &open_record.intent.exchange,
                "submit_unwind_order",
            )
        })?;
    publish_order_event(state, "hedge_unwind_submitted", &record);
    Ok(refresh_order_status(state, record, "hedge_unwind_status_backfilled").await)
}

pub(super) fn unwind_intent(open_record: &OrderRecord, key: &str) -> OrderIntent {
    let open_leg = &open_record.intent;
    let side = match open_leg.side {
        OrderSide::Buy => OrderSide::Sell,
        OrderSide::Sell => OrderSide::Buy,
    };
    OrderIntent {
        id: format!("{key}-unwind"),
        source: open_leg.source,
        strategy: open_leg.strategy,
        mode: open_leg.mode,
        exchange: open_leg.exchange.clone(),
        symbol: open_leg.symbol.clone(),
        side,
        order_type: OrderType::Market,
        quantity: open_record
            .filled_quantity
            .filter(|quantity| quantity.is_finite() && *quantity > 0.0)
            .unwrap_or(open_leg.quantity),
        price: open_leg.price,
        slippage_tolerance_bps: None,
        reduce_only: true,
        time_in_force: open_leg.time_in_force,
        post_only: false,
        margin_mode: open_leg.margin_mode,
        leverage: open_leg.leverage,
        client_order_id: format!("{key}-unwind"),
        client_order_id_policy: None,
        created_at_ms: common::time::now_ms(),
    }
}

pub(super) fn unwind_status(
    success: HedgeConfirmStatus,
    failed: HedgeConfirmStatus,
    error: &Option<ApiProblem>,
) -> HedgeConfirmStatus {
    if error.is_some() {
        failed
    } else {
        success
    }
}
