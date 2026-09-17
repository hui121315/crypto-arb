//! Binance Spot WebSocket API user-data subscription and event parser.
//!
//! Official docs:
//! - Subscription: <https://developers.binance.com/docs/binance-spot-api-docs/websocket-api/user-data-stream-requests>
//! - Events: <https://developers.binance.com/docs/binance-spot-api-docs/user-data-stream>

use super::binance_ws_user::{
    BinanceAccountBalanceDelta, BinanceAccountUpdate, BinanceOrderTradeUpdate, BinanceUserEvent,
};
use crate::adapter::{client_order_id_from_str, strip_common_suffixes};
use crate::error::{ExchangeError, ExchangeResult};
use crate::signing::binance::sign_query;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shared_types::{LiveOrderState, OrderInfo, OrderSide, OrderStatus, OrderType};

pub const BINANCE_SPOT_WS_API_URL: &str = "wss://ws-api.binance.com:443/ws-api/v3";

#[derive(Debug, Clone, PartialEq)]
pub enum BinanceSpotUserMessage {
    Control(BinanceSpotUserControl),
    Event(BinanceUserEvent),
    Ignored,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinanceSpotUserControl {
    Acknowledged {
        request_id: String,
    },
    Rejected {
        request_id: String,
        authentication_failed: bool,
        error: String,
    },
}

pub fn subscription_payload(
    api_key: &str,
    api_secret: &str,
    request_id: &str,
) -> ExchangeResult<String> {
    let timestamp = common::time::now_ms();
    subscription_payload_at(api_key, api_secret, request_id, timestamp)
}

pub fn parse_message(text: &str) -> ExchangeResult<BinanceSpotUserMessage> {
    let root: Value = serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!("binance spot user ws json: {error}; body={text}"))
    })?;
    if root.get("status").is_some() {
        return parse_control(&root).map(BinanceSpotUserMessage::Control);
    }
    let event = root.get("event").unwrap_or(&root).clone();
    let envelope: RawEvent = serde_json::from_value(event).map_err(|error| {
        ExchangeError::Parse(format!("binance spot user ws event: {error}; body={text}"))
    })?;
    match envelope.event_type.as_str() {
        "executionReport" => parse_order(&envelope).map(BinanceSpotUserMessage::Event),
        "outboundAccountPosition" => parse_account(envelope).map(BinanceSpotUserMessage::Event),
        _ => Ok(BinanceSpotUserMessage::Ignored),
    }
}

fn subscription_payload_at(
    api_key: &str,
    api_secret: &str,
    request_id: &str,
    timestamp: i64,
) -> ExchangeResult<String> {
    let signing_payload = format!("apiKey={api_key}&timestamp={timestamp}");
    let request = SubscriptionRequest {
        id: required_text(request_id, "id")?,
        method: "userDataStream.subscribe.signature",
        params: SubscriptionParams {
            api_key: required_text(api_key, "apiKey")?,
            timestamp,
            signature: sign_query(api_secret.as_bytes(), &signing_payload),
        },
    };
    serde_json::to_string(&request).map_err(|error| {
        ExchangeError::Parse(format!("binance spot user ws subscribe payload: {error}"))
    })
}

fn parse_control(root: &Value) -> ExchangeResult<BinanceSpotUserControl> {
    let request_id = value_text(root.get("id"))
        .ok_or_else(|| ExchangeError::Parse("binance spot user ws ack missing id".into()))?;
    let status = root
        .get("status")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    if status == 200 {
        return Ok(BinanceSpotUserControl::Acknowledged { request_id });
    }
    let code = root
        .pointer("/error/code")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let message = root
        .pointer("/error/msg")
        .and_then(Value::as_str)
        .unwrap_or("subscription rejected");
    Ok(BinanceSpotUserControl::Rejected {
        request_id,
        authentication_failed: matches!(status, 401 | 403) || matches!(code, -2014 | -2015),
        error: format!("status={status}; code={code}; message={message}"),
    })
}

fn parse_order(envelope: &RawEvent) -> ExchangeResult<BinanceUserEvent> {
    let event_time_ms = positive_timestamp(envelope.event_time_ms, "E")?;
    let transaction_time_ms = positive_timestamp(envelope.transaction_time_ms, "T")?;
    let status = order_status(&envelope.order_status)?;
    let execution_type = canonical_execution_type(&envelope.execution_type)?;
    let quantity = number(&envelope.original_quantity, "q")?;
    let filled_quantity = number(&envelope.filled_quantity, "z")?;
    let last_filled_quantity = number(&envelope.last_filled_quantity, "l")?;
    let last_filled_price = number(&envelope.last_filled_price, "L")?;
    let cumulative_quote = number(&envelope.cumulative_quote, "Z")?;
    let commission = number(&envelope.commission, "n")?;
    let trade_id =
        (execution_type == "TRADE" && envelope.trade_id >= 0).then_some(envelope.trade_id);
    let filled_price = if filled_quantity > 0.0 {
        cumulative_quote / filled_quantity
    } else {
        0.0
    };
    let created_at_ms = if envelope.order_create_time_ms > 0 {
        envelope.order_create_time_ms
    } else {
        transaction_time_ms
    };
    let order = OrderInfo {
        order_id: positive_identity(envelope.order_id, "i")?,
        symbol: strip_common_suffixes(&required_text(&envelope.symbol, "s")?),
        exchange: "binance".to_owned(),
        side: order_side(&envelope.side)?,
        order_type: order_type(&envelope.order_type, &envelope.time_in_force)?,
        status,
        quantity,
        price: number(&envelope.price, "p")?,
        filled_quantity,
        filled_price,
        fees: commission,
        created_at: event_time(created_at_ms)?,
        execution_style: None,
        venue_time_in_force: clean_text(&envelope.time_in_force),
        client_order_id: client_order_id_from_str(&envelope.client_order_id),
        reduce_only: None,
    };
    Ok(BinanceUserEvent::Order(Box::new(BinanceOrderTradeUpdate {
        event_time_ms,
        transaction_time_ms,
        trade_time_ms: transaction_time_ms,
        client_order_id: required_text(&envelope.client_order_id, "c")?,
        execution_type,
        order_status: envelope.order_status.to_ascii_uppercase(),
        reject_reason: clean_reject_reason(&envelope.reject_reason),
        trade_id,
        last_filled_quantity,
        last_filled_price,
        commission_asset: clean_text(&envelope.commission_asset),
        live_state: live_state(status),
        order,
    })))
}

fn parse_account(envelope: RawEvent) -> ExchangeResult<BinanceUserEvent> {
    let event_time_ms = positive_timestamp(envelope.event_time_ms, "E")?;
    let transaction_time_ms =
        positive_timestamp(envelope.account_update_time_ms.max(event_time_ms), "u")?;
    let balances = envelope
        .balances
        .into_iter()
        .map(|row| {
            let available = number(&row.available, "B.f")?;
            let locked = number(&row.locked, "B.l")?;
            Ok(BinanceAccountBalanceDelta {
                asset: required_text(&row.asset, "B.a")?,
                wallet_balance: available + locked,
                cross_wallet_balance: available + locked,
                balance_change: 0.0,
            })
        })
        .collect::<ExchangeResult<Vec<_>>>()?;
    Ok(BinanceUserEvent::Account(BinanceAccountUpdate {
        event_time_ms,
        transaction_time_ms,
        reason: "spot_outbound_account_position".to_owned(),
        balances,
        positions: Vec::new(),
    }))
}

fn order_status(raw: &str) -> ExchangeResult<OrderStatus> {
    match raw.to_ascii_uppercase().as_str() {
        "PENDING_NEW" => Ok(OrderStatus::Pending),
        "NEW" | "PENDING_CANCEL" => Ok(OrderStatus::Open),
        "PARTIALLY_FILLED" => Ok(OrderStatus::PartiallyFilled),
        "FILLED" => Ok(OrderStatus::Filled),
        "CANCELED" => Ok(OrderStatus::Canceled),
        "REJECTED" => Ok(OrderStatus::Rejected),
        "EXPIRED" | "EXPIRED_IN_MATCH" => Ok(OrderStatus::Expired),
        _ => Err(parse_error("order status", raw)),
    }
}

fn canonical_execution_type(raw: &str) -> ExchangeResult<String> {
    match raw.to_ascii_uppercase().as_str() {
        "NEW" | "CANCELED" | "REPLACED" | "REJECTED" | "TRADE" | "EXPIRED" | "TRADE_PREVENTION" => {
            Ok(raw.to_ascii_uppercase())
        }
        _ => Err(parse_error("execution type", raw)),
    }
}

fn order_side(raw: &str) -> ExchangeResult<OrderSide> {
    match raw.to_ascii_uppercase().as_str() {
        "BUY" => Ok(OrderSide::Buy),
        "SELL" => Ok(OrderSide::Sell),
        _ => Err(parse_error("order side", raw)),
    }
}

fn order_type(raw: &str, tif: &str) -> ExchangeResult<OrderType> {
    match (
        raw.to_ascii_uppercase().as_str(),
        tif.to_ascii_uppercase().as_str(),
    ) {
        ("MARKET", _) => Ok(OrderType::Market),
        ("LIMIT_MAKER", _) | ("LIMIT", "GTX") => Ok(OrderType::PostOnly),
        ("LIMIT", "GTC" | "IOC" | "FOK") => Ok(OrderType::Limit),
        _ => Err(parse_error("order type", raw)),
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

fn number(raw: &str, field: &str) -> ExchangeResult<f64> {
    let value = if raw.trim().is_empty() {
        0.0
    } else {
        raw.parse::<f64>().map_err(|error| {
            ExchangeError::Parse(format!(
                "binance spot user ws numeric field {field}: {error}; value={raw}"
            ))
        })?
    };
    if value.is_finite() {
        Ok(value)
    } else {
        Err(parse_error(field, raw))
    }
}

fn positive_timestamp(value: i64, field: &str) -> ExchangeResult<i64> {
    (value > 0)
        .then_some(value)
        .ok_or_else(|| parse_error(field, &value.to_string()))
}

fn positive_identity(value: i64, field: &str) -> ExchangeResult<String> {
    (value > 0)
        .then(|| value.to_string())
        .ok_or_else(|| parse_error(field, &value.to_string()))
}

fn event_time(value: i64) -> ExchangeResult<chrono::DateTime<Utc>> {
    chrono::DateTime::<Utc>::from_timestamp_millis(value)
        .ok_or_else(|| parse_error("timestamp", &value.to_string()))
}

fn required_text(raw: &str, field: &str) -> ExchangeResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        Err(parse_error(field, raw))
    } else {
        Ok(trimmed.to_owned())
    }
}

fn clean_text(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn clean_reject_reason(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("NONE")).then(|| trimmed.to_owned())
}

fn value_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_owned()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn parse_error(field: &str, value: &str) -> ExchangeError {
    ExchangeError::Parse(format!(
        "binance spot user ws invalid {field}: value={value}"
    ))
}

#[derive(Serialize)]
struct SubscriptionRequest {
    id: String,
    method: &'static str,
    params: SubscriptionParams,
}

#[derive(Serialize)]
struct SubscriptionParams {
    #[serde(rename = "apiKey")]
    api_key: String,
    timestamp: i64,
    signature: String,
}

#[derive(Debug, Default, Deserialize)]
struct RawEvent {
    #[serde(default, rename = "e")]
    event_type: String,
    #[serde(default, rename = "E")]
    event_time_ms: i64,
    #[serde(default, rename = "T")]
    transaction_time_ms: i64,
    #[serde(default, rename = "u")]
    account_update_time_ms: i64,
    #[serde(default, rename = "B")]
    balances: Vec<RawBalance>,
    #[serde(default, rename = "s")]
    symbol: String,
    #[serde(default, rename = "c")]
    client_order_id: String,
    #[serde(default, rename = "S")]
    side: String,
    #[serde(default, rename = "o")]
    order_type: String,
    #[serde(default, rename = "f")]
    time_in_force: String,
    #[serde(default, rename = "q")]
    original_quantity: String,
    #[serde(default, rename = "p")]
    price: String,
    #[serde(default, rename = "x")]
    execution_type: String,
    #[serde(default, rename = "X")]
    order_status: String,
    #[serde(default, rename = "r")]
    reject_reason: String,
    #[serde(default, rename = "i")]
    order_id: i64,
    #[serde(default, rename = "l")]
    last_filled_quantity: String,
    #[serde(default, rename = "z")]
    filled_quantity: String,
    #[serde(default, rename = "L")]
    last_filled_price: String,
    #[serde(default, rename = "Z")]
    cumulative_quote: String,
    #[serde(default, rename = "n")]
    commission: String,
    #[serde(default, rename = "N")]
    commission_asset: String,
    #[serde(default, rename = "t")]
    trade_id: i64,
    #[serde(default, rename = "O")]
    order_create_time_ms: i64,
}

#[derive(Debug, Default, Deserialize)]
struct RawBalance {
    #[serde(default, rename = "a")]
    asset: String,
    #[serde(default, rename = "f")]
    available: String,
    #[serde(default, rename = "l")]
    locked: String,
}

#[cfg(test)]
#[path = "binance_spot_ws_user_tests.rs"]
mod tests;
