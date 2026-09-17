//! Gate Spot private order stream.
//!
//! Official docs: <https://www.gate.com/docs/developers/apiv4/ws/en/#orders-channel>

use crate::adapter::{client_order_id_from_str, strip_common_suffixes};
use crate::error::{ExchangeError, ExchangeResult};
use crate::signing::gate as sign;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shared_types::{LiveOrderState, OrderInfo, OrderSide, OrderStatus, OrderType};

pub const GATE_SPOT_PRIVATE_WS_URL: &str = "wss://api.gateio.ws/ws/v4/";
pub const GATE_SPOT_ORDER_CHANNEL: &str = "spot.orders";

#[derive(Debug, Clone, PartialEq)]
pub enum GateSpotUserMessage {
    Control(GateSpotUserControl),
    Orders(Vec<GateSpotOrderUpdate>),
    Ignored,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateSpotUserControl {
    Acknowledged {
        request_id: Option<String>,
    },
    Rejected {
        request_id: Option<String>,
        authentication_failed: bool,
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct GateSpotOrderUpdate {
    pub client_order_id: String,
    pub live_state: LiveOrderState,
    pub order: OrderInfo,
    pub received_at_ms: i64,
}

pub fn subscribe_orders_payload(api_key: &str, api_secret: &str) -> ExchangeResult<String> {
    subscribe_orders_payload_at(api_key, api_secret, common::time::now_secs())
}

pub fn parse_message(text: &str) -> ExchangeResult<GateSpotUserMessage> {
    let envelope: RawEnvelope = serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!("gate spot user ws json: {error}; body={text}"))
    })?;
    if envelope.channel != GATE_SPOT_ORDER_CHANNEL {
        return Ok(GateSpotUserMessage::Ignored);
    }
    if envelope.event == "subscribe" {
        return Ok(GateSpotUserMessage::Control(control(envelope)));
    }
    if envelope.event != "update" {
        return Ok(GateSpotUserMessage::Ignored);
    }
    let rows: Vec<RawOrder> = serde_json::from_value(envelope.result)
        .map_err(|error| ExchangeError::Parse(format!("gate spot order rows: {error}")))?;
    let rows = rows
        .iter()
        .map(order_update)
        .collect::<ExchangeResult<Vec<_>>>()?;
    Ok(GateSpotUserMessage::Orders(rows))
}

fn subscribe_orders_payload_at(
    api_key: &str,
    api_secret: &str,
    time: i64,
) -> ExchangeResult<String> {
    let request = SubscribeRequest {
        time,
        channel: GATE_SPOT_ORDER_CHANNEL,
        event: "subscribe",
        payload: ["!all"],
        auth: AuthPayload {
            method: "api_key",
            key: required_text(api_key, "KEY")?,
            signature: sign::ws_sign(
                api_secret.as_bytes(),
                GATE_SPOT_ORDER_CHANNEL,
                "subscribe",
                &time.to_string(),
            ),
        },
    };
    serde_json::to_string(&request).map_err(|error| {
        ExchangeError::Parse(format!("gate spot user ws subscribe payload: {error}"))
    })
}

fn control(envelope: RawEnvelope) -> GateSpotUserControl {
    let request_id = value_text(&envelope.id).or_else(|| non_empty(&envelope.trace_id));
    if let Some(error) = envelope.error {
        return GateSpotUserControl::Rejected {
            request_id,
            authentication_failed: error.code == 4,
            error: format!("code={}; message={}", error.code, error.message),
        };
    }
    if envelope.result.get("status").and_then(Value::as_str) == Some("success") {
        GateSpotUserControl::Acknowledged { request_id }
    } else {
        GateSpotUserControl::Rejected {
            request_id,
            authentication_failed: false,
            error: "missing successful subscription status".to_owned(),
        }
    }
}

fn order_update(row: &RawOrder) -> ExchangeResult<GateSpotOrderUpdate> {
    let quantity = number(&row.amount, "amount")?;
    let left = number(&row.left, "left")?;
    let filled_quantity = (quantity - left).max(0.0);
    if left > quantity {
        return Err(parse_error("left exceeds amount", &row.left));
    }
    let status = order_status(&row.finish_as, &row.event, filled_quantity)?;
    let created_at_ms = timestamp_ms(&row.create_time_ms, &row.create_time)?;
    let received_at_ms = timestamp_ms_or(&row.update_time_ms, &row.update_time, created_at_ms)?;
    let client_order_id = row.text.trim().to_owned();
    let order = OrderInfo {
        order_id: required_text(&row.id, "id")?,
        symbol: strip_common_suffixes(&required_text(&row.currency_pair, "currency_pair")?),
        exchange: "gate".to_owned(),
        side: order_side(&row.side)?,
        order_type: order_type(&row.order_type, &row.time_in_force)?,
        status,
        quantity,
        price: number(&row.price, "price")?,
        filled_quantity,
        filled_price: number(&row.average_price, "avg_deal_price")?,
        fees: number(&row.fee, "fee")?,
        created_at: event_time(created_at_ms)?,
        execution_style: None,
        venue_time_in_force: non_empty(&row.time_in_force),
        client_order_id: client_order_id_from_str(&client_order_id),
        reduce_only: None,
    };
    Ok(GateSpotOrderUpdate {
        client_order_id,
        live_state: live_state(status),
        order,
        received_at_ms,
    })
}

fn order_status(finish_as: &str, event: &str, filled: f64) -> ExchangeResult<OrderStatus> {
    match finish_as.to_ascii_lowercase().as_str() {
        "open" if filled > 0.0 => Ok(OrderStatus::PartiallyFilled),
        "open" => Ok(OrderStatus::Open),
        "filled" => Ok(OrderStatus::Filled),
        "cancelled" => Ok(OrderStatus::Canceled),
        "ioc"
        | "stp"
        | "poc"
        | "fok"
        | "trader_not_enough"
        | "depth_not_enough"
        | "small"
        | "liquidate_cancelled" => Ok(OrderStatus::Expired),
        "-" if event == "put" => Ok(OrderStatus::Open),
        "-" if event == "update" && filled > 0.0 => Ok(OrderStatus::PartiallyFilled),
        "-" if event == "update" => Ok(OrderStatus::Open),
        _ => Err(parse_error("finish_as", finish_as)),
    }
}

fn order_side(raw: &str) -> ExchangeResult<OrderSide> {
    match raw.to_ascii_lowercase().as_str() {
        "buy" => Ok(OrderSide::Buy),
        "sell" => Ok(OrderSide::Sell),
        _ => Err(parse_error("side", raw)),
    }
}

fn order_type(raw: &str, tif: &str) -> ExchangeResult<OrderType> {
    let kind = raw.to_ascii_lowercase();
    if kind.starts_with("market") {
        Ok(OrderType::Market)
    } else if kind.starts_with("limit") && tif.eq_ignore_ascii_case("poc") {
        Ok(OrderType::PostOnly)
    } else if kind.starts_with("limit") {
        Ok(OrderType::Limit)
    } else {
        Err(parse_error("type", raw))
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

fn timestamp_ms(milliseconds: &str, seconds: &str) -> ExchangeResult<i64> {
    if !milliseconds.trim().is_empty() {
        return milliseconds
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && *value > 0.0)
            .map(|value| value.floor() as i64)
            .ok_or_else(|| parse_error("create_time_ms", milliseconds));
    }
    seconds
        .parse::<i64>()
        .ok()
        .filter(|value| *value > 0)
        .map(|value| value.saturating_mul(1_000))
        .ok_or_else(|| parse_error("create_time", seconds))
}

fn timestamp_ms_or(milliseconds: &str, seconds: &str, fallback: i64) -> ExchangeResult<i64> {
    if milliseconds.trim().is_empty() && seconds.trim().is_empty() {
        return Ok(fallback);
    }
    timestamp_ms(milliseconds, seconds)
}

fn event_time(value: i64) -> ExchangeResult<chrono::DateTime<Utc>> {
    chrono::DateTime::<Utc>::from_timestamp_millis(value)
        .ok_or_else(|| parse_error("timestamp", &value.to_string()))
}

fn number(raw: &str, field: &str) -> ExchangeResult<f64> {
    let value = if raw.trim().is_empty() {
        0.0
    } else {
        raw.parse::<f64>()
            .map_err(|error| ExchangeError::Parse(format!("gate spot {field}: {error}")))?
    };
    value
        .is_finite()
        .then_some(value)
        .ok_or_else(|| parse_error(field, raw))
}

fn required_text(raw: &str, field: &str) -> ExchangeResult<String> {
    non_empty(raw).ok_or_else(|| parse_error(field, raw))
}

fn non_empty(raw: &str) -> Option<String> {
    let value = raw.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => non_empty(text),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn parse_error(field: &str, value: &str) -> ExchangeError {
    ExchangeError::Parse(format!("gate spot user ws invalid {field}: value={value}"))
}

#[derive(Serialize)]
struct SubscribeRequest {
    time: i64,
    channel: &'static str,
    event: &'static str,
    payload: [&'static str; 1],
    auth: AuthPayload,
}

#[derive(Serialize)]
struct AuthPayload {
    method: &'static str,
    #[serde(rename = "KEY")]
    key: String,
    #[serde(rename = "SIGN")]
    signature: String,
}

#[derive(Debug, Default, Deserialize)]
struct RawEnvelope {
    #[serde(default)]
    id: Value,
    #[serde(default)]
    trace_id: String,
    #[serde(default)]
    channel: String,
    #[serde(default)]
    event: String,
    #[serde(default)]
    result: Value,
    #[serde(default)]
    error: Option<RawError>,
}

#[derive(Debug, Deserialize)]
struct RawError {
    code: i64,
    #[serde(default)]
    message: String,
}

#[derive(Debug, Default, Deserialize)]
struct RawOrder {
    #[serde(default)]
    id: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    create_time: String,
    #[serde(default)]
    create_time_ms: String,
    #[serde(default)]
    update_time: String,
    #[serde(default)]
    update_time_ms: String,
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
    time_in_force: String,
    #[serde(default)]
    left: String,
    #[serde(default, rename = "avg_deal_price")]
    average_price: String,
    #[serde(default)]
    fee: String,
    #[serde(default)]
    event: String,
    #[serde(default)]
    finish_as: String,
}

#[cfg(test)]
#[path = "gate_spot_ws_user_tests.rs"]
mod tests;
