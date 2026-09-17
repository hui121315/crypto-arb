//! Gate Spot WebSocket order request compiler and order projection.
//!
//! Official schema: <https://www.gate.com/docs/developers/apiv4/ws/en/#spot-account-trade>

use super::gate_trade_data::gate_text;
use super::spot_order_contract::CompiledSpotOrder;
use crate::adapter::strip_common_suffixes;
use crate::error::{ExchangeError, ExchangeResult};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use shared_types::{
    CancelOrderRequest, LiveOrderState, OrderAck, OrderInfo, OrderIntent, OrderSide, OrderStatus,
    OrderType, TimeInForce, VenueOrderIdentityUpdate,
};

const NAME: &str = "gate";

pub(super) fn place_params(
    intent: &OrderIntent,
    compiled: &CompiledSpotOrder,
) -> ExchangeResult<Value> {
    let (order_type, price, time_in_force) = match intent.order_type {
        OrderType::Limit => (
            "limit",
            compiled.price.clone(),
            Some(gate_spot_tif(intent.time_in_force)),
        ),
        OrderType::PostOnly => ("limit", compiled.price.clone(), Some("poc")),
        OrderType::Market => {
            let price = intent
                .price
                .filter(|value| value.is_finite() && *value > 0.0);
            let price = price.ok_or_else(|| {
                validation_error(
                    "gate spot market intent requires a protection price for base-quantity IOC"
                        .to_owned(),
                )
            })?;
            (
                "limit",
                Some(super::spot_order_contract::decimal_param(
                    NAME, "price", price,
                )?),
                Some("ioc"),
            )
        }
    };
    Ok(json!({
        "text": gate_text(&intent.client_order_id)?,
        "currency_pair": compiled.native_symbol,
        "type": order_type,
        "account": "spot",
        "side": gate_side(intent.side),
        "amount": compiled.quantity,
        "price": price,
        "time_in_force": time_in_force,
        "action_mode": "ACK",
    }))
}

pub(super) fn cancel_params(
    request: &CancelOrderRequest,
    native_symbol: &str,
) -> ExchangeResult<Value> {
    let order_id = request
        .exchange_order_id
        .clone()
        .unwrap_or(gate_text(&request.client_order_id)?);
    Ok(json!({
        "order_id": order_id,
        "currency_pair": native_symbol,
        "account": "spot",
        "action_mode": "ACK",
    }))
}

pub(super) fn status_params(order_id: &str, native_symbol: &str) -> ExchangeResult<Value> {
    let order_id = order_id.trim();
    if order_id.is_empty() {
        return Err(validation_error("gate spot order id is empty".to_owned()));
    }
    Ok(json!({
        "order_id": order_id,
        "currency_pair": native_symbol,
        "account": "spot",
    }))
}

pub(super) fn ack_from_row(
    internal_order_id: String,
    public_client_order_id: String,
    row: &GateSpotOrderRow,
    requested_state: LiveOrderState,
) -> ExchangeResult<OrderAck> {
    let venue_client_order_id = gate_text(&public_client_order_id)?;
    let exchange_order_id = non_empty(&row.id);
    let state = if row.status.is_empty() {
        requested_state
    } else {
        live_state(order_status(row))
    };
    Ok(OrderAck {
        internal_order_id,
        exchange_order_id: exchange_order_id.clone(),
        client_order_id: public_client_order_id.clone(),
        identity_update: VenueOrderIdentityUpdate::from_ids(
            public_client_order_id,
            non_empty(&row.text).unwrap_or(venue_client_order_id),
            exchange_order_id,
        ),
        state,
        accepted_at_ms: common::time::now_ms(),
        message: None,
        filled_quantity: filled_quantity(row),
        filled_price: parse_optional(&row.avg_deal_price),
        filled_fee: None,
    })
}

pub(super) fn order_info(row: GateSpotOrderRow) -> ExchangeResult<OrderInfo> {
    let quantity = parse_number("amount", &row.amount)?;
    let left = parse_number("left", &row.left)?;
    let filled_quantity = (quantity - left).max(0.0);
    let created_at = timestamp(row.create_time_ms, &row.create_time);
    let status = order_status(&row);
    Ok(OrderInfo {
        order_id: row.id,
        symbol: strip_common_suffixes(&row.currency_pair),
        exchange: NAME.to_owned(),
        side: match row.side.as_str() {
            "buy" => OrderSide::Buy,
            "sell" => OrderSide::Sell,
            value => return Err(ExchangeError::Parse(format!("gate spot side: {value}"))),
        },
        order_type: if row.order_type == "market" {
            OrderType::Market
        } else if row.time_in_force == "poc" {
            OrderType::PostOnly
        } else {
            OrderType::Limit
        },
        status,
        quantity,
        price: parse_number("price", &row.price)?,
        filled_quantity,
        filled_price: parse_number("avg_deal_price", &row.avg_deal_price)?,
        fees: 0.0,
        created_at,
        execution_style: None,
        venue_time_in_force: non_empty(&row.time_in_force),
        client_order_id: non_empty(&row.text),
        reduce_only: Some(false),
    })
}

fn gate_spot_tif(value: TimeInForce) -> &'static str {
    match value {
        TimeInForce::Gtc => "gtc",
        TimeInForce::Ioc => "ioc",
        TimeInForce::Fok => "fok",
        TimeInForce::Gtx => "poc",
    }
}

fn gate_side(side: OrderSide) -> &'static str {
    match side {
        OrderSide::Buy => "buy",
        OrderSide::Sell => "sell",
    }
}

fn order_status(row: &GateSpotOrderRow) -> OrderStatus {
    match (row.status.as_str(), row.finish_as.as_str()) {
        ("open", _)
            if parse_optional(&row.left).is_some_and(|left| {
                parse_optional(&row.amount).is_some_and(|amount| left < amount)
            }) =>
        {
            OrderStatus::PartiallyFilled
        }
        ("open", _) => OrderStatus::Open,
        ("closed", "filled") => OrderStatus::Filled,
        ("closed", "cancelled" | "canceled") | ("cancelled" | "canceled", _) => {
            OrderStatus::Canceled
        }
        ("closed", "expired") => OrderStatus::Expired,
        ("closed", "failed" | "rejected") => OrderStatus::Rejected,
        _ => OrderStatus::Pending,
    }
}

fn live_state(status: OrderStatus) -> LiveOrderState {
    match status {
        OrderStatus::Pending | OrderStatus::Open => LiveOrderState::Accepted,
        OrderStatus::PartiallyFilled => LiveOrderState::PartiallyFilled,
        OrderStatus::Filled => LiveOrderState::Filled,
        OrderStatus::Canceled => LiveOrderState::Cancelled,
        OrderStatus::Rejected => LiveOrderState::Rejected,
        OrderStatus::Expired => LiveOrderState::Failed,
    }
}

fn timestamp(milliseconds: i64, seconds: &str) -> DateTime<Utc> {
    let millis = if milliseconds > 0 {
        milliseconds
    } else {
        seconds
            .parse::<i64>()
            .unwrap_or_default()
            .saturating_mul(1_000)
    };
    DateTime::from_timestamp_millis(millis).unwrap_or_else(Utc::now)
}

fn parse_number(field: &str, value: &str) -> ExchangeResult<f64> {
    if value.is_empty() {
        return Ok(0.0);
    }
    value
        .parse::<f64>()
        .map_err(|error| ExchangeError::Parse(format!("gate spot {field}: {error}")))
}

fn parse_optional(value: &str) -> Option<f64> {
    value.parse::<f64>().ok()
}

fn filled_quantity(row: &GateSpotOrderRow) -> Option<f64> {
    let quantity = parse_optional(&row.amount)?;
    let left = parse_optional(&row.left)?;
    Some((quantity - left).max(0.0))
}

fn non_empty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.to_owned())
}

fn validation_error(message: String) -> ExchangeError {
    ExchangeError::Api {
        exchange: NAME.to_owned(),
        code: "validation".to_owned(),
        message,
    }
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct GateSpotOrderRow {
    #[serde(default)]
    pub(super) id: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    currency_pair: String,
    #[serde(default, rename = "type")]
    order_type: String,
    #[serde(default)]
    side: String,
    #[serde(default)]
    amount: String,
    #[serde(default)]
    price: String,
    #[serde(default)]
    left: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    finish_as: String,
    #[serde(default)]
    avg_deal_price: String,
    #[serde(default)]
    time_in_force: String,
    #[serde(default)]
    create_time: String,
    #[serde(default)]
    create_time_ms: i64,
}

#[cfg(test)]
#[path = "gate_spot_trade_data_tests.rs"]
mod tests;
