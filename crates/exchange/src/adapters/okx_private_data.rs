//! OKX private read response parsing.
//!
//! Official OKX V5 docs checked before moving these DTOs:
//! - GET /api/v5/account/balance
//! - GET /api/v5/account/positions
//! - GET /api/v5/trade/orders-pending

use crate::adapter::{client_order_id_from_str, strip_common_suffixes};
use crate::adapters::okx_read_parse::{
    parse_optional_reduce_only, parse_required_number, parse_timestamp_ms,
};
use crate::error::{ExchangeError, ExchangeResult};
use serde::Deserialize;
use shared_types::{BalanceInfo, OrderInfo, OrderSide, OrderStatus, OrderType, PositionInfo};
use std::collections::HashMap;

const NAME: &str = "okx";

#[derive(Debug, Deserialize)]
pub(super) struct AccountBalanceItem {
    details: Vec<BalanceDetail>,
}

#[derive(Debug, Deserialize)]
struct BalanceDetail {
    ccy: String,
    eq: String,
    #[serde(rename = "availBal")]
    avail_bal: String,
    #[serde(rename = "frozenBal")]
    frozen_bal: String,
    upl: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct PositionRow {
    #[serde(rename = "instId")]
    inst_id: String,
    pos: String,
    #[serde(rename = "avgPx")]
    avg_px: String,
    #[serde(rename = "markPx")]
    mark_px: String,
    upl: String,
    lever: String,
    #[serde(rename = "liqPx")]
    liq_px: String,
    imr: String,
    #[serde(rename = "posSide")]
    pos_side: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenOrderItem {
    #[serde(rename = "instId")]
    inst_id: String,
    #[serde(rename = "ordId")]
    ord_id: String,
    state: String,
    #[serde(rename = "ordType")]
    ord_type: String,
    side: String,
    px: String,
    sz: String,
    #[serde(rename = "accFillSz")]
    acc_fill_sz: String,
    #[serde(rename = "avgPx")]
    avg_px: String,
    #[serde(rename = "cTime")]
    c_time: String,
    #[serde(rename = "clOrdId")]
    cl_ord_id: String,
    #[serde(rename = "reduceOnly")]
    reduce_only: String,
    fee: String,
}

pub(super) fn parse_balance_response(
    items: Vec<AccountBalanceItem>,
    currency: Option<&str>,
) -> ExchangeResult<HashMap<String, BalanceInfo>> {
    let [account]: [AccountBalanceItem; 1] =
        items.try_into().map_err(|items: Vec<AccountBalanceItem>| {
            ExchangeError::Parse(format!(
                "okx balance expected one account row, got {}",
                items.len()
            ))
        })?;
    parse_balance_details(account, currency)
}

pub(super) fn parse_positions(
    items: &[PositionRow],
    target: Option<&str>,
) -> ExchangeResult<Vec<PositionInfo>> {
    items
        .iter()
        .map(|row| {
            let position = parse_position(row)?;
            Ok(target
                .is_none_or(|want| row.inst_id == want)
                .then_some(position)
                .flatten())
        })
        .filter_map(Result::transpose)
        .collect()
}

pub(super) fn parse_open_orders(items: Vec<OpenOrderItem>) -> ExchangeResult<Vec<OrderInfo>> {
    items
        .into_iter()
        .map(|item| parse_open_order(&item))
        .collect()
}

fn parse_balance_details(
    account: AccountBalanceItem,
    currency: Option<&str>,
) -> ExchangeResult<HashMap<String, BalanceInfo>> {
    let mut out = HashMap::new();
    for detail in account.details {
        let ccy = required_text("balance", "ccy", &detail.ccy)?;
        let scope = format!("balance {ccy}");
        let total = parse_required_number(&scope, "eq", &detail.eq)?;
        let available = parse_required_number(&scope, "availBal", &detail.avail_bal)?;
        let frozen = parse_required_number(&scope, "frozenBal", &detail.frozen_bal)?;
        let unrealized_pnl = parse_required_number(&scope, "upl", &detail.upl)?;
        if currency.is_some_and(|want| !want.eq_ignore_ascii_case(ccy)) {
            continue;
        }
        out.insert(
            ccy.to_owned(),
            BalanceInfo {
                currency: ccy.to_owned(),
                total,
                available,
                frozen,
                unrealized_pnl,
            },
        );
    }
    Ok(out)
}

fn parse_position(row: &PositionRow) -> ExchangeResult<Option<PositionInfo>> {
    let inst_id = required_text("position", "instId", &row.inst_id)?;
    let scope = format!("position {inst_id}");
    let qty = parse_required_number(&scope, "pos", &row.pos)?;
    let side = parse_position_side(&scope, &row.pos_side, qty)?;
    let entry_price = parse_non_negative_number(&scope, "avgPx", &row.avg_px)?;
    let mark_price = parse_non_negative_number(&scope, "markPx", &row.mark_px)?;
    let unrealized_pnl = parse_required_number(&scope, "upl", &row.upl)?;
    let leverage = parse_positive_number(&scope, "lever", &row.lever)?;
    let liquidation_price = parse_optional_positive_number(&scope, "liqPx", &row.liq_px)?;
    let margin = parse_non_negative_number(&scope, "imr", &row.imr)?;
    if qty == 0.0 {
        return Ok(None);
    }
    if entry_price <= 0.0 {
        return Err(parse_error(&scope, "avgPx", &row.avg_px));
    }
    if mark_price <= 0.0 {
        return Err(parse_error(&scope, "markPx", &row.mark_px));
    }
    Ok(Some(PositionInfo {
        symbol: strip_common_suffixes(inst_id),
        exchange: NAME.into(),
        side,
        quantity: qty.abs(),
        entry_price,
        mark_price,
        unrealized_pnl,
        leverage,
        liquidation_price,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin,
        maintenance_margin_ratio: 0.0,
        position_mode: None,
        margin_mode: None,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }))
}

fn parse_open_order(order: &OpenOrderItem) -> ExchangeResult<OrderInfo> {
    let order_id = required_text("order", "ordId", &order.ord_id)?;
    let symbol = required_text("order", "instId", &order.inst_id)?;
    let scope = format!("order {order_id}");
    let order_type = parse_order_type(&scope, &order.ord_type)?;
    let quantity = parse_positive_number(&scope, "sz", &order.sz)?;
    let filled_quantity = parse_non_negative_number(&scope, "accFillSz", &order.acc_fill_sz)?;
    if filled_quantity > quantity {
        return Err(parse_error(&scope, "accFillSz", &order.acc_fill_sz));
    }
    let price = parse_order_price(&scope, order_type, &order.px)?;
    let filled_price = parse_filled_price(&scope, &order.avg_px, filled_quantity)?;
    Ok(OrderInfo {
        execution_style: None,
        venue_time_in_force: Some(order.ord_type.clone()),
        client_order_id: client_order_id_from_str(&order.cl_ord_id),
        reduce_only: parse_optional_reduce_only(&scope, &order.reduce_only)?,
        order_id: order_id.to_owned(),
        symbol: strip_common_suffixes(symbol),
        exchange: NAME.into(),
        side: parse_order_side(&scope, &order.side)?,
        order_type,
        status: parse_order_status(&scope, &order.state)?,
        quantity,
        price,
        filled_quantity,
        filled_price,
        fees: parse_required_number(&scope, "fee", &order.fee)?,
        created_at: parse_timestamp_ms(&scope, "cTime", &order.c_time)?,
    })
}

fn parse_order_price(scope: &str, order_type: OrderType, raw: &str) -> ExchangeResult<f64> {
    match order_type {
        OrderType::Market => parse_blank_as_zero(scope, "px", raw),
        OrderType::Limit | OrderType::PostOnly => parse_positive_number(scope, "px", raw),
    }
}

fn parse_order_side(scope: &str, raw: &str) -> ExchangeResult<OrderSide> {
    match raw {
        "buy" => Ok(OrderSide::Buy),
        "sell" => Ok(OrderSide::Sell),
        _ => Err(parse_error(scope, "side", raw)),
    }
}

fn parse_order_type(scope: &str, raw: &str) -> ExchangeResult<OrderType> {
    match raw {
        "market" | "optimal_limit_ioc" => Ok(OrderType::Market),
        "post_only" => Ok(OrderType::PostOnly),
        "limit" | "ioc" | "fok" => Ok(OrderType::Limit),
        _ => Err(parse_error(scope, "ordType", raw)),
    }
}

fn parse_order_status(scope: &str, raw: &str) -> ExchangeResult<OrderStatus> {
    match raw {
        "live" => Ok(OrderStatus::Open),
        "partially_filled" => Ok(OrderStatus::PartiallyFilled),
        "filled" => Ok(OrderStatus::Filled),
        "canceled" | "mmp_canceled" => Ok(OrderStatus::Canceled),
        "rejected" => Ok(OrderStatus::Rejected),
        _ => Err(parse_error(scope, "state", raw)),
    }
}

fn parse_position_side(scope: &str, raw: &str, quantity: f64) -> ExchangeResult<String> {
    match raw {
        "long" if quantity > 0.0 => Ok("long".to_owned()),
        "short" if quantity > 0.0 => Ok("short".to_owned()),
        "net" if quantity < 0.0 => Ok("short".to_owned()),
        "net" => Ok("long".to_owned()),
        _ => Err(parse_error(scope, "posSide", raw)),
    }
}

fn parse_positive_number(scope: &str, field: &str, raw: &str) -> ExchangeResult<f64> {
    let value = parse_required_number(scope, field, raw)?;
    if value <= 0.0 {
        return Err(parse_error(scope, field, raw));
    }
    Ok(value)
}

fn parse_non_negative_number(scope: &str, field: &str, raw: &str) -> ExchangeResult<f64> {
    let value = parse_required_number(scope, field, raw)?;
    if value < 0.0 {
        return Err(parse_error(scope, field, raw));
    }
    Ok(value)
}

fn parse_optional_positive_number(
    scope: &str,
    field: &str,
    raw: &str,
) -> ExchangeResult<Option<f64>> {
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let value = parse_non_negative_number(scope, field, raw)?;
    Ok((value > 0.0).then_some(value))
}

fn parse_blank_as_zero(scope: &str, field: &str, raw: &str) -> ExchangeResult<f64> {
    if raw.trim().is_empty() {
        return Ok(0.0);
    }
    parse_non_negative_number(scope, field, raw)
}

fn parse_filled_price(scope: &str, raw: &str, filled_quantity: f64) -> ExchangeResult<f64> {
    if raw.trim().is_empty() {
        return (filled_quantity == 0.0)
            .then_some(0.0)
            .ok_or_else(|| parse_error(scope, "avgPx", raw));
    }
    let price = parse_non_negative_number(scope, "avgPx", raw)?;
    if filled_quantity > 0.0 && price <= 0.0 {
        return Err(parse_error(scope, "avgPx", raw));
    }
    Ok(price)
}

fn required_text<'a>(scope: &str, field: &str, raw: &'a str) -> ExchangeResult<&'a str> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(parse_error(scope, field, raw));
    }
    Ok(value)
}

fn parse_error(scope: &str, field: &str, value: &str) -> ExchangeError {
    ExchangeError::Parse(format!("okx {scope} field {field} invalid: {value}"))
}

#[cfg(test)]
#[path = "okx_private_data_tests.rs"]
mod tests;
