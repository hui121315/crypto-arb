//! Bybit order write payload and ack parsing.
//!
//! Official Bybit V5 docs checked before moving these DTOs:
//! - POST /v5/order/create
//! - POST /v5/order/cancel
//! - WebSocket Trade `order.create` / `order.cancel`

use crate::error::{ExchangeError, ExchangeResult};
use common::time::now_ms;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use shared_types::{
    CancelOrderRequest, LiveOrderState, OrderAck, OrderIntent, OrderSide, OrderType, TimeInForce,
    VenueOrderIdentityUpdate,
};

const NAME: &str = "bybit";
const BYBIT_ID_MAX_LEN: usize = 36;
const BYBIT_ID_HASH_BYTES: usize = 12;
const BYBIT_WS_REQ_ID_PREFIX: &str = "br-";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlaceOrderBody {
    category: &'static str,
    symbol: String,
    side: &'static str,
    #[serde(rename = "orderType")]
    order_type: &'static str,
    qty: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    price: Option<String>,
    #[serde(rename = "timeInForce", skip_serializing_if = "Option::is_none")]
    time_in_force: Option<&'static str>,
    #[serde(
        rename = "slippageToleranceType",
        skip_serializing_if = "Option::is_none"
    )]
    slippage_tolerance_type: Option<&'static str>,
    #[serde(rename = "slippageTolerance", skip_serializing_if = "Option::is_none")]
    slippage_tolerance: Option<String>,
    #[serde(rename = "positionIdx", skip_serializing_if = "Option::is_none")]
    position_idx: Option<u8>,
    #[serde(rename = "marketUnit", skip_serializing_if = "Option::is_none")]
    market_unit: Option<&'static str>,
    #[serde(rename = "orderLinkId")]
    order_link_id: String,
    #[serde(rename = "reduceOnly", skip_serializing_if = "Option::is_none")]
    reduce_only: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CancelOrderBody {
    category: &'static str,
    symbol: String,
    #[serde(rename = "orderId", skip_serializing_if = "Option::is_none")]
    order_id: Option<String>,
    #[serde(rename = "orderLinkId")]
    order_link_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct OrderAckRow {
    #[serde(default, rename = "orderId")]
    order_id: String,
    #[serde(default, rename = "orderLinkId")]
    order_link_id: String,
}

struct BybitOrderShape {
    order_type: &'static str,
    price: Option<String>,
    time_in_force: Option<&'static str>,
    slippage_tolerance_type: Option<&'static str>,
    slippage_tolerance: Option<String>,
}

pub(super) fn place_order_body_json(
    intent: &OrderIntent,
    symbol: String,
    position_idx: u8,
) -> ExchangeResult<String> {
    let body = build_place_order_body(intent, symbol, position_idx)?;
    serde_json::to_string(&body)
        .map_err(|error| ExchangeError::Parse(format!("bybit place order body: {error}")))
}

pub(super) fn pre_check_order_body_json(
    intent: &OrderIntent,
    symbol: String,
    position_idx: u8,
) -> ExchangeResult<String> {
    let body = build_place_order_body(intent, symbol, position_idx)?;
    serde_json::to_string(&body)
        .map_err(|error| ExchangeError::Parse(format!("bybit pre-check order body: {error}")))
}

pub(super) fn cancel_order_body_json(
    request: &CancelOrderRequest,
    symbol: String,
) -> ExchangeResult<String> {
    let body = build_cancel_order_body(request, symbol)?;
    serde_json::to_string(&body)
        .map_err(|error| ExchangeError::Parse(format!("bybit cancel order body: {error}")))
}

pub(super) fn place_order_arg(
    intent: &OrderIntent,
    symbol: String,
    position_idx: u8,
) -> ExchangeResult<serde_json::Value> {
    let body = build_place_order_body(intent, symbol, position_idx)?;
    serde_json::to_value(body)
        .map_err(|error| ExchangeError::Parse(format!("bybit place order arg: {error}")))
}

pub(super) fn place_spot_order_arg(
    intent: &OrderIntent,
    symbol: String,
) -> ExchangeResult<serde_json::Value> {
    serde_json::to_value(build_spot_place_order_body(intent, symbol)?)
        .map_err(|error| ExchangeError::Parse(format!("bybit spot place order arg: {error}")))
}

#[cfg(test)]
pub(super) fn pre_check_order_arg(
    intent: &OrderIntent,
    symbol: String,
    position_idx: u8,
) -> ExchangeResult<serde_json::Value> {
    let body = build_place_order_body(intent, symbol, position_idx)?;
    serde_json::to_value(body)
        .map_err(|error| ExchangeError::Parse(format!("bybit pre-check order arg: {error}")))
}

pub(super) fn cancel_order_arg(
    request: &CancelOrderRequest,
    symbol: String,
) -> ExchangeResult<serde_json::Value> {
    let body = build_cancel_order_body(request, symbol)?;
    serde_json::to_value(body)
        .map_err(|error| ExchangeError::Parse(format!("bybit cancel order arg: {error}")))
}

pub(super) fn cancel_spot_order_arg(
    request: &CancelOrderRequest,
    symbol: String,
) -> ExchangeResult<serde_json::Value> {
    serde_json::to_value(build_cancel_order_body_for(request, symbol, "spot")?)
        .map_err(|error| ExchangeError::Parse(format!("bybit spot cancel order arg: {error}")))
}

pub(super) fn bybit_order_link_id(client_order_id: &str) -> ExchangeResult<String> {
    crate::client_order_id_policy::required_venue_client_order_id(NAME, client_order_id)
}

pub(super) fn bybit_ws_req_id(req_id: &str) -> ExchangeResult<String> {
    let raw = trimmed_required_id("reqId", req_id)?;
    if raw.len() <= BYBIT_ID_MAX_LEN {
        return Ok(raw.to_owned());
    }
    let derived = derived_bybit_id(BYBIT_WS_REQ_ID_PREFIX, b"crossline:bybit:reqId:v1:", raw);
    validate_ws_req_id(&derived)?;
    Ok(derived)
}

pub(super) fn validate_ws_req_id(req_id: &str) -> ExchangeResult<()> {
    validate_required_id("reqId", req_id)
}

pub(super) fn ack_from_row(
    internal_order_id: String,
    public_client_order_id: String,
    venue_client_order_id: String,
    row: OrderAckRow,
    state: LiveOrderState,
    message: Option<String>,
) -> OrderAck {
    let exchange_order_id = non_empty(row.order_id);
    let venue_client_order_id = non_empty(row.order_link_id).unwrap_or(venue_client_order_id);
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

fn build_place_order_body(
    intent: &OrderIntent,
    symbol: String,
    position_idx: u8,
) -> ExchangeResult<PlaceOrderBody> {
    let shape = bybit_order_shape(intent)?;
    let order_link_id = bybit_order_link_id(&intent.client_order_id)?;
    Ok(PlaceOrderBody {
        category: "linear",
        symbol,
        side: bybit_side(intent.side),
        order_type: shape.order_type,
        qty: number_param(intent.quantity)?,
        price: shape.price,
        time_in_force: shape.time_in_force,
        slippage_tolerance_type: shape.slippage_tolerance_type,
        slippage_tolerance: shape.slippage_tolerance,
        position_idx: Some(position_idx),
        market_unit: None,
        order_link_id,
        reduce_only: intent.reduce_only.then_some(true),
    })
}

fn build_cancel_order_body(
    request: &CancelOrderRequest,
    symbol: String,
) -> ExchangeResult<CancelOrderBody> {
    build_cancel_order_body_for(request, symbol, "linear")
}

fn build_spot_place_order_body(
    intent: &OrderIntent,
    symbol: String,
) -> ExchangeResult<PlaceOrderBody> {
    let shape = bybit_order_shape(intent)?;
    let order_link_id = bybit_order_link_id(&intent.client_order_id)?;
    Ok(PlaceOrderBody {
        category: "spot",
        symbol,
        side: bybit_side(intent.side),
        order_type: shape.order_type,
        qty: number_param(intent.quantity)?,
        price: shape.price,
        time_in_force: shape.time_in_force,
        slippage_tolerance_type: shape.slippage_tolerance_type,
        slippage_tolerance: shape.slippage_tolerance,
        position_idx: None,
        market_unit: Some("baseCoin"),
        order_link_id,
        reduce_only: None,
    })
}

fn build_cancel_order_body_for(
    request: &CancelOrderRequest,
    symbol: String,
    category: &'static str,
) -> ExchangeResult<CancelOrderBody> {
    let order_link_id = bybit_order_link_id(&request.client_order_id)?;
    Ok(CancelOrderBody {
        category,
        symbol,
        order_id: request.exchange_order_id.clone(),
        order_link_id,
    })
}

fn bybit_order_shape(intent: &OrderIntent) -> ExchangeResult<BybitOrderShape> {
    match intent.order_type {
        OrderType::Market => Ok(BybitOrderShape {
            order_type: "Market",
            price: None,
            time_in_force: None,
            slippage_tolerance_type: Some("Percent"),
            slippage_tolerance: Some(bybit_market_slippage_percent(intent)?),
        }),
        OrderType::Limit => Ok(BybitOrderShape {
            order_type: "Limit",
            price: Some(required_price(intent)?),
            time_in_force: Some(bybit_limit_time_in_force(intent.time_in_force)),
            slippage_tolerance_type: None,
            slippage_tolerance: None,
        }),
        OrderType::PostOnly => Ok(BybitOrderShape {
            order_type: "Limit",
            price: Some(required_price(intent)?),
            time_in_force: Some("PostOnly"),
            slippage_tolerance_type: None,
            slippage_tolerance: None,
        }),
    }
}

fn bybit_market_slippage_percent(intent: &OrderIntent) -> ExchangeResult<String> {
    let bps = intent.slippage_tolerance_bps.ok_or_else(|| {
        validation_error(
            "bybit market order requires official slippageTolerance evidence".to_owned(),
        )
    })?;
    if !bps.is_finite() || !(1.0..=1_000.0).contains(&bps) {
        return Err(validation_error(
            "bybit slippageTolerance Percent supports 1..=1000 bps".to_owned(),
        ));
    }
    if bps.fract().abs() > 1e-9 {
        return Err(validation_error(
            "bybit slippageTolerance Percent requires whole bps".to_owned(),
        ));
    }
    Ok(format_bybit_percent_slippage(bps))
}

fn format_bybit_percent_slippage(bps: f64) -> String {
    let text = format!("{:.2}", bps / 100.0);
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn bybit_limit_time_in_force(time_in_force: TimeInForce) -> &'static str {
    match time_in_force {
        TimeInForce::Gtc => "GTC",
        TimeInForce::Ioc => "IOC",
        TimeInForce::Fok => "FOK",
        TimeInForce::Gtx => "PostOnly",
    }
}

fn required_price(intent: &OrderIntent) -> ExchangeResult<String> {
    let price = intent
        .price
        .ok_or_else(|| validation_error("bybit limit order requires price".to_owned()))?;
    number_param(price)
}

fn bybit_side(side: OrderSide) -> &'static str {
    match side {
        OrderSide::Buy => "Buy",
        OrderSide::Sell => "Sell",
    }
}

fn number_param(value: f64) -> ExchangeResult<String> {
    if !value.is_finite() || value <= 0.0 {
        return Err(validation_error(format!(
            "bybit invalid positive number: {value}"
        )));
    }
    let text = format!("{value:.12}");
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    Ok(trimmed.to_owned())
}

fn validate_required_id(field: &str, value: &str) -> ExchangeResult<()> {
    if value.is_empty() {
        return Err(validation_error(format!("bybit {field} cannot be empty")));
    }
    if value.len() > BYBIT_ID_MAX_LEN {
        return Err(validation_error(format!(
            "bybit {field} cannot exceed {BYBIT_ID_MAX_LEN} characters"
        )));
    }
    Ok(())
}

fn trimmed_required_id<'a>(field: &str, value: &'a str) -> ExchangeResult<&'a str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(validation_error(format!("bybit {field} cannot be empty")));
    }
    Ok(trimmed)
}

fn derived_bybit_id(prefix: &str, namespace: &[u8], raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(namespace);
    hasher.update(raw.as_bytes());
    let digest = hasher.finalize();
    format!("{prefix}{}", hex::encode(&digest[..BYBIT_ID_HASH_BYTES]))
}

fn non_empty(value: String) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn validation_error(message: String) -> ExchangeError {
    ExchangeError::Api {
        exchange: NAME.into(),
        code: "validation".into(),
        message,
    }
}

#[cfg(test)]
#[path = "bybit_trade_data_tests.rs"]
mod tests;
