//! KuCoin Classic Spot private Order V2 stream.
//!
//! Official docs: <https://www.kucoin.com/docs-new/3470073w0>

use super::kucoin_ws_user::{KucoinFillUpdate, KucoinOrderUpdate, KucoinUserEvent};
use crate::adapter::{client_order_id_from_str, strip_common_suffixes};
use crate::error::{ExchangeError, ExchangeResult};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shared_types::{LiveOrderState, OrderInfo, OrderSide, OrderStatus, OrderType};

pub const KUCOIN_SPOT_PRIVATE_BULLET_PATH: &str = "/api/v1/bullet-private";
pub const KUCOIN_SPOT_ORDER_TOPIC: &str = "/spotMarket/tradeOrdersV2";

#[derive(Debug, Clone, PartialEq)]
pub enum KucoinSpotUserMessage {
    Control(KucoinSpotUserControl),
    Event(KucoinUserEvent),
    Ignored,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KucoinSpotUserControl {
    Acknowledged {
        request_id: String,
    },
    Rejected {
        request_id: Option<String>,
        authentication_failed: bool,
        error: String,
    },
}

pub fn subscribe_orders_payload(request_id: &str) -> ExchangeResult<String> {
    let request = SubscribeRequest {
        id: required_text(request_id, "id")?,
        message_type: "subscribe",
        topic: KUCOIN_SPOT_ORDER_TOPIC,
        response: true,
        private_channel: true,
    };
    serde_json::to_string(&request).map_err(|error| {
        ExchangeError::Parse(format!("kucoin spot order subscribe payload: {error}"))
    })
}

pub fn parse_message(text: &str) -> ExchangeResult<KucoinSpotUserMessage> {
    let envelope: RawEnvelope = serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!("kucoin spot user ws json: {error}; body={text}"))
    })?;
    match envelope.message_type.as_str() {
        "ack" => Ok(KucoinSpotUserMessage::Control(
            KucoinSpotUserControl::Acknowledged {
                request_id: required_text(&envelope.id, "ack id")?,
            },
        )),
        "error" => Ok(KucoinSpotUserMessage::Control(
            KucoinSpotUserControl::Rejected {
                request_id: non_empty(&envelope.id),
                authentication_failed: matches!(envelope.code.as_str(), "400001" | "400002"),
                error: format!(
                    "code={}; message={}",
                    envelope.code,
                    value_text(&envelope.data).unwrap_or_else(|| "request rejected".to_owned())
                ),
            },
        )),
        "message"
            if envelope.topic == KUCOIN_SPOT_ORDER_TOPIC && envelope.subject == "orderChange" =>
        {
            serde_json::from_value(envelope.data)
                .map_err(|error| ExchangeError::Parse(format!("kucoin spot order row: {error}")))
                .and_then(order_update)
                .map(Box::new)
                .map(KucoinUserEvent::Order)
                .map(KucoinSpotUserMessage::Event)
        }
        _ => Ok(KucoinSpotUserMessage::Ignored),
    }
}

fn order_update(row: RawOrder) -> ExchangeResult<KucoinOrderUpdate> {
    let status = order_status(
        &row.event_type,
        &row.status,
        &row.filled_size,
        &row.origin_size,
    )?;
    let event_time_ms = timestamp_ms(row.timestamp)?;
    let client_order_id = required_text(&row.client_order_id, "clientOid")?;
    let exchange_order_id = required_text(&row.order_id, "orderId")?;
    let side = order_side(&row.side)?;
    let quantity = number(&row.origin_size, "originSize")?;
    let filled_quantity = number(&row.filled_size, "filledSize")?;
    let order = OrderInfo {
        order_id: exchange_order_id.clone(),
        symbol: strip_common_suffixes(&required_text(&row.symbol, "symbol")?),
        exchange: "kucoin".to_owned(),
        side,
        order_type: order_type(&row.order_type)?,
        status,
        quantity,
        price: number(&row.price, "price")?,
        filled_quantity,
        filled_price: number(&row.match_price, "matchPrice")?,
        fees: 0.0,
        created_at: event_time(if row.order_time_ms > 0 {
            row.order_time_ms
        } else {
            event_time_ms
        })?,
        execution_style: None,
        venue_time_in_force: None,
        client_order_id: client_order_id_from_str(&client_order_id),
        reduce_only: None,
    };
    let fill = fill_update(
        &row,
        &order,
        &exchange_order_id,
        &client_order_id,
        event_time_ms,
    )?;
    let terminal = matches!(
        status,
        OrderStatus::Filled | OrderStatus::Canceled | OrderStatus::Rejected | OrderStatus::Expired
    );
    Ok(KucoinOrderUpdate {
        client_order_id,
        event_type: row.event_type,
        event_time_ms,
        terminal,
        live_state: live_state(status),
        order,
        fill,
    })
}

fn fill_update(
    row: &RawOrder,
    order: &OrderInfo,
    exchange_order_id: &str,
    client_order_id: &str,
    event_time_ms: i64,
) -> ExchangeResult<Option<KucoinFillUpdate>> {
    if row.event_type != "match" {
        return Ok(None);
    }
    let trade_id = required_text(&row.trade_id, "tradeId")?;
    let quantity = number(&row.match_size, "matchSize")?;
    let price = number(&row.match_price, "matchPrice")?;
    if quantity <= 0.0 || price <= 0.0 {
        return Err(ExchangeError::Parse(
            "kucoin spot match event has non-positive fill".into(),
        ));
    }
    Ok(Some(KucoinFillUpdate {
        exchange_order_id: exchange_order_id.to_owned(),
        client_order_id: client_order_id_from_str(client_order_id),
        trade_id: trade_id.clone(),
        venue_event_id: format!("kucoin_spot_trade:{exchange_order_id}:{trade_id}"),
        symbol: order.symbol.clone(),
        side: order.side,
        quantity,
        price,
        liquidity: row.liquidity.trim().to_owned(),
        fee_type: row.fee_type.trim().to_owned(),
        fee_amount: None,
        fee_currency: None,
        occurred_at_ms: event_time_ms,
    }))
}

fn order_status(
    event_type: &str,
    status: &str,
    filled_size: &str,
    origin_size: &str,
) -> ExchangeResult<OrderStatus> {
    match event_type.to_ascii_lowercase().as_str() {
        "received" => Ok(OrderStatus::Pending),
        "open" => Ok(OrderStatus::Open),
        "match" => Ok(OrderStatus::PartiallyFilled),
        "filled" => Ok(OrderStatus::Filled),
        "canceled" => Ok(OrderStatus::Canceled),
        "update" if status.eq_ignore_ascii_case("done") => {
            let filled = number(filled_size, "filledSize")?;
            let origin = number(origin_size, "originSize")?;
            if origin > 0.0 && filled >= origin {
                Ok(OrderStatus::Filled)
            } else {
                Ok(OrderStatus::Canceled)
            }
        }
        "update" if status.eq_ignore_ascii_case("open") => Ok(OrderStatus::Open),
        _ => Err(parse_error(
            "type/status",
            &format!("{event_type}/{status}"),
        )),
    }
}

fn order_side(raw: &str) -> ExchangeResult<OrderSide> {
    match raw.to_ascii_lowercase().as_str() {
        "buy" => Ok(OrderSide::Buy),
        "sell" => Ok(OrderSide::Sell),
        _ => Err(parse_error("side", raw)),
    }
}

fn order_type(raw: &str) -> ExchangeResult<OrderType> {
    match raw.to_ascii_lowercase().as_str() {
        "market" => Ok(OrderType::Market),
        "limit" => Ok(OrderType::Limit),
        _ => Err(parse_error("orderType", raw)),
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

fn timestamp_ms(timestamp: i64) -> ExchangeResult<i64> {
    let value = if timestamp > 10_000_000_000_000_000 {
        timestamp / 1_000_000
    } else if timestamp > 10_000_000_000 {
        timestamp
    } else {
        timestamp.saturating_mul(1_000)
    };
    (value > 0)
        .then_some(value)
        .ok_or_else(|| parse_error("ts", &timestamp.to_string()))
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
            .map_err(|error| ExchangeError::Parse(format!("kucoin spot {field}: {error}")))?
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
        Value::Object(object) => object
            .get("msg")
            .and_then(Value::as_str)
            .and_then(non_empty),
        _ => None,
    }
}

fn parse_error(field: &str, value: &str) -> ExchangeError {
    ExchangeError::Parse(format!(
        "kucoin spot user ws invalid {field}: value={value}"
    ))
}

#[derive(Serialize)]
struct SubscribeRequest {
    id: String,
    #[serde(rename = "type")]
    message_type: &'static str,
    topic: &'static str,
    response: bool,
    #[serde(rename = "privateChannel")]
    private_channel: bool,
}

#[derive(Debug, Default, Deserialize)]
struct RawEnvelope {
    #[serde(default)]
    id: String,
    #[serde(default, rename = "type")]
    message_type: String,
    #[serde(default)]
    topic: String,
    #[serde(default)]
    subject: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    data: Value,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawOrder {
    #[serde(default, rename = "clientOid")]
    client_order_id: String,
    #[serde(default)]
    order_id: String,
    #[serde(default)]
    order_time_ms: i64,
    #[serde(default)]
    order_type: String,
    #[serde(default)]
    origin_size: String,
    #[serde(default)]
    side: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    symbol: String,
    #[serde(default, rename = "ts")]
    timestamp: i64,
    #[serde(default, rename = "type")]
    event_type: String,
    #[serde(default)]
    filled_size: String,
    #[serde(default)]
    match_price: String,
    #[serde(default)]
    match_size: String,
    #[serde(default)]
    price: String,
    #[serde(default)]
    trade_id: String,
    #[serde(default)]
    liquidity: String,
    #[serde(default)]
    fee_type: String,
}

#[cfg(test)]
#[path = "kucoin_spot_ws_user_tests.rs"]
mod tests;
