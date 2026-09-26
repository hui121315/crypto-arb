use super::intent::{
    close_intent_for_context, close_order_plan, close_run_id, position_notional,
};
use super::problems::{close_run_problem, problem_from_error};
use super::*;

async fn submit_close(
    state: &AppState,
    row: &PositionRow,
    context: &CloseRequestContext,
    leg_index: usize,
) -> Result<OrderRecord, AppError> {
    let (mode, engine) = context.execution.as_ref().ok_or_else(|| AppError::domain(
        StatusCode::CONFLICT,
        shared_types::problem::codes::CLOSE_RUN_PRE_TRADE_REJECTED,
        "平仓执行环境未绑定；未提交订单",
    ))?;
    let mode = *mode;
    let intent = close_intent_for_context(row, mode, context, leg_index)?;
    let mut plan = close_order_plan(&intent);
    if let Some(preflight) = crate::services::hedge_preflight::collect_live_order_preflight(
        state, mode, &intent, &mut plan,
    )
    .await
    {
        if let Some(guard) =
            crate::services::hedge_preflight::single_live_order_preflight_guards(preflight)
                .into_iter()
                .find(|guard| !guard.passed)
        {
            return Err(close_preflight_error(&guard));
        }
    }
    state
        .trading_service()
        .submit_on_engine(intent, engine)
        .await
        .map_err(crate::trading_errors::map_trading_error)
}

fn close_preflight_error(guard: &shared_types::ExecutionGuard) -> AppError {
    AppError::domain(
        StatusCode::BAD_REQUEST,
        shared_types::problem::codes::CLOSE_RUN_PRE_TRADE_REJECTED,
        guard.detail.clone(),
    )
    .with_details(serde_json::json!({ "guard": guard }))
}

pub(super) async fn submit_close_leg(
    state: &AppState,
    row: &PositionRow,
    context: &CloseRequestContext,
    leg_index: usize,
) -> CloseLeg {
    match submit_close(state, row, context, leg_index).await {
        Ok(order) => close_leg_submitted(row, order),
        Err(error) => close_leg_failed(row, &error),
    }
}

fn close_leg_submitted(row: &PositionRow, order: OrderRecord) -> CloseLeg {
    close_leg(row, CloseLegStatus::Submitted, Some(order), None)
}

fn close_leg_failed(row: &PositionRow, error: &AppError) -> CloseLeg {
    close_leg(
        row,
        CloseLegStatus::Failed,
        None,
        Some(problem_from_error(error)),
    )
}

fn close_leg(
    row: &PositionRow,
    status: CloseLegStatus,
    order: Option<OrderRecord>,
    problem: Option<ApiProblem>,
) -> CloseLeg {
    CloseLeg {
        venue: row.venue.clone(),
        symbol: row.symbol.clone(),
        side: row.side,
        status,
        quantity: row.quantity,
        mark_price: row.mark_price,
        notional_usd: position_notional(row),
        order,
        finality_source: None,
        confirmed_filled_at_ms: None,
        problem,
        pair_evidence: row.pair_evidence.clone(),
        cost_events: Vec::new(),
    }
}

pub(super) fn close_run(
    scope: CloseRunScope,
    legs: Vec<CloseLeg>,
    started_at_ms: i64,
    context: CloseRequestContext,
) -> CloseRun {
    let status = close_run_status(&legs);
    let submitted_order_count = submitted_order_count(&legs);
    let failed_leg_count = failed_leg_count(&legs);
    let naked_exposure_usd = naked_exposure_usd(&legs);
    let message = close_run_message(status, submitted_order_count, failed_leg_count);
    let problem = close_run_problem(status, &message, &legs);
    let now_ms = common::time::now_ms();
    CloseRun {
        id: close_run_id(scope),
        scope,
        status,
        action_run_id: None,
        request_id: common::request_id::current(),
        idempotency_key: context.idempotency_key.clone(),
        snapshot_version: context.snapshot_version,
        expected_leg_count: context.expected_leg_count,
        reason: context.reason,
        legs,
        submitted_order_count,
        failed_leg_count,
        naked_exposure_usd,
        message,
        problem,
        finality_problem: None,
        finality_checked_at_ms: None,
        unwind_plan: None,
        cost_events: Vec::new(),
        cost_reconciliation: None,
        started_at_ms,
        updated_at_ms: now_ms,
    }
}

fn close_run_status(legs: &[CloseLeg]) -> CloseRunStatus {
    let failed = failed_leg_count(legs);
    let submitted = submitted_order_count(legs);
    if failed == 0 {
        CloseRunStatus::Submitted
    } else if submitted == 0 {
        CloseRunStatus::Failed
    } else {
        CloseRunStatus::PartiallySubmitted
    }
}

fn submitted_order_count(legs: &[CloseLeg]) -> usize {
    legs.iter()
        .filter(|leg| matches!(leg.status, CloseLegStatus::Submitted))
        .count()
}

fn failed_leg_count(legs: &[CloseLeg]) -> usize {
    legs.iter()
        .filter(|leg| matches!(leg.status, CloseLegStatus::Failed))
        .count()
}

fn naked_exposure_usd(legs: &[CloseLeg]) -> f64 {
    legs.iter()
        .filter(|leg| !matches!(leg.status, CloseLegStatus::Submitted))
        .map(|leg| leg.notional_usd)
        .sum()
}

fn close_run_message(status: CloseRunStatus, submitted: usize, failed: usize) -> String {
    match status {
        CloseRunStatus::Submitted => format!("已提交 {submitted} 条平仓订单，等待交易所成交终态"),
        CloseRunStatus::Succeeded => format!("平仓已完成：{submitted} 条订单已确认"),
        CloseRunStatus::PartiallySubmitted => {
            format!("部分平仓已提交：成功 {submitted} 条，失败 {failed} 条")
        }
        CloseRunStatus::UnwindRequired => {
            format!("平仓事故需补偿：成功 {submitted} 条，失败 {failed} 条")
        }
        CloseRunStatus::CompensationSubmitted => {
            "平仓事故补偿订单已提交，等待交易所成交终态".to_owned()
        }
        CloseRunStatus::Compensated => "平仓事故补偿已完成".to_owned(),
        CloseRunStatus::CompensationFailed => "平仓事故补偿失败，需人工处理".to_owned(),
        CloseRunStatus::ManuallyResolved => "平仓事故已人工复核并记录终结证据".to_owned(),
        CloseRunStatus::Failed => format!("平仓未提交成功：失败 {failed} 条"),
    }
}
