//! KuCoin futures order write payload and ack parsing.
//!
//! Official KuCoin docs checked before moving these DTOs:
//! - POST /api/v1/orders
//! - DELETE /api/v1/orders/client-order/{clientOid}?symbol={symbol}
//! - GET /api/v1/orders/byClientOid?clientOid={clientOid}

use crate::error::{ExchangeError, ExchangeResult};
use common::time::now_ms;
use serde::{Deserialize, Serialize};
use shared_types::{
    LiveOrderState, MarginMode, OrderAck, OrderInfo, OrderIntent, OrderSide, OrderStatus,
    OrderType, TimeInForce, VenueOrderIdentityUpdate,
};

const NAME: &str = "kucoin";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct KucoinPlaceOrderBody {
    client_oid: String,
    side: &'static str,
    symbol: String,
    #[serde(rename = "type")]
    order_type: &'static str,
    size: i64,
    leverage: u32,
    #[serde(rename = "marginMode")]
    margin_mode: &'static str,
    #[serde(rename = "positionSide")]
    position_side: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    price: Option<String>,
    #[serde(rename = "timeInForce", skip_serializing_if = "Option::is_none")]
    time_in_force: Option<&'static str>,
    #[serde(rename = "postOnly", skip_serializing_if = "Option::is_none")]
    post_only: Option<bool>,
    #[serde(rename = "reduceOnly", skip_serializing_if = "Option::is_none")]
    reduce_only: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KucoinOrderAckRow {
    order_id: String,
    client_oid: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KucoinCancelRow {
    #[serde(default, rename = "cancelledOrderIds")]
    cancelled_order_ids: Vec<String>,
    #[serde(default)]
    client_oid: Option<String>,
}

struct KucoinOrderShape {
    order_type: &'static str,
    price: Option<String>,
    time_in_force: Option<&'static str>,
    post_only: Option<bool>,
}

pub(super) fn place_order_body_json(
    intent: &OrderIntent,
    symbol: String,
    contract_unit: f64,
    position_side: &str,
) -> ExchangeResult<String> {
    let body = build_place_order_body(intent, symbol, contract_unit, position_side)?;
    serde_json::to_string(&body)
        .map_err(|error| ExchangeError::Parse(format!("kucoin place order body: {error}")))
}

pub(super) fn safe_cancel_probe_path(client_order_id: &str, symbol: &str) -> String {
    let client_oid = path_segment(client_order_id);
    format!("/api/v1/orders/client-order/{client_oid}?symbol={symbol}")
}

pub(super) fn get_order_by_client_oid_path(client_order_id: &str) -> ExchangeResult<String> {
    let client_oid = checked_client_oid(client_order_id)?;
    Ok(format!(
        "/api/v1/orders/byClientOid?clientOid={}",
        query_value(&client_oid)
    ))
}

pub(super) fn get_order_by_order_id_path(order_id: &str) -> ExchangeResult<String> {
    let order_id = order_id.trim();
    if order_id.is_empty() || !order_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(validation_error(format!(
            "kucoin orderId must be a non-empty decimal value: {order_id:?}"
        )));
    }
    Ok(format!("/api/v1/orders/{order_id}"))
}

pub(super) fn path_segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn query_value(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

pub(super) fn ack_from_order_row(
    internal_order_id: String,
    fallback_client_order_id: String,
    row: KucoinOrderAckRow,
    state: LiveOrderState,
    message: Option<String>,
) -> OrderAck {
    let exchange_order_id = Some(row.order_id);
    let venue_client_order_id = if row.client_oid.is_empty() {
        fallback_client_order_id.clone()
    } else {
        row.client_oid
    };
    OrderAck {
        internal_order_id,
        exchange_order_id: exchange_order_id.clone(),
        client_order_id: fallback_client_order_id.clone(),
        identity_update: VenueOrderIdentityUpdate::from_ids(
            fallback_client_order_id,
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

pub(super) fn ack_from_cancel_row(
    internal_order_id: String,
    client_order_id: String,
    fallback_order_id: Option<String>,
    row: KucoinCancelRow,
) -> OrderAck {
    let venue_client_order_id = row
        .client_oid
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(&client_order_id)
        .to_owned();
    let exchange_order_id = row
        .cancelled_order_ids
        .into_iter()
        .next()
        .or(fallback_order_id);
    let public_client_order_id = client_order_id.clone();
    OrderAck {
        internal_order_id,
        exchange_order_id: exchange_order_id.clone(),
        client_order_id,
        identity_update: VenueOrderIdentityUpdate::from_ids(
            public_client_order_id,
            venue_client_order_id,
            exchange_order_id,
        ),
        state: LiveOrderState::CancelRequested,
        accepted_at_ms: now_ms(),
        message: Some("kucoin cancel accepted; final state requires order stream/query".to_owned()),
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
    }
}

pub(super) fn ack_from_order_query(
    internal_order_id: String,
    client_order_id: String,
    order: OrderInfo,
) -> OrderAck {
    let exchange_order_id = non_empty(order.order_id);
    let venue_client_order_id = order
        .client_order_id
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| client_order_id.clone());
    OrderAck {
        internal_order_id,
        exchange_order_id: exchange_order_id.clone(),
        client_order_id: client_order_id.clone(),
        identity_update: VenueOrderIdentityUpdate::from_ids(
            client_order_id,
            venue_client_order_id,
            exchange_order_id,
        ),
        state: live_state_from_order_status(order.status),
        accepted_at_ms: now_ms(),
        message: Some(
            "kucoin place result recovered by GET /api/v1/orders/byClientOid using clientOid"
                .to_owned(),
        ),
        filled_quantity: (order.filled_quantity > 0.0).then_some(order.filled_quantity),
        filled_price: (order.filled_price > 0.0).then_some(order.filled_price),
        filled_fee: (order.fees != 0.0).then_some(order.fees),
    }
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn live_state_from_order_status(status: OrderStatus) -> LiveOrderState {
    match status {
        OrderStatus::Pending | OrderStatus::Open => LiveOrderState::Accepted,
        OrderStatus::PartiallyFilled => LiveOrderState::PartiallyFilled,
        OrderStatus::Filled => LiveOrderState::Filled,
        OrderStatus::Canceled => LiveOrderState::Cancelled,
        OrderStatus::Rejected => LiveOrderState::Rejected,
        OrderStatus::Expired => LiveOrderState::Failed,
    }
}

pub(super) fn validation_error(message: String) -> ExchangeError {
    ExchangeError::Api {
        exchange: NAME.into(),
        code: "validation".into(),
        message,
    }
}

fn build_place_order_body(
    intent: &OrderIntent,
    symbol: String,
    contract_unit: f64,
    position_side: &str,
) -> ExchangeResult<KucoinPlaceOrderBody> {
    let shape = kucoin_order_shape(intent)?;
    Ok(KucoinPlaceOrderBody {
        client_oid: checked_client_oid(&intent.client_order_id)?,
        side: kucoin_side(intent.side),
        symbol,
        order_type: shape.order_type,
        size: contract_count(intent.quantity, contract_unit)?,
        leverage: leverage_from_intent(intent.leverage)?,
        margin_mode: kucoin_margin_mode(intent.margin_mode),
        position_side: kucoin_position_side(position_side)?,
        price: shape.price,
        time_in_force: shape.time_in_force,
        post_only: shape.post_only,
        reduce_only: intent.reduce_only.then_some(true),
    })
}

fn kucoin_position_side(value: &str) -> ExchangeResult<&'static str> {
    match value {
        "BOTH" => Ok("BOTH"),
        "LONG" => Ok("LONG"),
        "SHORT" => Ok("SHORT"),
        _ => Err(validation_error(format!(
            "kucoin positionSide must be BOTH, LONG, or SHORT from verified account mode: {value}"
        ))),
    }
}

fn leverage_from_intent(leverage: f64) -> ExchangeResult<u32> {
    let rounded = leverage.round();
    if leverage.is_finite()
        && rounded >= 1.0
        && rounded <= f64::from(u32::MAX)
        && (leverage - rounded).abs() <= 1e-9
    {
        Ok(rounded as u32)
    } else {
        Err(validation_error(format!(
            "kucoin leverage must be a positive integer from OrderIntent: {leverage}"
        )))
    }
}

fn kucoin_margin_mode(mode: MarginMode) -> &'static str {
    match mode {
        MarginMode::Cross => "CROSS",
        MarginMode::Isolated => "ISOLATED",
    }
}

pub(super) fn checked_client_oid(value: &str) -> ExchangeResult<String> {
    crate::client_order_id_policy::required_venue_client_order_id(NAME, value)
}

fn kucoin_order_shape(intent: &OrderIntent) -> ExchangeResult<KucoinOrderShape> {
    match intent.order_type {
        OrderType::Market => Ok(KucoinOrderShape {
            order_type: "market",
            price: None,
            time_in_force: None,
            post_only: None,
        }),
        OrderType::Limit => Ok(KucoinOrderShape {
            order_type: "limit",
            price: Some(positive_number(intent.price, "price")?),
            time_in_force: Some(kucoin_time_in_force(intent.time_in_force)),
            post_only: kucoin_limit_post_only(intent.time_in_force),
        }),
        OrderType::PostOnly => Ok(KucoinOrderShape {
            order_type: "limit",
            price: Some(positive_number(intent.price, "price")?),
            time_in_force: Some("GTC"),
            post_only: Some(true),
        }),
    }
}

fn kucoin_time_in_force(time_in_force: TimeInForce) -> &'static str {
    match time_in_force {
        TimeInForce::Gtc | TimeInForce::Gtx => "GTC",
        TimeInForce::Ioc => "IOC",
        TimeInForce::Fok => "FOK",
    }
}

fn kucoin_limit_post_only(time_in_force: TimeInForce) -> Option<bool> {
    matches!(time_in_force, TimeInForce::Gtx).then_some(true)
}

fn kucoin_side(side: OrderSide) -> &'static str {
    match side {
        OrderSide::Buy => "buy",
        OrderSide::Sell => "sell",
    }
}

fn contract_count(quantity: f64, contract_unit: f64) -> ExchangeResult<i64> {
    let quantity = positive_value(quantity, "quantity")?;
    if !(contract_unit.is_finite() && contract_unit > 0.0) {
        return Err(validation_error(format!(
            "kucoin invalid contract unit: {contract_unit}"
        )));
    }
    let raw = quantity / contract_unit;
    let rounded = raw.round();
    if (raw - rounded).abs() > 1e-8 || rounded < 1.0 {
        return Err(validation_error(format!(
            "kucoin quantity {quantity} is not an integer contract count for unit {contract_unit}"
        )));
    }
    Ok(rounded as i64)
}

fn positive_number(value: Option<f64>, field: &str) -> ExchangeResult<String> {
    let value =
        value.ok_or_else(|| validation_error(format!("kucoin limit order requires {field}")))?;
    let value = positive_value(value, field)?;
    let text = format!("{value:.12}");
    Ok(text.trim_end_matches('0').trim_end_matches('.').to_owned())
}

fn positive_value(value: f64, field: &str) -> ExchangeResult<f64> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(validation_error(format!(
            "kucoin invalid positive {field}: {value}"
        )))
    }
}

#[cfg(test)]
#[path = "kucoin_trade_data_tests.rs"]
mod tests;
