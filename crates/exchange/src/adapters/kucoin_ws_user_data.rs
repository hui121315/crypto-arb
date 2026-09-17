//! KuCoin Futures private WebSocket event parser internals.

use super::kucoin_ws_user::{
    KucoinBalanceDelta, KucoinFillUpdate, KucoinOrderUpdate, KucoinPositionDelta,
    KucoinProOrderAck, KucoinUserEvent, EXCHANGE, TOPIC_BALANCE, TOPIC_ORDER,
    TOPIC_POSITION_PREFIX, TYPE_MESSAGE,
};
use crate::adapter::client_order_id_from_str;
use crate::error::{ExchangeError, ExchangeResult};
use chrono::Utc;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;
use shared_types::{LiveOrderState, OrderInfo, OrderSide, OrderStatus, OrderType};

pub(super) fn parse_user_event(text: &str) -> ExchangeResult<Option<KucoinUserEvent>> {
    let envelope: RawEnvelope = serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!("kucoin user ws event: {error}; body={text}"))
    })?;
    if envelope.message_type != TYPE_MESSAGE {
        return Ok(None);
    }
    match envelope.topic_kind() {
        KucoinTopicKind::Order => parse_data(envelope.data, order_update)
            .map(|event| KucoinUserEvent::Order(Box::new(event)))
            .map(Some),
        KucoinTopicKind::Balance => parse_balance_data(envelope.subject, envelope.data)
            .map(KucoinUserEvent::Balance)
            .map(Some),
        KucoinTopicKind::Position => {
            { parse_position_data(&envelope.subject, &envelope.topic, envelope.data) }
                .map(KucoinUserEvent::Position)
                .map(Some)
        }
        KucoinTopicKind::Unknown => Ok(None),
    }
}

pub(super) fn parse_pro_order_ack(text: &str) -> ExchangeResult<Option<KucoinProOrderAck>> {
    let value: Value = serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!(
            "kucoin pro ws order response: {error}; body={text}"
        ))
    })?;
    let Some(response_type) = value.get("type").and_then(Value::as_str) else {
        return Ok(None);
    };
    if response_type != "response" && response_type != "message" {
        return Ok(None);
    }
    let response: RawProResponse = serde_json::from_value(value)
        .map_err(|error| ExchangeError::Parse(format!("kucoin pro ws response: {error}")))?;
    let request_id = required_str(&response.id, "pro.id")?.to_owned();
    let client_order_id = required_text(&response.data.client_oid, "pro.data.clientOid")?;
    let exchange_order_id = required_text(&response.data.order_id, "pro.data.orderId")?;
    let state = if response.success {
        LiveOrderState::Accepted
    } else {
        LiveOrderState::Rejected
    };
    Ok(Some(KucoinProOrderAck {
        request_id,
        client_order_id,
        exchange_order_id: Some(exchange_order_id),
        live_state: state,
        message: response.msg,
    }))
}

fn parse_data<T, O>(data: Value, convert: fn(&T) -> ExchangeResult<O>) -> ExchangeResult<O>
where
    T: DeserializeOwned,
{
    let row = serde_json::from_value(data)
        .map_err(|error| ExchangeError::Parse(format!("kucoin user ws data: {error}")))?;
    convert(&row)
}

fn parse_balance_data(subject: String, data: Value) -> ExchangeResult<KucoinBalanceDelta> {
    let row: RawBalanceDelta = serde_json::from_value(data)
        .map_err(|error| ExchangeError::Parse(format!("kucoin user ws balance: {error}")))?;
    balance_delta(subject, &row)
}

fn parse_position_data(
    subject: &str,
    topic: &str,
    data: Value,
) -> ExchangeResult<KucoinPositionDelta> {
    let row: RawPositionDelta = serde_json::from_value(data)
        .map_err(|error| ExchangeError::Parse(format!("kucoin user ws position: {error}")))?;
    position_delta(subject, topic, &row)
}

fn order_update(row: &RawOrderUpdate) -> ExchangeResult<KucoinOrderUpdate> {
    let event_type = order_event_type(&row.event_type)?;
    let order_id = required_text(&row.order_id, "order.orderId")?;
    let symbol = required_str(&row.symbol, "order.symbol")?;
    let filled_quantity = required_number(&row.filled_size, "order.filledSize")?;
    let quantity = positive_number(&row.size, "order.size")?;
    let remaining_quantity = non_negative_number(&row.remain_size, "order.remainSize")?;
    validate_filled_quantity(filled_quantity, quantity)?;
    validate_terminal_quantities(event_type, quantity, filled_quantity, remaining_quantity)?;
    let status = order_status(&row.status, event_type, filled_quantity)?;
    let created_at = event_time(required_time_ms("order.orderTime", &row.order_time)?)?;
    let event_time_ms = required_time_ms("order.ts", &row.ts)?;
    let side = order_side(&row.side)?;
    let fill = match_fill(row, event_type, &order_id, symbol, side, event_time_ms)?;
    let order = OrderInfo {
        execution_style: None,
        venue_time_in_force: None,
        client_order_id: client_order_id_from_str(&text_field(&row.client_oid)),
        reduce_only: None,
        order_id,
        symbol: normalized_symbol(symbol),
        exchange: EXCHANGE.into(),
        side,
        order_type: order_type(&row.order_type)?,
        status,
        quantity,
        price: non_negative_number(&row.price, "order.price")?,
        filled_quantity,
        filled_price: optional_number_or_zero(&row.match_price, "order.matchPrice")?,
        // Classic order pushes expose feeType, not an amount or currency.
        fees: 0.0,
        created_at,
    };
    Ok(KucoinOrderUpdate {
        client_order_id: text_field(&row.client_oid),
        event_type: event_type.to_owned(),
        event_time_ms,
        terminal: matches!(event_type, "filled" | "canceled"),
        live_state: live_state(status),
        order,
        fill,
    })
}

fn match_fill(
    row: &RawOrderUpdate,
    event_type: &str,
    order_id: &str,
    symbol: &str,
    side: OrderSide,
    occurred_at_ms: i64,
) -> ExchangeResult<Option<KucoinFillUpdate>> {
    if event_type != "match" {
        reject_match_only_fields(row)?;
        return Ok(None);
    }
    let trade_id = required_text(&row.trade_id, "order.tradeId")?;
    let liquidity = required_str(&row.liquidity, "order.liquidity")?.to_ascii_lowercase();
    let fee_type = required_text(&row.fee_type, "order.feeType")?;
    validate_fee_role(&liquidity, &fee_type)?;
    Ok(Some(KucoinFillUpdate {
        exchange_order_id: order_id.to_owned(),
        client_order_id: optional_text_field(&row.client_oid),
        venue_event_id: format!("kucoin_match:{order_id}:{trade_id}"),
        trade_id,
        symbol: normalized_symbol(symbol),
        side,
        quantity: positive_number(&row.match_size, "order.matchSize")?,
        price: positive_number(&row.match_price, "order.matchPrice")?,
        liquidity,
        fee_type,
        fee_amount: None,
        fee_currency: None,
        occurred_at_ms,
    }))
}

fn reject_match_only_fields(row: &RawOrderUpdate) -> ExchangeResult<()> {
    for (field, value) in [
        ("order.tradeId", &row.trade_id),
        ("order.matchSize", &row.match_size),
        ("order.matchPrice", &row.match_price),
        ("order.feeType", &row.fee_type),
    ] {
        if !value.is_null() && optional_text_field(value).is_some() {
            return Err(ExchangeError::Parse(format!(
                "kucoin ws {field} is only valid for type=match"
            )));
        }
    }
    Ok(())
}

fn validate_fee_role(liquidity: &str, fee_type: &str) -> ExchangeResult<()> {
    let expected = match liquidity {
        "maker" => "makerFee",
        "taker" => "takerFee",
        _ => {
            return Err(ExchangeError::Parse(format!(
                "kucoin ws order liquidity invalid: {liquidity}"
            )))
        }
    };
    if fee_type == expected {
        Ok(())
    } else {
        Err(ExchangeError::Parse(format!(
            "kucoin ws order feeType {fee_type} conflicts with liquidity {liquidity}"
        )))
    }
}

fn validate_filled_quantity(filled: f64, quantity: f64) -> ExchangeResult<()> {
    if filled >= 0.0 && filled <= quantity {
        Ok(())
    } else {
        Err(ExchangeError::Parse(format!(
            "kucoin ws order filledSize out of range: {filled}/{quantity}"
        )))
    }
}

fn validate_terminal_quantities(
    event_type: &str,
    quantity: f64,
    filled: f64,
    remaining: f64,
) -> ExchangeResult<()> {
    let valid = match event_type {
        "filled" => filled == quantity && remaining == 0.0,
        "canceled" => remaining == 0.0,
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err(ExchangeError::Parse(format!(
            "kucoin ws terminal quantity conflict: type={event_type}, size={quantity}, filledSize={filled}, remainSize={remaining}"
        )))
    }
}

fn balance_delta(subject: String, row: &RawBalanceDelta) -> ExchangeResult<KucoinBalanceDelta> {
    let currency = required_str(&row.currency, "balance.currency")?.to_owned();
    let _ = required_time_ms("balance.timestamp", &row.timestamp)?;
    Ok(KucoinBalanceDelta {
        subject,
        currency,
        total: required_number(&row.equity, "balance.equity")?,
        available: required_number(&row.available_balance, "balance.availableBalance")?,
        hold_balance: required_number(&row.hold_balance, "balance.holdBalance")?,
        unrealized_pnl: required_number(&row.cross_unrealized_pnl, "balance.crossUnPnl")?
            + required_number(&row.isolated_unrealized_pnl, "balance.isolatedUnPnl")?,
    })
}

fn position_delta(
    subject: &str,
    topic: &str,
    row: &RawPositionDelta,
) -> ExchangeResult<KucoinPositionDelta> {
    match subject.trim().to_ascii_lowercase().as_str() {
        "position.change" => Ok(KucoinPositionDelta::Change {
            native_symbol: required_str(&row.symbol, "position.symbol")?.to_ascii_uppercase(),
            current_contracts: required_number(&row.current_qty, "position.currentQty")?,
            updated_time_ms: required_time_ms("position.currentTimestamp", &row.current_timestamp)?,
        }),
        "position.settlement" => Ok(KucoinPositionDelta::Settlement {
            native_symbol: settlement_native_symbol(&row.symbol, topic),
            current_contracts: required_number(&row.qty, "position.qty")?,
            updated_time_ms: settlement_time_ms(row)?,
        }),
        "position.adjustrisklimit" => Ok(KucoinPositionDelta::RiskLimitAdjustment {
            success: row.success.ok_or_else(|| {
                ExchangeError::Parse("kucoin ws position.adjustRiskLimit success missing".into())
            })?,
        }),
        _ => Err(ExchangeError::Parse(format!(
            "kucoin ws position subject invalid: {subject}"
        ))),
    }
}

fn settlement_native_symbol(symbol: &str, topic: &str) -> Option<String> {
    let symbol = symbol.trim();
    if !symbol.is_empty() {
        return Some(symbol.to_ascii_uppercase());
    }
    topic
        .strip_prefix("/contract/position:")
        .map(str::trim)
        .filter(|symbol| !symbol.is_empty())
        .map(str::to_ascii_uppercase)
}

fn settlement_time_ms(row: &RawPositionDelta) -> ExchangeResult<i64> {
    if !row.ts.is_null() {
        required_time_ms("position.ts", &row.ts)
    } else {
        required_time_ms("position.fundingTime", &row.funding_time)
    }
}

fn normalized_symbol(symbol: &str) -> String {
    let upper = symbol.to_ascii_uppercase();
    let stripped = upper
        .strip_suffix("USDTM")
        .or_else(|| upper.strip_suffix("USDM"))
        .unwrap_or(&upper);
    if stripped == "XBT" {
        "BTC".to_owned()
    } else {
        stripped.to_owned()
    }
}

fn required_number(value: &Value, field: &str) -> ExchangeResult<f64> {
    optional_number(value, field)?
        .ok_or_else(|| ExchangeError::Parse(format!("kucoin ws numeric field {field} missing")))
}

fn positive_number(value: &Value, field: &str) -> ExchangeResult<f64> {
    let parsed = required_number(value, field)?;
    if parsed > 0.0 {
        Ok(parsed)
    } else {
        Err(ExchangeError::Parse(format!(
            "kucoin ws non-positive numeric field {field}: {parsed}"
        )))
    }
}

fn non_negative_number(value: &Value, field: &str) -> ExchangeResult<f64> {
    let parsed = required_number(value, field)?;
    if parsed >= 0.0 {
        Ok(parsed)
    } else {
        Err(ExchangeError::Parse(format!(
            "kucoin ws negative numeric field {field}: {parsed}"
        )))
    }
}

fn optional_number_or_zero(value: &Value, field: &str) -> ExchangeResult<f64> {
    optional_number(value, field).map(|value| value.unwrap_or(0.0))
}

fn optional_number(value: &Value, field: &str) -> ExchangeResult<Option<f64>> {
    match value {
        Value::Number(number) => number
            .as_f64()
            .filter(|value| value.is_finite())
            .map(Some)
            .ok_or_else(|| {
                ExchangeError::Parse(format!("kucoin ws non-f64 field {field}: {number}"))
            }),
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                trimmed
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite())
                    .map(Some)
                    .ok_or_else(|| {
                        ExchangeError::Parse(format!(
                            "kucoin ws numeric field {field} invalid; value={text}"
                        ))
                    })
            }
        }
        Value::Null => Ok(None),
        other => Err(ExchangeError::Parse(format!(
            "kucoin ws unsupported numeric field {field}: {other}"
        ))),
    }
}

fn required_time_ms(field: &str, value: &Value) -> ExchangeResult<i64> {
    let raw = match value {
        Value::Number(number) => number
            .as_i64()
            .map(i128::from)
            .or_else(|| number.as_u64().map(i128::from)),
        Value::String(text) => text.parse::<i128>().ok(),
        _ => None,
    }
    .ok_or_else(|| {
        ExchangeError::Parse(format!(
            "kucoin user ws timestamp {field} must be an exact integer: {value}"
        ))
    })?;
    let timestamp = match raw {
        value if value > 10_000_000_000_000_000 => value / 1_000_000,
        value if value > 10_000_000_000_000 => value / 1_000,
        value if value > 10_000_000_000 => value,
        value if value > 0 => value * 1_000,
        _ => {
            return Err(ExchangeError::Parse(format!(
                "kucoin user ws invalid timestamp {field}: {raw}"
            )));
        }
    };
    i64::try_from(timestamp).map_err(|_| {
        ExchangeError::Parse(format!(
            "kucoin user ws timestamp {field} is out of range: {raw}"
        ))
    })
}

fn text_field(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        _ => String::new(),
    }
}

fn required_text(value: &Value, field: &str) -> ExchangeResult<String> {
    optional_text_field(value)
        .ok_or_else(|| ExchangeError::Parse(format!("kucoin ws text field {field} missing")))
}

fn required_str<'a>(value: &'a str, field: &str) -> ExchangeResult<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        Err(ExchangeError::Parse(format!(
            "kucoin ws text field {field} missing"
        )))
    } else {
        Ok(value)
    }
}

fn optional_text_field(value: &Value) -> Option<String> {
    let text = text_field(value);
    (!text.is_empty()).then_some(text)
}

fn event_time(timestamp_ms: i64) -> ExchangeResult<chrono::DateTime<Utc>> {
    chrono::DateTime::<Utc>::from_timestamp_millis(timestamp_ms).ok_or_else(|| {
        ExchangeError::Parse(format!("kucoin user ws invalid timestamp: {timestamp_ms}"))
    })
}

fn order_side(side: &str) -> ExchangeResult<OrderSide> {
    match side.to_ascii_lowercase().as_str() {
        "buy" => Ok(OrderSide::Buy),
        "sell" => Ok(OrderSide::Sell),
        _ => Err(ExchangeError::Parse(format!(
            "kucoin ws order side invalid: {side}"
        ))),
    }
}

fn order_type(order_type: &str) -> ExchangeResult<OrderType> {
    match order_type.to_ascii_lowercase().as_str() {
        "market" => Ok(OrderType::Market),
        "limit" => Ok(OrderType::Limit),
        _ => Err(ExchangeError::Parse(format!(
            "kucoin ws orderType invalid: {order_type}"
        ))),
    }
}

fn order_event_type(event_type: &str) -> ExchangeResult<&'static str> {
    match event_type.to_ascii_lowercase().as_str() {
        "open" => Ok("open"),
        "update" => Ok("update"),
        "match" => Ok("match"),
        "filled" => Ok("filled"),
        "canceled" => Ok("canceled"),
        _ => Err(ExchangeError::Parse(format!(
            "kucoin ws order event type invalid: {event_type}"
        ))),
    }
}

fn order_status(status: &str, event_type: &str, filled: f64) -> ExchangeResult<OrderStatus> {
    let status = status.to_ascii_lowercase();
    match (event_type, status.as_str()) {
        ("open" | "update", "open") if filled > 0.0 => Ok(OrderStatus::PartiallyFilled),
        ("open" | "update", "open") => Ok(OrderStatus::Open),
        ("match", "open" | "match" | "done") => Ok(OrderStatus::PartiallyFilled),
        ("filled", "done") => Ok(OrderStatus::Filled),
        ("canceled", "done") => Ok(OrderStatus::Canceled),
        _ => Err(ExchangeError::Parse(format!(
            "kucoin ws order event/status conflict: type={event_type}, status={status}"
        ))),
    }
}

fn live_state(status: OrderStatus) -> LiveOrderState {
    match status {
        OrderStatus::Open => LiveOrderState::Accepted,
        OrderStatus::PartiallyFilled => LiveOrderState::PartiallyFilled,
        OrderStatus::Filled => LiveOrderState::Filled,
        OrderStatus::Canceled => LiveOrderState::Cancelled,
        OrderStatus::Rejected => LiveOrderState::Rejected,
        _ => LiveOrderState::Unknown,
    }
}

#[derive(Debug, Deserialize)]
struct RawEnvelope {
    #[serde(default, rename = "type")]
    message_type: String,
    #[serde(default)]
    topic: String,
    #[serde(default)]
    subject: String,
    #[serde(default)]
    data: Value,
}

impl RawEnvelope {
    fn topic_kind(&self) -> KucoinTopicKind {
        let subject = self.subject.to_ascii_lowercase();
        if self.topic.starts_with(TOPIC_ORDER) || subject.contains("order") {
            KucoinTopicKind::Order
        } else if self.topic == TOPIC_BALANCE || subject.contains("wallet") {
            KucoinTopicKind::Balance
        } else if self.topic.starts_with(TOPIC_POSITION_PREFIX) || subject.contains("position") {
            KucoinTopicKind::Position
        } else {
            KucoinTopicKind::Unknown
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawOrderUpdate {
    #[serde(default)]
    symbol: String,
    #[serde(default, rename = "orderId")]
    order_id: Value,
    #[serde(default, rename = "clientOid")]
    client_oid: Value,
    #[serde(default, rename = "type")]
    event_type: String,
    #[serde(default)]
    side: String,
    #[serde(default)]
    status: String,
    #[serde(default, rename = "orderType")]
    order_type: String,
    #[serde(default)]
    size: Value,
    #[serde(default)]
    price: Value,
    #[serde(default, rename = "filledSize")]
    filled_size: Value,
    #[serde(default, rename = "remainSize")]
    remain_size: Value,
    #[serde(default, rename = "matchPrice")]
    match_price: Value,
    #[serde(default, rename = "matchSize")]
    match_size: Value,
    #[serde(default, rename = "tradeId")]
    trade_id: Value,
    #[serde(default)]
    liquidity: String,
    #[serde(default, rename = "feeType")]
    fee_type: Value,
    #[serde(default, rename = "orderTime")]
    order_time: Value,
    #[serde(default)]
    ts: Value,
}

#[derive(Debug, Deserialize)]
struct RawBalanceDelta {
    #[serde(default)]
    currency: String,
    #[serde(default)]
    equity: Value,
    #[serde(default, rename = "availableBalance")]
    available_balance: Value,
    #[serde(default, rename = "holdBalance")]
    hold_balance: Value,
    #[serde(default, rename = "crossUnPnl")]
    cross_unrealized_pnl: Value,
    #[serde(default, rename = "isolatedUnPnl")]
    isolated_unrealized_pnl: Value,
    #[serde(default)]
    timestamp: Value,
}

#[derive(Debug, Deserialize)]
struct RawPositionDelta {
    #[serde(default)]
    symbol: String,
    #[serde(default, rename = "currentQty")]
    current_qty: Value,
    #[serde(default)]
    qty: Value,
    #[serde(default, rename = "currentTimestamp")]
    current_timestamp: Value,
    #[serde(default, rename = "fundingTime")]
    funding_time: Value,
    #[serde(default)]
    ts: Value,
    #[serde(default)]
    success: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct RawProResponse {
    id: String,
    success: bool,
    msg: Option<String>,
    data: RawProData,
}

#[derive(Debug, Deserialize)]
struct RawProData {
    #[serde(rename = "clientOid")]
    client_oid: Value,
    #[serde(rename = "orderId")]
    order_id: Value,
}

enum KucoinTopicKind {
    Order,
    Balance,
    Position,
    Unknown,
}
