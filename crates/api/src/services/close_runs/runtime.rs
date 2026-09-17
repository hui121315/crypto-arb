use super::*;

pub(super) async fn validate_compensation_submit_runtime(
    state: &AppState,
    intent: &shared_types::OrderIntent,
) -> Result<(), AppError> {
    validate_compensation_capabilities(state, intent)?;
    validate_compensation_orderbook_fresh(state, intent)?;
    validate_live_operation_health(state, intent)?;
    state
        .trading_service()
        .preflight_order(intent)
        .await
        .map_err(|error| {
            AppError::domain(
                StatusCode::BAD_REQUEST,
                codes::UNSUPPORTED_CAPABILITY,
                format!("compensation order preflight failed: {error}"),
            )
            .with_details(json!({
                "exchange": intent.exchange,
                "symbol": intent.symbol,
                "orderType": intent.order_type,
                "timeInForce": intent.time_in_force,
            }))
        })
}

pub(super) fn validate_compensation_orderbook_fresh(
    state: &AppState,
    intent: &shared_types::OrderIntent,
) -> Result<(), AppError> {
    let read = state.market_data().orderbook_read(
        &intent.exchange,
        &intent.symbol,
        common::time::now_ms(),
    );
    if compensation_orderbook_ready(&read) {
        return Ok(());
    }
    Err(compensation_orderbook_error(intent, &read))
}

pub(super) fn compensation_orderbook_ready(read: &MarketRead<OrderBookInfo>) -> bool {
    read.quality == MarketQuality::Fresh && read.value.is_some()
}

pub(super) fn compensation_orderbook_error(
    intent: &shared_types::OrderIntent,
    read: &MarketRead<OrderBookInfo>,
) -> AppError {
    AppError::domain(
        StatusCode::CONFLICT,
        compensation_orderbook_problem_code(read),
        "close-run compensation requires fresh orderbook before submit",
    )
    .with_details(json!({
        "exchange": intent.exchange,
        "symbol": intent.symbol,
        "orderType": intent.order_type,
        "timeInForce": intent.time_in_force,
        "marketQuality": read.quality.as_str(),
        "marketSource": read.source.as_str(),
        "freshnessMs": read.freshness_ms,
        "retryAfterMs": read.retry_after_ms,
        "lastError": read.last_error,
    }))
}

pub(super) fn compensation_orderbook_problem_code(
    read: &MarketRead<OrderBookInfo>,
) -> &'static str {
    if read.quality == MarketQuality::Fresh && read.value.is_none() {
        return MarketQuality::Missing.problem_code();
    }
    read.quality.problem_code()
}

pub(super) fn validate_compensation_capabilities(
    state: &AppState,
    intent: &shared_types::OrderIntent,
) -> Result<(), AppError> {
    let capabilities = state
        .trading_service()
        .exchange_capabilities(&intent.exchange)
        .map_err(|error| {
            AppError::domain(
                StatusCode::BAD_REQUEST,
                codes::UNSUPPORTED_CAPABILITY,
                format!("compensation venue capability read failed: {error}"),
            )
            .with_details(json!({
                "exchange": intent.exchange,
                "symbol": intent.symbol,
            }))
        })?;
    if capabilities.supports_limit_orders {
        return Ok(());
    }
    Err(AppError::domain(
        StatusCode::BAD_REQUEST,
        codes::UNSUPPORTED_CAPABILITY,
        "compensation venue does not support limit orders",
    )
    .with_details(json!({
        "exchange": intent.exchange,
        "symbol": intent.symbol,
        "orderType": intent.order_type,
    })))
}

pub(super) fn validate_live_operation_health(
    state: &AppState,
    intent: &shared_types::OrderIntent,
) -> Result<(), AppError> {
    if intent.mode != ExecutionMode::Live {
        return Ok(());
    }
    let snapshot = crate::services::venue_operation_health::snapshot(state);
    let missing = required_live_operations()
        .into_iter()
        .filter(|operation| !operation_passed(&snapshot.rows, &intent.exchange, operation))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    Err(AppError::domain(
        StatusCode::CONFLICT,
        codes::CLOSE_RUN_UNWIND_REQUIRED,
        "live compensation requires fresh order permission and private WS order stream evidence",
    )
    .with_details(json!({
        "exchange": intent.exchange,
        "symbol": intent.symbol,
        "missingOperations": missing,
    })))
}

pub(super) fn required_live_operations() -> [&'static str; 3] {
    [
        "credential_probe:order_permission",
        "private_ws_subscribe",
        "private_ws_order_stream",
    ]
}

pub(super) fn operation_passed(
    rows: &[VenueOperationHealth],
    venue: &str,
    operation: &str,
) -> bool {
    rows.iter().any(|row| {
        venue_names_equal(&row.venue, venue)
            && row.operation == operation
            && row.status == VenueOperationStatus::Ok
            && row.supported != Some(false)
    })
}
