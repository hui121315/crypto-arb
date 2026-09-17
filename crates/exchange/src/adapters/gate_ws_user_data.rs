//! Gate.io futures private WebSocket event parser internals.

use super::gate_ws_user::{
    GateBalanceDelta, GateOrderUpdate, GatePositionDelta, GateUserEvent, GateUserTradeDelta,
    CHANNEL_BALANCES, CHANNEL_ORDERS, CHANNEL_POSITIONS, CHANNEL_USERTRADES, EVENT_UPDATE,
    EXCHANGE,
};
use crate::adapter::{client_order_id_from_str, strip_common_suffixes};
use crate::error::{ExchangeError, ExchangeResult};
use chrono::Utc;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;
use shared_types::{LiveOrderState, OrderInfo, OrderSide, OrderStatus, OrderType};

pub(super) fn parse_user_event(text: &str) -> ExchangeResult<Option<GateUserEvent>> {
    let envelope: RawEnvelope = serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!("gate user ws event: {error}; body={text}"))
    })?;
    if envelope.event != EVENT_UPDATE {
        return Ok(None);
    }
    match envelope.channel.as_str() {
        CHANNEL_ORDERS => parse_event_rows(envelope.result, order_update)
            .map(GateUserEvent::Order)
            .map(Some),
        CHANNEL_POSITIONS => parse_event_rows(envelope.result, position_delta)
            .map(GateUserEvent::Position)
            .map(Some),
        CHANNEL_BALANCES => parse_event_rows(envelope.result, balance_delta)
            .map(GateUserEvent::Balance)
            .map(Some),
        CHANNEL_USERTRADES => parse_event_rows(envelope.result, user_trade_delta)
            .map(GateUserEvent::UserTrade)
            .map(Some),
        _ => Ok(None),
    }
}

fn parse_event_rows<T, O>(
    value: Value,
    convert: fn(T) -> ExchangeResult<O>,
) -> ExchangeResult<Vec<O>>
where
    T: DeserializeOwned,
{
    let rows: Vec<T> = serde_json::from_value(value)
        .map_err(|error| ExchangeError::Parse(format!("gate user ws rows: {error}")))?;
    rows.into_iter().map(convert).collect()
}

fn order_update(row: RawOrderUpdate) -> ExchangeResult<GateOrderUpdate> {
    let size = row.size_value()?;
    let quantity = size.abs();
    let left = row.left_value()?.abs();
    let status = order_status(&row.status, &row.finish_as, left, quantity)?;
    let price = finite_value_number(&row.price, "order.price")?;
    let order = OrderInfo {
        execution_style: None,
        venue_time_in_force: Some(row.tif.clone()),
        client_order_id: client_order_id_from_str(&row.text),
        reduce_only: Some(row.is_reduce_only),
        order_id: row.id.to_string(),
        symbol: strip_common_suffixes(&row.contract),
        exchange: EXCHANGE.into(),
        side: signed_side(size),
        order_type: order_type(&row.tif, price)?,
        status,
        quantity,
        price,
        filled_quantity: (quantity - left).max(0.0),
        filled_price: finite_value_number(&row.fill_price, "order.fill_price")?,
        fees: 0.0,
        created_at: event_time(row.create_time_ms)?,
    };
    Ok(GateOrderUpdate {
        client_order_id: row.text,
        live_state: live_state(status),
        order,
    })
}

fn position_delta(row: RawPositionDelta) -> ExchangeResult<GatePositionDelta> {
    let RawPositionDelta {
        contract,
        size,
        entry_price,
        mark_price,
        unrealised_pnl,
        leverage,
        liq_price,
        margin,
        maintenance_rate,
        update_time,
    } = row;
    let size = finite_value_number(&size, "position.size")?;
    Ok(GatePositionDelta {
        symbol: strip_common_suffixes(&contract),
        side: signed_position_side(size),
        size: size.abs(),
        entry_price: finite_value_number(&entry_price, "position.entry_price")?,
        mark_price: finite_value_number(&mark_price, "position.mark_price")?,
        unrealized_pnl: finite_value_number(&unrealised_pnl, "position.unrealised_pnl")?,
        leverage: finite_value_number(&leverage, "position.leverage")?,
        liquidation_price: positive_value_option(&liq_price, "position.liq_price")?,
        margin: finite_value_number(&margin, "position.margin")?,
        maintenance_margin_ratio: non_negative_value_number(
            &maintenance_rate,
            "position.maintenance_rate",
        )?,
        updated_time_ms: update_time,
    })
}

fn balance_delta(row: RawBalanceDelta) -> ExchangeResult<GateBalanceDelta> {
    Ok(GateBalanceDelta {
        currency: row.currency,
        balance: finite_value_number(&row.balance, "balance.balance")?,
        change: finite_value_number(&row.change, "balance.change")?,
        available: finite_value_number(&row.available, "balance.available")?,
        position_margin: finite_value_number(&row.position_margin, "balance.position_margin")?,
        order_margin: finite_value_number(&row.order_margin, "balance.order_margin")?,
        unrealized_pnl: finite_value_number(&row.unrealised_pnl, "balance.unrealised_pnl")?,
    })
}

fn user_trade_delta(row: RawUserTradeDelta) -> ExchangeResult<GateUserTradeDelta> {
    let RawUserTradeDelta {
        id,
        order_id,
        contract,
        size,
        price,
        fee,
        point_fee,
        create_time_ms,
    } = row;
    let size = required_value_number(&size, "usertrade.size")?;
    let quantity = positive_quantity(size, "usertrade.size")?;
    Ok(GateUserTradeDelta {
        trade_id: required_text(&id, "usertrade.id")?,
        exchange_order_id: required_text(&order_id, "usertrade.order_id")?,
        symbol: strip_common_suffixes(&required_text(&contract, "usertrade.contract")?),
        quantity,
        price: positive_number(&price, "usertrade.price")?,
        fee: required_value_number(&fee, "usertrade.fee")?,
        point_fee: value_number(&point_fee, "usertrade.point_fee")?,
        occurred_at_ms: create_time_ms,
    })
}

fn parse_number(raw: &str, field: &str) -> ExchangeResult<f64> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(0.0);
    }
    value.parse().map_err(|error| {
        ExchangeError::Parse(format!(
            "gate user ws numeric field {field}: {error}; value={raw}"
        ))
    })
}

fn finite_number(raw: &str, field: &str) -> ExchangeResult<f64> {
    let value = parse_number(raw, field)?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(ExchangeError::Parse(format!(
            "gate user ws non-finite numeric field {field}: {raw}"
        )))
    }
}

fn positive_number(raw: &str, field: &str) -> ExchangeResult<f64> {
    let value = finite_number(raw, field)?;
    if value > 0.0 {
        Ok(value)
    } else {
        Err(ExchangeError::Parse(format!(
            "gate user ws non-positive numeric field {field}: {raw}"
        )))
    }
}

fn non_negative_value_number(raw: &Value, field: &str) -> ExchangeResult<f64> {
    let value = finite_value_number(raw, field)?;
    if value >= 0.0 {
        Ok(value)
    } else {
        Err(ExchangeError::Parse(format!(
            "gate user ws negative numeric field {field}: {raw}"
        )))
    }
}

fn positive_quantity(value: f64, field: &str) -> ExchangeResult<f64> {
    let quantity = value.abs();
    if quantity.is_finite() && quantity > 0.0 {
        Ok(quantity)
    } else {
        Err(ExchangeError::Parse(format!(
            "gate user ws non-positive quantity field {field}: {value}"
        )))
    }
}

fn required_text(raw: &str, field: &str) -> ExchangeResult<String> {
    let value = raw.trim();
    if value.is_empty() {
        Err(ExchangeError::Parse(format!(
            "gate user ws missing required field {field}"
        )))
    } else {
        Ok(value.to_owned())
    }
}

fn positive_value_option(raw: &Value, field: &str) -> ExchangeResult<Option<f64>> {
    let value = finite_value_number(raw, field)?;
    Ok((value > 0.0).then_some(value))
}

fn event_time(timestamp_ms: i64) -> ExchangeResult<chrono::DateTime<Utc>> {
    chrono::DateTime::<Utc>::from_timestamp_millis(timestamp_ms).ok_or_else(|| {
        ExchangeError::Parse(format!("gate user ws invalid timestamp: {timestamp_ms}"))
    })
}

fn signed_side(size: f64) -> OrderSide {
    if size < 0.0 {
        OrderSide::Sell
    } else {
        OrderSide::Buy
    }
}

fn signed_position_side(size: f64) -> String {
    if size < 0.0 {
        "short".to_owned()
    } else {
        "long".to_owned()
    }
}

fn order_type(tif: &str, price: f64) -> ExchangeResult<OrderType> {
    match tif.to_ascii_lowercase().as_str() {
        "poc" => Ok(OrderType::PostOnly),
        "ioc" if price == 0.0 => Ok(OrderType::Market),
        "gtc" | "ioc" | "fok" => Ok(OrderType::Limit),
        other => Err(ExchangeError::Parse(format!(
            "gate user ws order has unknown tif {other}"
        ))),
    }
}

fn order_status(
    status: &str,
    finish_as: &str,
    left: f64,
    quantity: f64,
) -> ExchangeResult<OrderStatus> {
    if left > quantity {
        return Err(ExchangeError::Parse(format!(
            "gate user ws order left {left} exceeds size {quantity}"
        )));
    }
    match status.to_ascii_lowercase().as_str() {
        "open" if left < quantity => Ok(OrderStatus::PartiallyFilled),
        "open" => Ok(OrderStatus::Open),
        "finished" if left == 0.0 && finish_as.eq_ignore_ascii_case("filled") => {
            Ok(OrderStatus::Filled)
        }
        "finished" => Ok(OrderStatus::Canceled),
        "cancelled" | "canceled" => Ok(OrderStatus::Canceled),
        other => Err(ExchangeError::Parse(format!(
            "gate user ws order has unknown status {other}"
        ))),
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

#[derive(Debug, Deserialize)]
struct RawEnvelope {
    #[serde(default)]
    channel: String,
    #[serde(default)]
    event: String,
    #[serde(default)]
    result: Value,
}

#[derive(Debug, Deserialize)]
struct RawOrderUpdate {
    id: i64,
    contract: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    finish_as: String,
    #[serde(default)]
    size: Value,
    #[serde(default)]
    left: Value,
    #[serde(default)]
    price: Value,
    #[serde(default, rename = "fill_price")]
    fill_price: Value,
    #[serde(default, rename = "create_time_ms")]
    create_time_ms: i64,
    #[serde(default)]
    text: String,
    #[serde(default)]
    tif: String,
    #[serde(default, rename = "is_reduce_only")]
    is_reduce_only: bool,
}

impl RawOrderUpdate {
    fn size_value(&self) -> ExchangeResult<f64> {
        value_number(&self.size, "order.size")
    }

    fn left_value(&self) -> ExchangeResult<f64> {
        value_number(&self.left, "order.left")
    }
}

#[derive(Debug, Deserialize)]
struct RawPositionDelta {
    contract: String,
    #[serde(default)]
    size: Value,
    #[serde(default, rename = "entry_price")]
    entry_price: Value,
    #[serde(default, rename = "mark_price")]
    mark_price: Value,
    #[serde(default, rename = "unrealised_pnl")]
    unrealised_pnl: Value,
    #[serde(default)]
    leverage: Value,
    #[serde(default, rename = "liq_price")]
    liq_price: Value,
    #[serde(default)]
    margin: Value,
    #[serde(default, rename = "maintenance_rate")]
    maintenance_rate: Value,
    #[serde(default, rename = "update_time", alias = "time_ms")]
    update_time: i64,
}

#[derive(Debug, Deserialize)]
struct RawBalanceDelta {
    #[serde(default)]
    currency: String,
    #[serde(default)]
    balance: Value,
    #[serde(default)]
    change: Value,
    #[serde(default)]
    available: Value,
    #[serde(default, rename = "position_margin")]
    position_margin: Value,
    #[serde(default, rename = "order_margin")]
    order_margin: Value,
    #[serde(default, rename = "unrealised_pnl")]
    unrealised_pnl: Value,
}

#[derive(Debug, Deserialize)]
struct RawUserTradeDelta {
    id: String,
    #[serde(rename = "order_id")]
    order_id: String,
    contract: String,
    #[serde(default)]
    size: Value,
    price: String,
    fee: Value,
    #[serde(default, rename = "point_fee")]
    point_fee: Value,
    #[serde(rename = "create_time_ms")]
    create_time_ms: i64,
}

fn value_number(value: &Value, field: &str) -> ExchangeResult<f64> {
    match value {
        Value::Number(number) => number.as_f64().ok_or_else(|| {
            ExchangeError::Parse(format!(
                "gate user ws non-f64 numeric field {field}: {number}"
            ))
        }),
        Value::String(text) => parse_number(text, field),
        Value::Null => Ok(0.0),
        other => Err(ExchangeError::Parse(format!(
            "gate user ws unsupported numeric field {field}: {other}"
        ))),
    }
}

fn finite_value_number(value: &Value, field: &str) -> ExchangeResult<f64> {
    let number = value_number(value, field)?;
    if number.is_finite() {
        Ok(number)
    } else {
        Err(ExchangeError::Parse(format!(
            "gate user ws non-finite numeric field {field}: {value}"
        )))
    }
}

fn required_value_number(value: &Value, field: &str) -> ExchangeResult<f64> {
    if matches!(value, Value::Null) {
        Err(ExchangeError::Parse(format!(
            "gate user ws missing numeric field {field}"
        )))
    } else {
        let number = value_number(value, field)?;
        if number.is_finite() {
            Ok(number)
        } else {
            Err(ExchangeError::Parse(format!(
                "gate user ws non-finite numeric field {field}: {value}"
            )))
        }
    }
}
