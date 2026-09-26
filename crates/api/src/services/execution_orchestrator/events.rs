use super::run_model::update_run;
use super::*;

pub(super) fn valuation_failed_response(
    state: &AppState,
    idempotency_key: String,
    mut run: ExecutionRun,
    long_record: Option<OrderRecord>,
    short_record: Option<OrderRecord>,
    problem: ApiProblem,
) -> HedgeConfirmResponse {
    run.state = ExecutionRunState::UnwindRequired;
    run.recovery_action = Some(RecoveryAction::ManualReview);
    run.status_reason = problem.message.clone();
    run.valuation_problem = Some(problem);
    let error = run.status_reason.clone();
    update_run(state, &mut run, "执行估值缺少价格，需要人工复核");
    confirm_response(
        idempotency_key,
        HedgeConfirmStatus::ValuationMissing,
        run,
        (long_record, short_record),
        None,
        Some(error),
    )
}

pub(super) fn first_leg_event(record: &OrderRecord) -> &'static str {
    if record.state == LiveOrderState::PartiallyFilled {
        "hedge_first_leg_partial"
    } else {
        "hedge_first_leg_submitted"
    }
}

pub(super) fn publish_order_event(state: &AppState, event: &'static str, record: &OrderRecord) {
    if let Err(error) = crate::services::ws_publish::publish_order_event(state, event, record) {
        tracing::warn!(
            %error,
            event,
            order_id = %record.intent.id,
            "failed to publish hedge order event"
        );
    }
}

pub(super) async fn refresh_order_status(
    state: &AppState,
    record: OrderRecord,
    event: &'static str,
    engine: &ExecutionEngine,
) -> OrderRecord {
    if order_finality_complete(&record) {
        return record;
    }
    let record = await_private_order_finality(state, record).await;
    if order_finality_complete(&record) {
        publish_order_event(state, event, &record);
        return record;
    }
    match state
        .trading_service()
        .refresh_order_state_on_engine(&record.intent.id, engine)
        .await
    {
        Ok(Some(updated)) => {
            publish_order_event(state, event, &updated);
            updated
        }
        Ok(None) => record,
        Err(error) => {
            tracing::warn!(
                %error,
                order_id = %record.intent.id,
                "order status backfill failed"
            );
            record
        }
    }
}

pub(super) async fn settle_first_leg(
    state: &AppState,
    record: OrderRecord,
    engine: &ExecutionEngine,
) -> Result<OrderRecord, (OrderRecord, ApiProblem)> {
    if order_terminal(record.state) {
        return Ok(record);
    }
    let order_id = record.intent.id.clone();
    let cancelled = match state.trading_service().cancel_on_engine(&order_id, engine).await {
        Ok(cancelled) => cancelled,
        Err(error) => {
            let problem = crate::trading_errors::trading_error_problem(
                &error,
                &record.intent.exchange,
                "cancel_unfilled_first_leg",
            );
            return Err((record, problem));
        }
    };
    publish_order_event(state, "hedge_first_leg_cancelled_before_second", &cancelled);
    Ok(refresh_order_status(
        state,
        cancelled,
        "hedge_first_leg_cancel_finality_backfilled",
        engine,
    )
    .await)
}

async fn await_private_order_finality(state: &AppState, record: OrderRecord) -> OrderRecord {
    const WAIT_MS: u64 = 600;
    const POLL_MS: u64 = 20;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(WAIT_MS);
    let mut latest = record;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(POLL_MS)).await;
        if let Some(updated) = state.trading_service().get_order(&latest.intent.id) {
            latest = updated;
            if order_finality_complete(&latest) {
                break;
            }
        }
    }
    latest
}

pub(super) fn order_finality_complete(record: &OrderRecord) -> bool {
    match record.state {
        LiveOrderState::Filled | LiveOrderState::PartiallyFilled => {
            record
                .filled_quantity
                .is_some_and(|quantity| quantity.is_finite() && quantity > 0.0)
                && record
                    .filled_price
                    .is_some_and(|price| price.is_finite() && price > 0.0)
        }
        LiveOrderState::Cancelled | LiveOrderState::Rejected | LiveOrderState::Failed => true,
        _ => false,
    }
}

fn order_terminal(state: LiveOrderState) -> bool {
    matches!(
        state,
        LiveOrderState::Filled
            | LiveOrderState::PartiallyFilled
            | LiveOrderState::Cancelled
            | LiveOrderState::Rejected
            | LiveOrderState::Failed
    )
}

pub(super) fn publish_run_event(state: &AppState, run: &ExecutionRun) {
    if let Err(error) = crate::services::ws_publish::publish_execution_run_event(
        state,
        "execution_run_updated",
        run,
    ) {
        tracing::warn!(
            %error,
            run_id = %run.run_id,
            "failed to publish hedge execution run event"
        );
    }
}

pub(super) fn confirm_response(
    idempotency_key: String,
    status: HedgeConfirmStatus,
    run: ExecutionRun,
    records: (Option<OrderRecord>, Option<OrderRecord>),
    problem: Option<ApiProblem>,
    error: Option<impl std::fmt::Display>,
) -> HedgeConfirmResponse {
    let (long_record, short_record) = records;
    HedgeConfirmResponse {
        idempotency_key,
        status,
        context: shared_types::HedgeConfirmContext::default(),
        problem: problem.or_else(|| run_problem(&run)),
        execution_run: Some(run),
        long_record,
        short_record,
        unwind_record: None,
        partial_outcome: None,
        error: error.map(|error| error.to_string()),
    }
}

fn run_problem(run: &ExecutionRun) -> Option<ApiProblem> {
    run.unwind_problem
        .clone()
        .or_else(|| run.valuation_problem.clone())
}

pub(super) fn problem_message(problem: &Option<ApiProblem>) -> Option<String> {
    problem.as_ref().map(|problem| problem.message.clone())
}

pub(super) fn combine_errors(primary: Option<String>, secondary: Option<String>) -> Option<String> {
    match (primary, secondary) {
        (Some(primary), Some(secondary)) => Some(format!("{primary}; {secondary}")),
        (Some(primary), None) => Some(primary),
        (None, Some(secondary)) => Some(secondary),
        (None, None) => None,
    }
}
