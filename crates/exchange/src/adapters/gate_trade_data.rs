//! Gate futures order write payload and ack parsing.
//!
//! Official Gate API v4 docs checked before moving these DTOs:
//! - `POST /futures/{settle}/orders`
//! - `DELETE /futures/{settle}/orders/{order_id}`
//! - WebSocket `futures.order_place` / `futures.order_cancel`

use crate::error::{ExchangeError, ExchangeResult};
use common::time::now_ms;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shared_types::{
    CancelOrderRequest, LiveOrderState, OrderAck, OrderIntent, OrderSide, OrderType, TimeInForce,
    VenueOrderIdentityUpdate,
};

const NAME: &str = "gate";

#[derive(Debug, Serialize)]
struct GatePlaceOrderParams {
    contract: String,
    size: String,
    price: String,
    tif: &'static str,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none", rename = "reduce_only")]
    reduce_only: Option<bool>,
}

#[derive(Debug, Serialize)]
struct GateCancelOrderParams {
    #[serde(rename = "order_id")]
    order_id: i64,
}

struct GateOrderShape {
    price: String,
    tif: &'static str,
}

#[derive(Debug, Deserialize)]
pub(super) struct GateOrderAckRow {
    #[serde(default, alias = "id")]
    pub(super) order_id: Option<i64>,
}

pub(super) fn place_order_params(
    intent: &OrderIntent,
    symbol: String,
    contract_unit: f64,
) -> ExchangeResult<Value> {
    let size = gate_order_size(intent, contract_unit)?;
    let shape = gate_order_shape(intent)?;
    serde_json::to_value(GatePlaceOrderParams {
        contract: symbol,
        size,
        price: shape.price,
        tif: shape.tif,
        text: gate_text(&intent.client_order_id)?,
        reduce_only: intent.reduce_only.then_some(true),
    })
    .map_err(|error| ExchangeError::Parse(format!("gate place order params: {error}")))
}

pub(super) fn cancel_order_params(request: &CancelOrderRequest) -> ExchangeResult<Value> {
    let order_id = request
        .exchange_order_id
        .as_deref()
        .ok_or_else(|| validation_error("gate cancel requires exchange_order_id".to_owned()))?
        .parse::<i64>()
        .map_err(|_| validation_error("gate order_id must be numeric".to_owned()))?;
    serde_json::to_value(GateCancelOrderParams { order_id })
        .map_err(|error| ExchangeError::Parse(format!("gate cancel order params: {error}")))
}

pub(super) fn gate_text(client_order_id: &str) -> ExchangeResult<String> {
    crate::client_order_id_policy::required_venue_client_order_id(NAME, client_order_id)
}

pub(super) fn ack_from_row(
    internal_order_id: String,
    public_client_order_id: String,
    venue_client_order_id: String,
    row: &GateOrderAckRow,
    state: LiveOrderState,
    message: Option<String>,
) -> OrderAck {
    let exchange_order_id = row.order_id.map(|id| id.to_string());
    OrderAck {
        internal_order_id,
        exchange_order_id: exchange_order_id.clone(),
        client_order_id: public_client_order_id.clone(),
        identity_update: VenueOrderIdentityUpdate::from_ids(
            public_client_order_id,
            venue_client_order_id,
            exchange_order_id,
        ),
        state,
        accepted_at_ms: now_ms(),
        message,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
    }
}

pub(super) fn validation_error(message: String) -> ExchangeError {
    ExchangeError::Api {
        exchange: NAME.into(),
        code: "validation".into(),
        message,
    }
}

fn gate_order_shape(intent: &OrderIntent) -> ExchangeResult<GateOrderShape> {
    match intent.order_type {
        OrderType::Limit => Ok(GateOrderShape {
            price: required_price(intent)?,
            tif: gate_limit_tif(intent.time_in_force)?,
        }),
        OrderType::PostOnly => Ok(GateOrderShape {
            price: required_price(intent)?,
            tif: "poc",
        }),
        OrderType::Market => Ok(GateOrderShape {
            price: "0".into(),
            tif: "ioc",
        }),
    }
}

fn gate_limit_tif(time_in_force: TimeInForce) -> ExchangeResult<&'static str> {
    match time_in_force {
        TimeInForce::Gtc => Ok("gtc"),
        TimeInForce::Ioc => Ok("ioc"),
        TimeInForce::Fok => Ok("fok"),
        TimeInForce::Gtx => Err(validation_error(
            "gate futures has no gtx tif; use post-only order_type for official poc".to_owned(),
        )),
    }
}

fn required_price(intent: &OrderIntent) -> ExchangeResult<String> {
    let price = intent
        .price
        .ok_or_else(|| validation_error("gate limit order requires price".to_owned()))?;
    positive_number(price, "price")
}

fn gate_order_size(intent: &OrderIntent, contract_unit: f64) -> ExchangeResult<String> {
    let quantity = positive_number_value(intent.quantity, "quantity")?;
    let raw = quantity / contract_unit;
    let rounded = raw.round();
    if (raw - rounded).abs() > 1e-8 || rounded < 1.0 {
        return Err(validation_error(format!(
            "gate quantity {quantity} is not an integer contract count for multiplier {contract_unit}"
        )));
    }
    let contracts = rounded as i64;
    Ok(match intent.side {
        OrderSide::Buy => contracts.to_string(),
        OrderSide::Sell => (-contracts).to_string(),
    })
}

fn positive_number(value: f64, field: &str) -> ExchangeResult<String> {
    let value = positive_number_value(value, field)?;
    let text = format!("{value:.12}");
    Ok(text.trim_end_matches('0').trim_end_matches('.').to_owned())
}

fn positive_number_value(value: f64, field: &str) -> ExchangeResult<f64> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(validation_error(format!(
            "gate invalid positive {field}: {value}"
        )))
    }
}

#[cfg(test)]
#[path = "gate_trade_data_tests.rs"]
mod tests;
