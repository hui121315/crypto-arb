//! Gate futures private REST fill evidence.

use crate::error::{ExchangeError, ExchangeResult};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct GateFuturesFillEvidence {
    pub(super) trade_id: String,
    pub(super) order_id: String,
    pub(super) contract: String,
    pub(super) size: f64,
    pub(super) price: f64,
    pub(super) role: GateLiquidityRole,
    pub(super) fee: f64,
    pub(super) fee_currency: String,
    pub(super) point_fee: f64,
    pub(super) occurred_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GateLiquidityRole {
    Maker,
    Taker,
}

#[derive(Debug, Deserialize)]
pub(super) struct GateMyTradeRow {
    id: serde_json::Value,
    create_time: serde_json::Value,
    contract: String,
    order_id: serde_json::Value,
    size: serde_json::Value,
    price: serde_json::Value,
    fee: serde_json::Value,
    #[serde(default)]
    point_fee: serde_json::Value,
    role: String,
}

pub(super) fn parse_my_trades(
    rows: Vec<GateMyTradeRow>,
    settle: &str,
    requested_order_id: &str,
) -> ExchangeResult<Vec<GateFuturesFillEvidence>> {
    let fee_currency = verified_settle(settle)?;
    let requested_order_id = positive_u64(requested_order_id, "requested order")?.to_string();
    rows.into_iter()
        .map(|row| parse_trade(&row, &fee_currency, &requested_order_id))
        .collect()
}

fn parse_trade(
    row: &GateMyTradeRow,
    fee_currency: &str,
    requested_order_id: &str,
) -> ExchangeResult<GateFuturesFillEvidence> {
    let order_id = value_u64(&row.order_id, "my_trades.order_id")?.to_string();
    if order_id != requested_order_id {
        return Err(ExchangeError::Parse(format!(
            "gate my_trades order mismatch: requested={requested_order_id} returned={order_id}"
        )));
    }
    Ok(GateFuturesFillEvidence {
        trade_id: value_u64(&row.id, "my_trades.id")?.to_string(),
        order_id,
        contract: required_text(&row.contract, "my_trades.contract")?,
        size: non_zero_number(&row.size, "my_trades.size")?,
        price: positive_number(&row.price, "my_trades.price")?,
        role: liquidity_role(&row.role)?,
        fee: finite_number(&row.fee, "my_trades.fee")?,
        fee_currency: fee_currency.to_owned(),
        point_fee: optional_point_fee(&row.point_fee)?,
        occurred_at_ms: seconds_to_millis(&row.create_time)?,
    })
}

fn verified_settle(settle: &str) -> ExchangeResult<String> {
    match settle.trim().to_ascii_lowercase().as_str() {
        "usdt" => Ok("USDT".into()),
        "btc" => Ok("BTC".into()),
        other => Err(ExchangeError::Parse(format!(
            "gate my_trades unsupported settle evidence: {other}"
        ))),
    }
}

fn liquidity_role(value: &str) -> ExchangeResult<GateLiquidityRole> {
    match value.trim().to_ascii_lowercase().as_str() {
        "maker" => Ok(GateLiquidityRole::Maker),
        "taker" => Ok(GateLiquidityRole::Taker),
        other => Err(ExchangeError::Parse(format!(
            "gate my_trades.role unsupported: {other}"
        ))),
    }
}

fn required_text(value: &str, field: &str) -> ExchangeResult<String> {
    let value = value.trim();
    if value.is_empty() {
        Err(ExchangeError::Parse(format!("gate {field} is empty")))
    } else {
        Ok(value.to_owned())
    }
}

fn value_u64(value: &serde_json::Value, field: &str) -> ExchangeResult<u64> {
    match value {
        serde_json::Value::Number(number) => positive_u64(&number.to_string(), field),
        serde_json::Value::String(text) => positive_u64(text, field),
        _ => Err(ExchangeError::Parse(format!("gate {field} is not numeric"))),
    }
}

fn positive_u64(value: &str, field: &str) -> ExchangeResult<u64> {
    let value = value
        .parse::<u64>()
        .map_err(|_| ExchangeError::Parse(format!("gate {field} invalid integer: {value}")))?;
    if value == 0 {
        return Err(ExchangeError::Parse(format!(
            "gate {field} must be greater than zero"
        )));
    }
    Ok(value)
}

fn positive_number(value: &serde_json::Value, field: &str) -> ExchangeResult<f64> {
    let number = finite_number(value, field)?;
    if number <= 0.0 {
        return Err(ExchangeError::Parse(format!(
            "gate {field} must be positive"
        )));
    }
    Ok(number)
}

fn non_zero_number(value: &serde_json::Value, field: &str) -> ExchangeResult<f64> {
    let number = finite_number(value, field)?;
    if number == 0.0 {
        return Err(ExchangeError::Parse(format!(
            "gate {field} must be non-zero"
        )));
    }
    Ok(number)
}

fn finite_number(value: &serde_json::Value, field: &str) -> ExchangeResult<f64> {
    let parsed = match value {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
    .filter(|number| number.is_finite())
    .ok_or_else(|| ExchangeError::Parse(format!("gate {field} invalid number: {value}")))?;
    Ok(parsed)
}

fn optional_point_fee(value: &serde_json::Value) -> ExchangeResult<f64> {
    if value.is_null() {
        Ok(0.0)
    } else {
        finite_number(value, "my_trades.point_fee")
    }
}

fn seconds_to_millis(value: &serde_json::Value) -> ExchangeResult<i64> {
    let seconds = finite_number(value, "my_trades.create_time")?;
    let millis = seconds * 1_000.0;
    if seconds <= 0.0 || millis > i64::MAX as f64 {
        return Err(ExchangeError::Parse(
            "gate my_trades.create_time outside supported range".into(),
        ));
    }
    Ok(millis.round() as i64)
}

#[cfg(test)]
#[path = "gate_fill_evidence_tests.rs"]
mod tests;
