//! Gate private read response parsing.
//!
//! Official Gate API v4 docs checked before moving these DTOs:
//! - GET /futures/{settle}/accounts
//! - GET /futures/{settle}/positions
//! - GET /futures/{settle}/orders
//! - Generated SDK model docs: `FuturesAccount`, `Position`, `FuturesOrder`

use crate::adapter::{client_order_id_from_str, strip_common_suffixes};
use crate::error::{ExchangeError, ExchangeResult};
use crate::live::{venue_balance_rows, VenueAccountRead};
use serde::Deserialize;
use serde_json::Value;
use shared_types::{
    AccountEquityScope, BalanceInfo, OrderInfo, OrderSide, OrderStatus, OrderType, PositionInfo,
    VenueAccountSummary,
};
use std::collections::HashMap;

const NAME: &str = "gate";

#[derive(Debug, Deserialize)]
pub(super) struct AccountItem {
    #[serde(default)]
    currency: String,
    total: String,
    available: String,
    #[serde(rename = "position_margin")]
    position_margin: String,
    #[serde(rename = "order_margin")]
    order_margin: String,
    #[serde(rename = "unrealised_pnl")]
    unrealised_pnl: String,
    #[serde(default, rename = "cross_maintenance_margin")]
    cross_maintenance_margin: String,
    #[serde(default, rename = "position_mode")]
    position_mode: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct PositionRow {
    contract: String,
    #[serde(deserialize_with = "de_f64_from_string_or_number")]
    size: f64,
    #[serde(default, rename = "entry_price")]
    entry_price: String,
    #[serde(default, rename = "mark_price")]
    mark_price: String,
    #[serde(default, rename = "unrealised_pnl")]
    unrealised_pnl: String,
    #[serde(default)]
    leverage: String,
    #[serde(default, rename = "liq_price")]
    liq_price: String,
    #[serde(default)]
    margin: String,
    #[serde(rename = "maintenance_rate")]
    maintenance_rate: String,
    #[serde(default)]
    mode: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenOrderItem {
    id: i64,
    contract: String,
    status: String,
    #[serde(default, rename = "finish_as")]
    finish_as: String,
    #[serde(deserialize_with = "de_f64_from_string_or_number")]
    size: f64,
    #[serde(deserialize_with = "de_f64_from_string_or_number")]
    left: f64,
    price: String,
    #[serde(rename = "fill_price")]
    fill_price: String,
    #[serde(default, rename = "create_time_ms", deserialize_with = "de_ms_to_i64")]
    create_time_ms: i64,
    #[serde(default)]
    create_time: Value,
    #[serde(default)]
    text: String,
    tif: String,
    #[serde(default, rename = "is_reduce_only")]
    is_reduce_only: Option<bool>,
}

pub(super) fn parse_balance_response(
    item: &AccountItem,
    currency: Option<&str>,
    endpoint_settle: &str,
) -> ExchangeResult<HashMap<String, BalanceInfo>> {
    let key = account_currency(item, endpoint_settle)?;
    if let Some(want) = currency {
        if !want.eq_ignore_ascii_case(&key) {
            return Ok(HashMap::new());
        }
    }

    let total = gate_account_decimal("total", &item.total, &key)?;
    let available = gate_account_decimal("available", &item.available, &key)?;
    let position_margin = gate_account_decimal("position_margin", &item.position_margin, &key)?;
    let order_margin = gate_account_decimal("order_margin", &item.order_margin, &key)?;
    let unrealized_pnl = gate_account_decimal("unrealised_pnl", &item.unrealised_pnl, &key)?;

    let mut out = HashMap::new();
    out.insert(
        key.clone(),
        BalanceInfo {
            currency: key,
            total,
            available,
            frozen: position_margin + order_margin,
            unrealized_pnl,
        },
    );
    Ok(out)
}

pub(super) fn parse_account_read(
    item: &AccountItem,
    currency: Option<&str>,
    endpoint_settle: &str,
    observed_at_ms: i64,
) -> ExchangeResult<VenueAccountRead> {
    let balances = parse_balance_response(item, currency, endpoint_settle)?;
    let summary = parse_account_summary(item, endpoint_settle, observed_at_ms)?;
    Ok(VenueAccountRead {
        balances: venue_balance_rows(NAME, balances),
        summaries: vec![summary],
        asset_valuations: Vec::new(),
        issues: Vec::new(),
    })
}

fn parse_account_summary(
    item: &AccountItem,
    endpoint_settle: &str,
    observed_at_ms: i64,
) -> ExchangeResult<VenueAccountSummary> {
    let currency = account_currency(item, endpoint_settle)?;
    let total = gate_account_decimal("total", &item.total, &currency)?;
    let unrealized_pnl = gate_account_decimal("unrealised_pnl", &item.unrealised_pnl, &currency)?;
    let total_equity_usd = total + unrealized_pnl;
    if !total_equity_usd.is_finite() || total_equity_usd < 0.0 {
        return Err(gate_parse(format!(
            "gate account {currency} has invalid total equity {total_equity_usd}"
        )));
    }
    let total_available_balance_usd =
        gate_account_decimal("available", &item.available, &currency)?;
    let position_margin =
        gate_account_decimal("position_margin", &item.position_margin, &currency)?;
    let order_margin = gate_account_decimal("order_margin", &item.order_margin, &currency)?;
    let total_initial_margin_usd = position_margin + order_margin;
    let total_maintenance_margin_usd = if item.cross_maintenance_margin.trim().is_empty() {
        0.0
    } else {
        gate_account_decimal(
            "cross_maintenance_margin",
            &item.cross_maintenance_margin,
            &currency,
        )?
    };
    Ok(VenueAccountSummary {
        venue: NAME.to_owned(),
        account_type: "usdt_futures".to_owned(),
        equity_scope: AccountEquityScope::Perpetuals,
        total_equity_usd,
        total_available_balance_usd,
        withdrawable_balance_usd: Some(total_available_balance_usd),
        total_initial_margin_usd,
        total_maintenance_margin_usd,
        account_im_rate: account_ratio(total_initial_margin_usd, total_equity_usd),
        account_mm_rate: account_ratio(total_maintenance_margin_usd, total_equity_usd),
        source: "gate.GET /api/v4/futures/usdt/accounts".to_owned(),
        observed_at_ms,
        freshness_ms: Some(0),
        problem: None,
    })
}

fn account_ratio(value: f64, equity: f64) -> f64 {
    if equity > 0.0 {
        value / equity
    } else {
        0.0
    }
}

pub(super) fn parse_account_position_mode(item: &AccountItem) -> ExchangeResult<String> {
    let mode = item.position_mode.trim().to_ascii_lowercase();
    match mode.as_str() {
        "single" | "dual" | "dual_plus" => Ok(mode),
        _ => Err(gate_parse(format!(
            "gate account has unsupported position_mode {:?}",
            item.position_mode
        ))),
    }
}

fn account_currency(item: &AccountItem, endpoint_settle: &str) -> ExchangeResult<String> {
    let currency = item.currency.trim();
    if !currency.is_empty() {
        return Ok(currency.to_owned());
    }
    let settle = endpoint_settle.trim();
    if settle.is_empty() {
        return Err(gate_parse(
            "gate account currency missing and endpoint settle unavailable".to_owned(),
        ));
    }
    Ok(settle.to_ascii_uppercase())
}

pub(super) fn parse_positions<F>(
    rows: &[PositionRow],
    target: Option<&str>,
    mut contract_unit: F,
) -> ExchangeResult<Vec<PositionInfo>>
where
    F: FnMut(&str) -> ExchangeResult<f64>,
{
    let mut positions = Vec::new();
    for row in rows.iter().filter(|row| match target {
        Some(want) => row.contract == want,
        None => true,
    }) {
        if row.size == 0.0 {
            continue;
        }
        if let Some(position) = parse_position(row, contract_unit(&row.contract)?)? {
            positions.push(position);
        }
    }
    Ok(positions)
}

#[cfg(test)]
pub(super) fn parse_open_orders(rows: &[OpenOrderItem]) -> ExchangeResult<Vec<OrderInfo>> {
    rows.iter().map(parse_open_order).collect()
}

pub(super) fn open_order_contract(order: &OpenOrderItem) -> ExchangeResult<&str> {
    gate_order_contract(order)
}

pub(super) fn parse_open_order_with_contract_unit(
    order: &OpenOrderItem,
    contract_unit: f64,
) -> ExchangeResult<OrderInfo> {
    if !contract_unit.is_finite() || contract_unit <= 0.0 {
        return Err(gate_parse(format!(
            "gate order {} {} has invalid contract unit {contract_unit}",
            order.id, order.contract
        )));
    }
    let mut parsed = parse_open_order(order)?;
    parsed.quantity *= contract_unit;
    parsed.filled_quantity *= contract_unit;
    if !parsed.quantity.is_finite() || !parsed.filled_quantity.is_finite() {
        return Err(gate_parse(format!(
            "gate order {} {} contract quantity overflow",
            order.id, order.contract
        )));
    }
    Ok(parsed)
}

pub(super) fn open_order_text_matches(order: &OpenOrderItem, text: &str) -> bool {
    order.text == text
}

pub(super) fn parse_open_order(order: &OpenOrderItem) -> ExchangeResult<OrderInfo> {
    let contract = gate_order_contract(order)?;
    let side = gate_order_side(order)?;
    let quantity = gate_order_quantity(order);
    let left = gate_order_left(order, quantity)?;
    let price = gate_parse_decimal("price", &order.price, order.id, &order.contract)?;
    let order_type = gate_order_type(order, price)?;
    let status = gate_order_status(order, left, quantity)?;
    let filled = quantity - left;
    let filled_price =
        gate_parse_decimal("fill_price", &order.fill_price, order.id, &order.contract)?;
    let created_at = gate_order_created_at(order)?;
    Ok(OrderInfo {
        execution_style: None,
        venue_time_in_force: Some(order.tif.clone()),
        client_order_id: client_order_id_from_str(&order.text),
        reduce_only: order.is_reduce_only,
        order_id: order.id.to_string(),
        symbol: strip_common_suffixes(contract),
        exchange: NAME.into(),
        side,
        order_type,
        status,
        quantity,
        price,
        filled_quantity: filled,
        filled_price,
        fees: 0.0,
        created_at,
    })
}

fn gate_order_contract(order: &OpenOrderItem) -> ExchangeResult<&str> {
    if order.id <= 0 {
        return Err(gate_parse(format!(
            "gate order {} has invalid id",
            order.id
        )));
    }
    let contract = order.contract.trim();
    if contract.is_empty() {
        return Err(gate_parse(format!(
            "gate order {} has empty contract",
            order.id
        )));
    }
    Ok(contract)
}

fn gate_order_side(order: &OpenOrderItem) -> ExchangeResult<OrderSide> {
    if order.size < 0.0 {
        Ok(OrderSide::Sell)
    } else if order.size > 0.0 {
        Ok(OrderSide::Buy)
    } else {
        Err(gate_parse(format!(
            "gate order {} {} has zero size",
            order.id, order.contract
        )))
    }
}

fn gate_order_quantity(order: &OpenOrderItem) -> f64 {
    order.size.abs()
}

fn gate_order_left(order: &OpenOrderItem, quantity: f64) -> ExchangeResult<f64> {
    let left = order.left.abs();
    if left <= quantity {
        Ok(left)
    } else {
        Err(gate_parse(format!(
            "gate order {} {} left {} exceeds size {}",
            order.id, order.contract, order.left, order.size
        )))
    }
}

fn gate_order_type(order: &OpenOrderItem, price: f64) -> ExchangeResult<OrderType> {
    match order.tif.to_ascii_lowercase().as_str() {
        "poc" => Ok(OrderType::PostOnly),
        "ioc" if price == 0.0 => Ok(OrderType::Market),
        "gtc" | "ioc" | "fok" => Ok(OrderType::Limit),
        tif => Err(gate_parse(format!(
            "gate order {} {} has unknown tif {tif}",
            order.id, order.contract
        ))),
    }
}

fn gate_order_status(
    order: &OpenOrderItem,
    left: f64,
    quantity: f64,
) -> ExchangeResult<OrderStatus> {
    match order.status.to_ascii_lowercase().as_str() {
        "open" if left < quantity => Ok(OrderStatus::PartiallyFilled),
        "open" => Ok(OrderStatus::Open),
        "finished" => gate_finished_order_status(order, left),
        "cancelled" | "canceled" => Ok(OrderStatus::Canceled),
        status => Err(gate_parse(format!(
            "gate order {} {} has unknown status {status}",
            order.id, order.contract
        ))),
    }
}

fn gate_finished_order_status(order: &OpenOrderItem, left: f64) -> ExchangeResult<OrderStatus> {
    match order.finish_as.trim().to_ascii_lowercase().as_str() {
        "filled" if left == 0.0 => Ok(OrderStatus::Filled),
        "filled" => Err(gate_parse(format!(
            "gate order {} {} reports finish_as filled with left {}",
            order.id, order.contract, order.left
        ))),
        "cancelled" | "canceled" | "liquidated" | "ioc" | "auto_deleveraged" | "reduce_only"
        | "position_closed" | "reduce_out" | "stp" => Ok(OrderStatus::Canceled),
        finish_as => Err(gate_parse(format!(
            "gate order {} {} has unknown finish_as {finish_as:?}",
            order.id, order.contract
        ))),
    }
}

fn gate_order_created_at(order: &OpenOrderItem) -> ExchangeResult<chrono::DateTime<chrono::Utc>> {
    let created_at_ms = if order.create_time_ms > 0 {
        order.create_time_ms
    } else {
        gate_create_time_ms(order)?
    };
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(created_at_ms).ok_or_else(|| {
        gate_parse(format!(
            "gate order {} {} has invalid creation timestamp {}",
            order.id, order.contract, created_at_ms
        ))
    })
}

fn gate_create_time_ms(order: &OpenOrderItem) -> ExchangeResult<i64> {
    let seconds = match &order.create_time {
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.parse::<f64>().ok(),
        _ => None,
    }
    .filter(|value| value.is_finite() && *value > 0.0)
    .ok_or_else(|| {
        gate_parse(format!(
            "gate order {} {} is missing a valid create_time_ms/create_time",
            order.id, order.contract
        ))
    })?;
    let milliseconds = seconds * 1_000.0;
    if !milliseconds.is_finite() || milliseconds >= i64::MAX as f64 {
        return Err(gate_parse(format!(
            "gate order {} {} has out-of-range create_time {}",
            order.id, order.contract, order.create_time
        )));
    }
    Ok(milliseconds.round() as i64)
}

fn gate_parse_decimal(
    field: &str,
    value: &str,
    order_id: i64,
    contract: &str,
) -> ExchangeResult<f64> {
    let identity = format!("{order_id} {contract}");
    let parsed = gate_finite_decimal("order", field, value, &identity)?;
    if parsed >= 0.0 {
        Ok(parsed)
    } else {
        Err(gate_parse(format!(
            "gate order {order_id} {contract} has negative {field} {parsed}"
        )))
    }
}

fn gate_parse(message: String) -> ExchangeError {
    ExchangeError::Parse(message)
}

fn parse_position(row: &PositionRow, contract_unit: f64) -> ExchangeResult<Option<PositionInfo>> {
    if row.size == 0.0 {
        return Ok(None);
    }
    if !contract_unit.is_finite() || contract_unit <= 0.0 {
        return Err(gate_parse(format!(
            "gate position {} has invalid contract unit {contract_unit}",
            row.contract
        )));
    }
    let entry_price = gate_position_price("entry_price", &row.entry_price, row)?;
    let mark_price = gate_position_price("mark_price", &row.mark_price, row)?;
    let unrealized_pnl = gate_position_decimal("unrealised_pnl", &row.unrealised_pnl, row)?;
    let leverage = gate_position_non_negative("leverage", &row.leverage, row)?;
    let margin = gate_position_non_negative("margin", &row.margin, row)?;
    let maintenance_margin_ratio = gate_position_maintenance_rate(&row.maintenance_rate, row)?;
    let liquidation_price = if row.liq_price.trim().is_empty() {
        None
    } else {
        gate_liquidation_price(&row.liq_price, row)?
    };
    let liquidation_distance_pct = gate_liquidation_distance_pct(mark_price, liquidation_price);
    let position_mode = gate_position_mode(&row.mode, row)?;
    let margin_mode = if leverage == 0.0 { "cross" } else { "isolated" };
    Ok(Some(PositionInfo {
        symbol: strip_common_suffixes(&row.contract),
        exchange: NAME.into(),
        side: if row.size > 0.0 { "long" } else { "short" }.to_owned(),
        quantity: row.size.abs() * contract_unit,
        entry_price,
        mark_price,
        unrealized_pnl,
        leverage,
        liquidation_price,
        liquidation_distance_pct,
        next_funding_ms: None,
        paired_with: None,
        margin,
        maintenance_margin_ratio,
        position_mode,
        margin_mode: Some(margin_mode.to_owned()),
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }))
}

/// Gate `Position.mode` is venue-native: `single` or `dual_long` / `dual_short`.
fn gate_position_mode(mode: &str, row: &PositionRow) -> ExchangeResult<Option<String>> {
    match mode.trim().to_ascii_lowercase().as_str() {
        "" => Ok(None),
        value @ ("single" | "dual_long" | "dual_short") => Ok(Some(value.to_owned())),
        other => Err(gate_parse(format!(
            "gate position {} has invalid mode {other}",
            row.contract
        ))),
    }
}

fn gate_account_decimal(field: &str, value: &str, currency: &str) -> ExchangeResult<f64> {
    gate_finite_decimal("account", field, value, currency)
}

fn gate_position_decimal(field: &str, value: &str, row: &PositionRow) -> ExchangeResult<f64> {
    gate_finite_decimal("position", field, value, &row.contract)
}

fn gate_position_price(field: &str, value: &str, row: &PositionRow) -> ExchangeResult<f64> {
    let parsed = gate_position_decimal(field, value, row)?;
    if parsed >= 0.0 {
        Ok(parsed)
    } else {
        Err(gate_parse(format!(
            "gate position {} has negative {field} {parsed}",
            row.contract
        )))
    }
}

fn gate_position_non_negative(field: &str, value: &str, row: &PositionRow) -> ExchangeResult<f64> {
    gate_position_price(field, value, row)
}

fn gate_position_maintenance_rate(value: &str, row: &PositionRow) -> ExchangeResult<f64> {
    if value.trim().is_empty() {
        // Gate omits an effective rate for some cross/portfolio positions. The
        // shared f64 field uses zero as its established unknown sentinel; API
        // field-quality projection keeps it visibly non-actual downstream.
        return Ok(0.0);
    }
    gate_position_non_negative("maintenance_rate", value, row)
}

fn gate_liquidation_price(value: &str, row: &PositionRow) -> ExchangeResult<Option<f64>> {
    let parsed = gate_position_price("liq_price", value, row)?;
    Ok((parsed > 0.0).then_some(parsed))
}

fn gate_liquidation_distance_pct(mark_price: f64, liquidation_price: Option<f64>) -> Option<f64> {
    let liquidation_price = liquidation_price.filter(|_| mark_price > 0.0)?;
    let distance = ((mark_price - liquidation_price).abs() / mark_price) * 100.0;
    distance.is_finite().then_some(distance)
}

fn gate_finite_decimal(
    scope: &str,
    field: &str,
    value: &str,
    identity: &str,
) -> ExchangeResult<f64> {
    let parsed = value.parse::<f64>().map_err(|source| {
        gate_parse(format!(
            "gate {scope} {identity} has invalid {field} {value:?}: {source}"
        ))
    })?;
    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(gate_parse(format!(
            "gate {scope} {identity} has non-finite {field} {value:?}"
        )))
    }
}

fn de_ms_to_i64<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;

    let value = Value::deserialize(deserializer)?;
    ms_value_to_i64(value).map_err(D::Error::custom)
}

fn ms_value_to_i64(value: Value) -> Result<i64, &'static str> {
    match value {
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                return Ok(value);
            }
            if let Some(value) = number.as_u64() {
                return i64::try_from(value).map_err(|_| "ms timestamp overflow");
            }
            let Some(value) = number.as_f64() else {
                return Err("ms timestamp not parseable");
            };
            if value.is_finite()
                && value.fract() == 0.0
                && value >= i64::MIN as f64
                && value < i64::MAX as f64
            {
                Ok(value as i64)
            } else {
                Err("ms timestamp must be a finite whole-number integer")
            }
        }
        Value::String(value) => value
            .parse::<i64>()
            .map_err(|_| "ms timestamp not parseable"),
        Value::Null => Err("ms timestamp missing"),
        _ => Err("expected integer, float, or string timestamp in milliseconds"),
    }
}

fn de_f64_from_string_or_number<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;

    let value = Value::deserialize(deserializer)?;
    let parsed = match value {
        Value::Number(number) => number.as_f64(),
        Value::String(value) => value.parse::<f64>().ok(),
        _ => None,
    }
    .ok_or_else(|| D::Error::custom("expected finite number or decimal string"))?;
    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(D::Error::custom("expected finite decimal size"))
    }
}

#[cfg(test)]
#[path = "gate_private_data_tests.rs"]
mod tests;
