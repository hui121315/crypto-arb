//! Shared OKX private-read parser rules for REST and live adapters.

use crate::error::{ExchangeError, ExchangeResult};
use chrono::{DateTime, Utc};
use shared_types::{OrderSide, OrderStatus, OrderType};

pub(super) fn parse_required_number(scope: &str, field: &str, raw: &str) -> ExchangeResult<f64> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(parse_error(scope, field, raw));
    }
    parse_finite_number(scope, field, value)
}

pub(super) fn parse_optional_zero_number(
    scope: &str,
    field: &str,
    raw: &str,
) -> ExchangeResult<f64> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(0.0);
    }
    parse_finite_number(scope, field, value)
}

pub(super) fn parse_positive_number_option(
    scope: &str,
    field: &str,
    raw: &str,
) -> ExchangeResult<Option<f64>> {
    let value = parse_optional_zero_number(scope, field, raw)?;
    Ok((value > 0.0).then_some(value))
}

pub(super) fn parse_timestamp_ms(
    scope: &str,
    field: &str,
    raw: &str,
) -> ExchangeResult<DateTime<Utc>> {
    let timestamp = raw
        .trim()
        .parse::<i64>()
        .map_err(|_| parse_error(scope, field, raw))?;
    if timestamp <= 0 {
        return Err(parse_error(scope, field, raw));
    }
    DateTime::<Utc>::from_timestamp_millis(timestamp).ok_or_else(|| parse_error(scope, field, raw))
}

pub(super) fn parse_order_side(scope: &str, raw: &str) -> ExchangeResult<OrderSide> {
    match raw.to_ascii_lowercase().as_str() {
        "buy" => Ok(OrderSide::Buy),
        "sell" => Ok(OrderSide::Sell),
        _ => Err(parse_error(scope, "side", raw)),
    }
}

pub(super) fn parse_order_type(scope: &str, raw: &str) -> ExchangeResult<OrderType> {
    match raw.to_ascii_lowercase().as_str() {
        "market" | "optimal_limit_ioc" => Ok(OrderType::Market),
        "post_only" => Ok(OrderType::PostOnly),
        "limit" | "ioc" | "fok" => Ok(OrderType::Limit),
        _ => Err(parse_error(scope, "ordType", raw)),
    }
}

pub(super) fn parse_order_status(scope: &str, raw: &str) -> ExchangeResult<OrderStatus> {
    match raw.to_ascii_lowercase().as_str() {
        "live" => Ok(OrderStatus::Open),
        "partially_filled" => Ok(OrderStatus::PartiallyFilled),
        "filled" => Ok(OrderStatus::Filled),
        "canceled" | "mmp_canceled" => Ok(OrderStatus::Canceled),
        "rejected" => Ok(OrderStatus::Rejected),
        _ => Err(parse_error(scope, "state", raw)),
    }
}

pub(super) fn parse_optional_reduce_only(scope: &str, raw: &str) -> ExchangeResult<Option<bool>> {
    match raw.trim() {
        "" => Ok(None),
        "true" => Ok(Some(true)),
        "false" => Ok(Some(false)),
        _ => Err(parse_error(scope, "reduceOnly", raw)),
    }
}

pub(super) fn parse_position_side(
    scope: &str,
    pos_side: &str,
    quantity: f64,
) -> ExchangeResult<String> {
    match pos_side.to_ascii_lowercase().as_str() {
        "long" => Ok("long".to_owned()),
        "short" => Ok("short".to_owned()),
        "" | "net" if quantity < 0.0 => Ok("short".to_owned()),
        "" | "net" => Ok("long".to_owned()),
        _ => Err(parse_error(scope, "posSide", pos_side)),
    }
}

fn parse_finite_number(scope: &str, field: &str, value: &str) -> ExchangeResult<f64> {
    let parsed = value
        .parse::<f64>()
        .map_err(|_| parse_error(scope, field, value))?;
    if !parsed.is_finite() {
        return Err(parse_error(scope, field, value));
    }
    Ok(parsed)
}

fn parse_error(scope: &str, field: &str, value: &str) -> ExchangeError {
    ExchangeError::Parse(format!("okx {scope} field {field} invalid: {value}"))
}
