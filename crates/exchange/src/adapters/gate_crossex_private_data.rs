//! Gate `CrossEx` private WebSocket/REST schemas and request compiler.
//!
//! Official contracts:
//! <https://www.gate.com/docs/developers/crossex/ws/en/>
//! <https://www.gate.com/docs/developers/apiv4/en/crossex/>

use super::gate_crossex_symbols::{CrossExBusiness, CrossExRoute};
use crate::error::{ExchangeError, ExchangeResult};
use chrono::{DateTime, Utc};
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shared_types::{
    AccountEquityScope, LiveOrderState, OrderInfo, OrderIntent, OrderStatus,
    OrderSubmissionContext, OrderType, PositionInfo, TimeInForce, VenueAccountSummary,
    VenueBalanceInfo, VenueOrderIdentityUpdate,
};

const EXCHANGE: &str = "gate_crossex";
pub(super) const SUCCESS_CODE: &str = "100000";

#[derive(Debug, Clone, PartialEq)]
pub(super) enum PrivatePush {
    Order(OrderInfo),
    Balance(VenueBalanceInfo),
    Position(PositionUpdate),
    Fill(FillUpdate),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PositionUpdate {
    pub key: String,
    pub row: Option<PositionInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct FillUpdate {
    pub transaction_id: String,
    pub order_id: String,
    pub filled_quantity: f64,
    pub filled_price: f64,
    pub fee: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ApiAck {
    pub order_id: Option<String>,
    pub venue_client_order_id: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct CompiledOrder {
    pub text: String,
    pub symbol: String,
    pub side: &'static str,
    #[serde(rename = "type")]
    pub order_type: &'static str,
    pub time_in_force: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qty: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote_qty: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<String>,
    pub reduce_only: bool,
    pub position_side: &'static str,
}

pub(super) fn compile_order(
    intent: &OrderIntent,
    context: Option<&OrderSubmissionContext>,
    route: &CrossExRoute,
) -> ExchangeResult<CompiledOrder> {
    let text = crate::client_order_id_policy::required_venue_client_order_id(
        EXCHANGE,
        &intent.client_order_id,
    )?;
    let quantity = decimal_string("quantity", intent.quantity)?;
    let order_type = match intent.order_type {
        OrderType::Market => "MARKET",
        OrderType::Limit | OrderType::PostOnly => "LIMIT",
    };
    let time_in_force = if intent.post_only || intent.order_type == OrderType::PostOnly {
        "POC"
    } else {
        match intent.time_in_force {
            TimeInForce::Ioc => "IOC",
            TimeInForce::Fok => "FOK",
            TimeInForce::Gtc => "GTC",
            TimeInForce::Gtx => "POC",
        }
    };
    let price = match intent.order_type {
        OrderType::Market => None,
        OrderType::Limit | OrderType::PostOnly => Some(decimal_string(
            "price",
            intent
                .price
                .ok_or_else(|| validation("limit order requires price"))?,
        )?),
    };
    let spot_market_buy = route.business == CrossExBusiness::Spot
        && intent.order_type == OrderType::Market
        && intent.side == shared_types::OrderSide::Buy;
    let (qty, quote_qty) = if spot_market_buy {
        let quote = context
            .and_then(|value| value.sizing_plan)
            .map(|plan| plan.actual_notional_usd)
            .ok_or_else(|| {
                validation("CrossEx spot market buy requires ticket-bound quote notional evidence")
            })?;
        (None, Some(decimal_string("quote_qty", quote)?))
    } else {
        (Some(quantity), None)
    };
    Ok(CompiledOrder {
        text,
        symbol: route.native_symbol.clone(),
        side: match intent.side {
            shared_types::OrderSide::Buy => "BUY",
            shared_types::OrderSide::Sell => "SELL",
        },
        order_type,
        time_in_force,
        qty,
        quote_qty,
        price,
        reduce_only: intent.reduce_only,
        position_side: "NONE",
    })
}

pub(super) fn parse_private_push(text: &str) -> ExchangeResult<Option<PrivatePush>> {
    let frame: WsFrame = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("Gate CrossEx private frame: {error}")))?;
    if let Some(error) = frame.error.as_ref() {
        return Err(api_error(error.code.clone(), error.message.clone()));
    }
    if matches!(frame.event.as_str(), "subscribe" | "login") {
        if let Some(result) = frame.result.as_ref() {
            if !result.code.is_empty() && result.code != SUCCESS_CODE {
                return Err(api_error(result.code.clone(), result.message.clone()));
            }
        }
    }
    if frame.event == "api" || frame.event == "login" || frame.payload.is_array() {
        return Ok(None);
    }
    match frame.channel.as_str() {
        "order" => parse_order_value(frame.payload)
            .map(PrivatePush::Order)
            .map(Some),
        "asset" => parse_asset_value(frame.payload)
            .map(PrivatePush::Balance)
            .map(Some),
        "position" => parse_position_value(frame.payload)
            .map(PrivatePush::Position)
            .map(Some),
        "usertrades" => parse_fill_value(frame.payload)
            .map(PrivatePush::Fill)
            .map(Some),
        _ => Ok(None),
    }
}

pub(super) fn is_api_response_for(
    text: &str,
    channel: &str,
    request_id: &str,
) -> ExchangeResult<bool> {
    let frame: WsFrame = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("Gate CrossEx api frame: {error}")))?;
    Ok(frame.event == "api" && frame.channel == channel && frame.request_id == request_id)
}

pub(super) fn parse_api_ack(text: &str, channel: &str, request_id: &str) -> ExchangeResult<ApiAck> {
    let frame: WsFrame = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("Gate CrossEx api response: {error}")))?;
    if frame.event != "api" || frame.channel != channel || frame.request_id != request_id {
        return Err(ExchangeError::Parse(format!(
            "Gate CrossEx mismatched api response channel={} request_id={}",
            frame.channel, frame.request_id
        )));
    }
    if let Some(error) = frame.error {
        return Err(api_error(error.code, error.message));
    }
    let result = frame.result.unwrap_or_default();
    if result.code != SUCCESS_CODE {
        return Err(api_error(result.code, result.message));
    }
    Ok(ApiAck {
        order_id: string_field(&frame.payload, "order_id"),
        venue_client_order_id: string_field(&frame.payload, "text"),
        message: result.message,
    })
}

pub(super) fn parse_order_text(text: &str) -> ExchangeResult<OrderInfo> {
    let row: CrossExOrder = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("Gate CrossEx order: {error}")))?;
    order_info(row)
}

pub(super) fn parse_orders_text(text: &str) -> ExchangeResult<Vec<OrderInfo>> {
    let rows: Vec<CrossExOrder> = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("Gate CrossEx open orders: {error}")))?;
    rows.into_iter().map(order_info).collect()
}

pub(super) fn parse_positions_text(text: &str) -> ExchangeResult<Vec<PositionInfo>> {
    let rows: Vec<CrossExPosition> = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("Gate CrossEx positions: {error}")))?;
    rows.iter()
        .map(position_update)
        .filter_map(|result| result.transpose())
        .collect()
}

pub(super) fn parse_account_text(
    text: &str,
    observed_at_ms: i64,
) -> ExchangeResult<(Vec<VenueBalanceInfo>, VenueAccountSummary)> {
    let account: CrossExAccount = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("Gate CrossEx account: {error}")))?;
    let balances = account
        .assets
        .iter()
        .cloned()
        .map(asset_info)
        .collect::<ExchangeResult<Vec<_>>>()?;
    let venue = account_venue(&account.exchange_type);
    let summary = VenueAccountSummary {
        venue,
        account_type: account.account_mode.clone(),
        equity_scope: AccountEquityScope::Unified,
        total_equity_usd: number("margin_balance", &account.margin_balance)?,
        total_available_balance_usd: number("available_margin", &account.available_margin)?,
        withdrawable_balance_usd: None,
        total_initial_margin_usd: number("initial_margin", &account.initial_margin)?,
        total_maintenance_margin_usd: number("maintenance_margin", &account.maintenance_margin)?,
        account_im_rate: number("initial_margin_rate", &account.initial_margin_rate)?,
        account_mm_rate: number("maintenance_margin_rate", &account.maintenance_margin_rate)?,
        source: "gate_crossex.GET /api/v4/crossex/accounts".to_owned(),
        observed_at_ms,
        freshness_ms: Some(0),
        problem: None,
    };
    Ok((balances, summary))
}

pub(super) fn ack_from_api(
    intent: &OrderIntent,
    compiled: &CompiledOrder,
    ack: ApiAck,
) -> shared_types::OrderAck {
    let exchange_order_id = ack.order_id;
    let venue_client_order_id = ack
        .venue_client_order_id
        .unwrap_or_else(|| compiled.text.clone());
    shared_types::OrderAck {
        internal_order_id: intent.id.clone(),
        exchange_order_id: exchange_order_id.clone(),
        client_order_id: intent.client_order_id.clone(),
        identity_update: VenueOrderIdentityUpdate::from_ids(
            intent.client_order_id.clone(),
            venue_client_order_id,
            exchange_order_id,
        ),
        state: LiveOrderState::Accepted,
        accepted_at_ms: common::time::now_ms(),
        message: Some(ack.message),
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
    }
}

fn parse_order_value(value: Value) -> ExchangeResult<OrderInfo> {
    serde_json::from_value(value)
        .map_err(|error| ExchangeError::Parse(format!("Gate CrossEx order push: {error}")))
        .and_then(order_info)
}

fn order_info(row: CrossExOrder) -> ExchangeResult<OrderInfo> {
    let route = CrossExRoute::parse(&row.symbol)?;
    let quantity = number("qty", &row.qty)?;
    let filled_quantity = number("executed_qty", &row.executed_qty)?;
    let tif = row.time_in_force.trim().to_ascii_uppercase();
    let order_type = match (row.order_type.as_str(), tif.as_str()) {
        (_, "POC") => OrderType::PostOnly,
        ("MARKET", _) => OrderType::Market,
        ("LIMIT", _) => OrderType::Limit,
        (other, _) => return Err(validation(format!("unknown order type {other}"))),
    };
    let status = match row.state.as_str() {
        "NEW" | "OPEN" => OrderStatus::Open,
        "PARTIALLY_FILLED" => OrderStatus::PartiallyFilled,
        "FILLED" => OrderStatus::Filled,
        "CANCELED" | "CANCELLED" => OrderStatus::Canceled,
        "EXPIRED" => OrderStatus::Expired,
        "FAIL" | "REJECT" | "REJECTED" => OrderStatus::Rejected,
        other => return Err(validation(format!("unknown order state {other}"))),
    };
    Ok(OrderInfo {
        order_id: row.order_id,
        symbol: route.base.clone(),
        exchange: route.venue(),
        side: match row.side.as_str() {
            "BUY" => shared_types::OrderSide::Buy,
            "SELL" => shared_types::OrderSide::Sell,
            other => return Err(validation(format!("unknown order side {other}"))),
        },
        order_type,
        status,
        quantity: if quantity > 0.0 {
            quantity
        } else {
            filled_quantity
        },
        price: number("price", &row.price)?,
        filled_quantity,
        filled_price: number("executed_avg_price", &row.executed_avg_price)?,
        fees: number("fee", &row.fee)?,
        created_at: timestamp("create_time", &row.create_time)?,
        execution_style: None,
        venue_time_in_force: non_empty(row.time_in_force),
        client_order_id: non_empty(row.text),
        reduce_only: Some(bool_string("reduce_only", &row.reduce_only)?),
    })
}

fn parse_asset_value(value: Value) -> ExchangeResult<VenueBalanceInfo> {
    serde_json::from_value(value)
        .map_err(|error| ExchangeError::Parse(format!("Gate CrossEx asset push: {error}")))
        .and_then(asset_info)
}

fn asset_info(row: CrossExAsset) -> ExchangeResult<VenueBalanceInfo> {
    let total = number("balance", &row.balance)?;
    let available = number("available_balance", &row.available_balance)?;
    Ok(VenueBalanceInfo {
        venue: account_venue(&row.exchange_type),
        currency: row.coin,
        total,
        available,
        frozen: (total - available).max(0.0),
        unrealized_pnl: number("upnl", &row.upnl)?,
    })
}

fn parse_position_value(value: Value) -> ExchangeResult<PositionUpdate> {
    let row: CrossExPosition = serde_json::from_value(value)
        .map_err(|error| ExchangeError::Parse(format!("Gate CrossEx position push: {error}")))?;
    let route = CrossExRoute::parse(&row.symbol)?;
    let key = format!("{}:{}", route.native_symbol, row.position_side);
    Ok(PositionUpdate {
        key,
        row: position_update(&row)?,
    })
}

fn position_update(row: &CrossExPosition) -> ExchangeResult<Option<PositionInfo>> {
    let route = CrossExRoute::parse(&row.symbol)?;
    let exchange = route.venue();
    let quantity = number("position_qty", &row.position_qty)?;
    if quantity == 0.0 || row.position_side == "NONE" {
        return Ok(None);
    }
    let position_value = number("position_value", &row.position_value)?.abs();
    let maintenance_margin = number("maintenance_margin", &row.maintenance_margin)?;
    Ok(Some(PositionInfo {
        symbol: route.base,
        exchange,
        side: match row.position_side.as_str() {
            "LONG" => "long".to_owned(),
            "SHORT" => "short".to_owned(),
            other => return Err(validation(format!("unknown position side {other}"))),
        },
        quantity: quantity.abs(),
        entry_price: number("entry_price", &row.entry_price)?,
        mark_price: number("mark_price", &row.mark_price)?,
        unrealized_pnl: number("upnl", &row.upnl)?,
        leverage: number("leverage", &row.leverage)?,
        liquidation_price: None,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin: number("initial_margin", &row.initial_margin)?,
        maintenance_margin_ratio: if position_value > 0.0 {
            maintenance_margin / position_value
        } else {
            0.0
        },
        position_mode: None,
        margin_mode: None,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }))
}

fn parse_fill_value(value: Value) -> ExchangeResult<FillUpdate> {
    let row: CrossExFill = serde_json::from_value(value)
        .map_err(|error| ExchangeError::Parse(format!("Gate CrossEx fill push: {error}")))?;
    Ok(FillUpdate {
        transaction_id: row.transaction_id,
        order_id: row.order_id,
        filled_quantity: number("qty", &row.qty)?,
        filled_price: number("price", &row.price)?,
        fee: number("fee", &row.fee)?,
    })
}

fn decimal_string(field: &str, value: f64) -> ExchangeResult<String> {
    if !value.is_finite() || value <= 0.0 {
        return Err(validation(format!("{field} must be finite and positive")));
    }
    Decimal::from_f64(value)
        .map(|value| value.normalize().to_string())
        .ok_or_else(|| validation(format!("{field} cannot be represented as decimal")))
}

fn number(field: &str, value: &str) -> ExchangeResult<f64> {
    let value = if value.trim().is_empty() {
        "0"
    } else {
        value.trim()
    };
    value
        .parse::<f64>()
        .map_err(|error| validation(format!("{field}={value:?}: {error}")))
        .and_then(|parsed| {
            parsed
                .is_finite()
                .then_some(parsed)
                .ok_or_else(|| validation(format!("{field} is not finite")))
        })
}

fn bool_string(field: &str, value: &str) -> ExchangeResult<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" => Ok(true),
        "false" | "0" | "" => Ok(false),
        other => Err(validation(format!("{field} has invalid boolean {other}"))),
    }
}

fn timestamp(field: &str, value: &str) -> ExchangeResult<DateTime<Utc>> {
    let millis = value
        .parse::<i64>()
        .map_err(|error| validation(format!("{field}={value:?}: {error}")))?;
    DateTime::from_timestamp_millis(millis)
        .ok_or_else(|| validation(format!("{field} is outside chrono range")))
}

fn account_venue(exchange_type: &str) -> String {
    match exchange_type.trim().to_ascii_lowercase().as_str() {
        "" | "crossex" => EXCHANGE.to_owned(),
        underlying => format!("{EXCHANGE}:{underlying}"),
    }
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

fn validation(message: impl Into<String>) -> ExchangeError {
    ExchangeError::Api {
        exchange: EXCHANGE.to_owned(),
        code: "validation".to_owned(),
        message: message.into(),
    }
}

fn api_error(code: String, message: String) -> ExchangeError {
    ExchangeError::Api {
        exchange: EXCHANGE.to_owned(),
        code,
        message,
    }
}

#[derive(Debug, Default, Deserialize)]
struct WsFrame {
    #[serde(default)]
    channel: String,
    #[serde(default)]
    event: String,
    #[serde(default)]
    request_id: String,
    #[serde(default)]
    payload: Value,
    #[serde(default)]
    result: Option<WsResult>,
    #[serde(default)]
    error: Option<WsResult>,
}

#[derive(Debug, Default, Deserialize)]
struct WsResult {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

#[derive(Debug, Clone, Deserialize)]
struct CrossExOrder {
    #[serde(default)]
    order_id: String,
    #[serde(default, alias = "client_order_id")]
    text: String,
    state: String,
    symbol: String,
    side: String,
    #[serde(rename = "type")]
    order_type: String,
    #[serde(default)]
    qty: String,
    #[serde(default)]
    price: String,
    #[serde(default)]
    time_in_force: String,
    #[serde(default)]
    executed_qty: String,
    #[serde(default)]
    executed_avg_price: String,
    #[serde(default)]
    fee: String,
    #[serde(default)]
    reduce_only: String,
    create_time: String,
}

#[derive(Debug, Clone, Deserialize)]
struct CrossExAsset {
    coin: String,
    #[serde(default)]
    exchange_type: String,
    #[serde(default)]
    balance: String,
    #[serde(default)]
    available_balance: String,
    #[serde(default)]
    upnl: String,
}

#[derive(Debug, Deserialize)]
struct CrossExAccount {
    #[serde(default)]
    available_margin: String,
    #[serde(default)]
    margin_balance: String,
    #[serde(default)]
    initial_margin: String,
    #[serde(default)]
    maintenance_margin: String,
    #[serde(default)]
    initial_margin_rate: String,
    #[serde(default)]
    maintenance_margin_rate: String,
    #[serde(default)]
    account_mode: String,
    #[serde(default)]
    exchange_type: String,
    #[serde(default)]
    assets: Vec<CrossExAsset>,
}

#[derive(Debug, Deserialize)]
struct CrossExPosition {
    symbol: String,
    #[serde(default)]
    position_side: String,
    #[serde(default)]
    initial_margin: String,
    #[serde(default)]
    maintenance_margin: String,
    #[serde(default)]
    position_qty: String,
    #[serde(default)]
    position_value: String,
    #[serde(default)]
    upnl: String,
    #[serde(default)]
    entry_price: String,
    #[serde(default)]
    mark_price: String,
    #[serde(default)]
    leverage: String,
}

#[derive(Debug, Deserialize)]
struct CrossExFill {
    transaction_id: String,
    order_id: String,
    #[serde(default)]
    qty: String,
    #[serde(default)]
    price: String,
    #[serde(default)]
    fee: String,
}

#[cfg(test)]
#[path = "gate_crossex_private_data_tests.rs"]
mod tests;
