use axum::http::StatusCode;
use common::AppError;
use exchange::client_order_id_policy;
use shared_types::problem::codes;
use shared_types::{
    ExecutionMode, KillSwitchRequest, OrderIntent, OrderType, SelectTradingAdapterRequest,
    SubmitOrderRequest, TradingRiskStatus,
};
use trading::RiskConfig;
use uuid::Uuid;

const KILL_SWITCH_REASON_MAX_CHARS: usize = 160;

pub(super) type SelectAdapterPayload = SelectTradingAdapterRequest;

pub(super) fn risk_snapshot(config: &RiskConfig) -> TradingRiskStatus {
    crate::services::risk_config::snapshot(config)
}

pub(super) fn submit_order_intent(payload: &SubmitOrderRequest) -> Result<OrderIntent, AppError> {
    validate_submit_order(payload)?;
    let id = payload
        .id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("ord-{}", Uuid::new_v4()));
    let client_order_id = submit_order_idempotency_key(payload)?;
    let order_type = if payload.options.post_only {
        OrderType::PostOnly
    } else {
        payload.order_type
    };
    let exchange = payload.exchange.trim().to_owned();
    let client_order_id_policy = Some(client_order_id_policy(&exchange, &client_order_id));
    Ok(OrderIntent {
        id,
        source: payload.source,
        strategy: payload.strategy,
        mode: payload.mode,
        exchange,
        symbol: payload.symbol.trim().to_owned(),
        side: payload.side,
        order_type,
        quantity: payload.quantity,
        price: payload.price,
        slippage_tolerance_bps: payload.options.slippage_tolerance_bps,
        reduce_only: payload.options.reduce_only,
        time_in_force: payload.options.time_in_force,
        post_only: payload.options.post_only,
        margin_mode: payload.options.margin_mode,
        leverage: payload.options.leverage,
        client_order_id,
        client_order_id_policy,
        created_at_ms: common::time::now_ms(),
    })
}

pub(super) fn submit_order_idempotency_key(
    payload: &SubmitOrderRequest,
) -> Result<String, AppError> {
    payload
        .client_order_id
        .as_deref()
        .map(str::trim)
        .filter(|client_id| !client_id.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            submit_order_invalid(&[field_issue(
                "clientOrderId",
                "required",
                "clientOrderId is required as the public idempotency key",
            )])
        })
}

pub(super) fn submit_order_request(
    payload: serde_json::Value,
) -> Result<SubmitOrderRequest, AppError> {
    serde_json::from_value(payload).map_err(|error| submit_order_body_invalid(&error))
}

pub(super) fn validate_kill_switch_request(
    payload: &KillSwitchRequest,
    current_active: bool,
    open_order_count: usize,
) -> Result<String, AppError> {
    let reason = normalized_kill_switch_reason(&payload.reason)?;
    let Some(expected_active) = payload.expected_active else {
        return Err(kill_switch_invalid(&[field_issue(
            "expectedActive",
            "required",
            "current kill switch state is required",
        )]));
    };
    if expected_active != current_active {
        return Err(kill_switch_stale_confirmation(
            "expectedActive",
            expected_active,
            current_active,
        ));
    }
    let Some(expected_open_order_count) = payload.expected_open_order_count else {
        return Err(kill_switch_invalid(&[field_issue(
            "expectedOpenOrderCount",
            "required",
            "current open order count is required",
        )]));
    };
    if expected_open_order_count != open_order_count {
        return Err(kill_switch_stale_confirmation(
            "expectedOpenOrderCount",
            expected_open_order_count,
            open_order_count,
        ));
    }
    Ok(reason)
}

fn validate_submit_order(payload: &SubmitOrderRequest) -> Result<(), AppError> {
    let mut issues = Vec::new();
    push_required(&mut issues, "exchange", &payload.exchange);
    push_required(&mut issues, "symbol", &payload.symbol);
    if submit_order_idempotency_key(payload).is_err() {
        issues.push(field_issue(
            "clientOrderId",
            "required",
            "clientOrderId is required as the public idempotency key",
        ));
    }
    push_positive(&mut issues, "quantity", payload.quantity);
    push_positive(&mut issues, "leverage", payload.options.leverage);
    push_optional_positive(&mut issues, "price", payload.price);
    push_optional_positive(
        &mut issues,
        "slippageToleranceBps",
        payload.options.slippage_tolerance_bps,
    );
    push_public_mode_issue(&mut issues, payload.mode);
    push_limit_price_issue(&mut issues, payload);
    push_post_only_issue(&mut issues, payload);
    if issues.is_empty() {
        Ok(())
    } else {
        Err(submit_order_invalid(&issues))
    }
}

fn push_required(issues: &mut Vec<serde_json::Value>, field: &'static str, value: &str) {
    if value.trim().is_empty() {
        issues.push(field_issue(field, "required", "field is required"));
    }
}

fn push_positive(issues: &mut Vec<serde_json::Value>, field: &'static str, value: f64) {
    if !value.is_finite() || value <= 0.0 {
        issues.push(field_issue(
            field,
            "positive",
            "field must be a positive number",
        ));
    }
}

fn push_optional_positive(
    issues: &mut Vec<serde_json::Value>,
    field: &'static str,
    value: Option<f64>,
) {
    if value.is_some_and(|price| !price.is_finite() || price <= 0.0) {
        issues.push(field_issue(
            field,
            "positive",
            "field must be a positive number",
        ));
    }
}

fn push_public_mode_issue(issues: &mut Vec<serde_json::Value>, mode: ExecutionMode) {
    if mode == ExecutionMode::Testnet {
        issues.push(field_issue(
            "mode",
            "unsupported_public_mode",
            "public order submit supports only paper or live mode",
        ));
    }
}

fn push_limit_price_issue(issues: &mut Vec<serde_json::Value>, payload: &SubmitOrderRequest) {
    if matches!(payload.order_type, OrderType::Limit | OrderType::PostOnly)
        && payload.price.is_none()
    {
        issues.push(field_issue(
            "price",
            "required_for_limit",
            "limit and post-only orders require price",
        ));
    }
}

fn push_post_only_issue(issues: &mut Vec<serde_json::Value>, payload: &SubmitOrderRequest) {
    if payload.options.post_only && payload.order_type == OrderType::Market {
        issues.push(field_issue(
            "postOnly",
            "requires_limit",
            "post-only cannot be used with market orders",
        ));
    }
}

fn submit_order_invalid(issues: &[serde_json::Value]) -> AppError {
    AppError::domain(
        StatusCode::BAD_REQUEST,
        codes::SUBMIT_ORDER_INVALID,
        "invalid submit order request",
    )
    .with_details(serde_json::json!({ "fields": issues }))
}

fn submit_order_body_invalid(error: &serde_json::Error) -> AppError {
    submit_order_invalid(&[field_issue("body", "invalid_json_shape", error.to_string())])
}

fn normalized_kill_switch_reason(reason: &str) -> Result<String, AppError> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(kill_switch_invalid(&[field_issue(
            "reason",
            "required",
            "kill switch reason is required",
        )]));
    }
    if reason.chars().count() > KILL_SWITCH_REASON_MAX_CHARS {
        return Err(kill_switch_invalid(&[field_issue(
            "reason",
            "too_long",
            format!("reason must be at most {KILL_SWITCH_REASON_MAX_CHARS} chars"),
        )]));
    }
    Ok(reason.to_owned())
}

fn kill_switch_invalid(issues: &[serde_json::Value]) -> AppError {
    AppError::domain(
        StatusCode::BAD_REQUEST,
        codes::KILL_SWITCH_REQUEST_INVALID,
        "invalid kill switch request",
    )
    .with_details(serde_json::json!({ "fields": issues }))
}

fn kill_switch_stale_confirmation<T>(field: &'static str, expected: T, actual: T) -> AppError
where
    T: serde::Serialize,
{
    AppError::domain(
        StatusCode::CONFLICT,
        codes::KILL_SWITCH_STALE_CONFIRMATION,
        "kill switch confirmation snapshot changed",
    )
    .with_details(serde_json::json!({
        "field": field,
        "expected": expected,
        "actual": actual,
    }))
}

fn field_issue(
    field: impl Into<String>,
    code: impl Into<String>,
    message: impl Into<String>,
) -> serde_json::Value {
    serde_json::json!({
        "field": field.into(),
        "code": code.into(),
        "message": message.into(),
    })
}

#[cfg(test)]
mod tests;
