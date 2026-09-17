//! Binance USD-M Futures user stream parser internals.

use super::binance_ws_user::{
    BinanceAccountBalanceDelta, BinanceAccountUpdate, BinanceOrderTradeUpdate,
    BinancePositionDelta, BinanceUserEvent, EVENT_ACCOUNT_UPDATE, EVENT_ORDER_UPDATE, EXCHANGE,
};
use crate::adapter::{client_order_id_from_str, strip_common_suffixes};
use crate::error::{ExchangeError, ExchangeResult};
use chrono::Utc;
use serde::Deserialize;
use shared_types::{LiveOrderState, OrderInfo, OrderSide, OrderStatus, OrderType};

pub(super) fn parse_user_event(text: &str) -> ExchangeResult<Option<BinanceUserEvent>> {
    let envelope: RawUserEnvelope = serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!("binance user ws event: {error}; body={text}"))
    })?;
    match envelope.event_type.as_str() {
        EVENT_ACCOUNT_UPDATE => parse_account_update(envelope).map(Some),
        EVENT_ORDER_UPDATE => parse_order_update(envelope).map(Some),
        _ => Ok(None),
    }
}

fn parse_account_update(envelope: RawUserEnvelope) -> ExchangeResult<BinanceUserEvent> {
    let data = envelope
        .account
        .ok_or_else(|| ExchangeError::Parse("binance account update missing a".into()))?;
    let event_time_ms = parse_timestamp_ms(envelope.event_time_ms, "E")?;
    let transaction_time_ms = parse_required_timestamp_ms(envelope.transaction_time_ms, "T")?;
    Ok(BinanceUserEvent::Account(BinanceAccountUpdate {
        event_time_ms,
        transaction_time_ms,
        reason: required_text(data.reason, "a.m")?,
        balances: parse_balances(data.balances)?,
        positions: parse_positions(data.positions)?,
    }))
}

fn parse_order_update(envelope: RawUserEnvelope) -> ExchangeResult<BinanceUserEvent> {
    let update = envelope
        .order
        .ok_or_else(|| ExchangeError::Parse("binance order update missing o".into()))?;
    let event_time_ms = parse_timestamp_ms(envelope.event_time_ms, "E")?;
    let order_status = canonical_order_status(&update.order_status)?;
    let status = order_status_from_canonical(&order_status);
    let execution_type = canonical_execution_type(&update.execution_type)?;
    let transaction_time_ms =
        parse_order_transaction_time(envelope.transaction_time_ms, update.trade_time_ms)?;
    let last_filled_quantity = parse_number(&update.last_filled_quantity, "o.l")?;
    let accumulated_filled_quantity = parse_number(&update.accumulated_filled_quantity, "o.z")?;
    let last_filled_price = parse_number(&update.last_filled_price, "o.L")?;
    let commission = parse_number(&update.commission, "o.n")?;
    let commission_asset = clean_optional_text(update.commission_asset);
    let trade_id = parse_trade_identity(
        &execution_type,
        &TradeEvidence {
            trade_id: update.trade_id,
            last_quantity: last_filled_quantity,
            last_price: last_filled_price,
            cumulative_quantity: accumulated_filled_quantity,
            commission,
            commission_asset: commission_asset.as_deref(),
        },
    )?;
    let order = OrderInfo {
        execution_style: None,
        venue_time_in_force: Some(update.time_in_force.clone()),
        client_order_id: client_order_id_from_str(&update.client_order_id),
        reduce_only: update.reduce_only,
        order_id: parse_order_id(update.order_id)?.to_string(),
        symbol: strip_common_suffixes(&required_text(update.symbol, "o.s")?),
        exchange: EXCHANGE.into(),
        side: order_side(&update.side)?,
        order_type: order_type(&update.order_type, &update.time_in_force)?,
        status,
        quantity: parse_number(&update.original_quantity, "o.q")?,
        price: parse_number(&update.original_price, "o.p")?,
        filled_quantity: accumulated_filled_quantity,
        filled_price: parse_number(&update.average_price, "o.ap")?,
        fees: commission,
        created_at: event_time(transaction_time_ms)?,
    };
    Ok(BinanceUserEvent::Order(Box::new(BinanceOrderTradeUpdate {
        event_time_ms,
        transaction_time_ms,
        trade_time_ms: parse_timestamp_ms(update.trade_time_ms, "o.T")?,
        client_order_id: required_text(update.client_order_id, "o.c")?,
        execution_type,
        order_status,
        reject_reason: clean_optional_text(update.reject_reason),
        trade_id,
        last_filled_quantity,
        last_filled_price,
        commission_asset,
        live_state: live_state(status),
        order,
    })))
}

fn parse_balances(rows: Vec<RawBalanceDelta>) -> ExchangeResult<Vec<BinanceAccountBalanceDelta>> {
    rows.into_iter().map(balance_delta).collect()
}

fn parse_positions(rows: Vec<RawPositionDelta>) -> ExchangeResult<Vec<BinancePositionDelta>> {
    rows.into_iter().map(position_delta).collect()
}

fn balance_delta(row: RawBalanceDelta) -> ExchangeResult<BinanceAccountBalanceDelta> {
    Ok(BinanceAccountBalanceDelta {
        asset: required_text(row.asset, "a.B.a")?,
        wallet_balance: parse_number(&row.wallet_balance, "a.B.wb")?,
        cross_wallet_balance: parse_number(&row.cross_wallet_balance, "a.B.cw")?,
        balance_change: parse_number(&row.balance_change, "a.B.bc")?,
    })
}

fn position_delta(row: RawPositionDelta) -> ExchangeResult<BinancePositionDelta> {
    Ok(BinancePositionDelta {
        symbol: strip_common_suffixes(&required_text(row.symbol, "a.P.s")?),
        side: position_side(&row.position_side)?,
        quantity: parse_number(&row.position_amount, "a.P.pa")?,
        entry_price: parse_number(&row.entry_price, "a.P.ep")?,
        accumulated_realized: parse_number(&row.accumulated_realized, "a.P.cr")?,
        unrealized_pnl: parse_number(&row.unrealized_pnl, "a.P.up")?,
        margin_type: row.margin_type,
        isolated_wallet: parse_number(&row.isolated_wallet, "a.P.iw")?,
    })
}

fn parse_number(raw: &str, field: &str) -> ExchangeResult<f64> {
    let parsed = raw.parse::<f64>().map_err(|error| {
        ExchangeError::Parse(format!(
            "binance user ws numeric field {field}: {error}; value={raw}"
        ))
    })?;
    if !parsed.is_finite() {
        return Err(ExchangeError::Parse(format!(
            "binance user ws numeric field {field}: non-finite value={raw}"
        )));
    }
    Ok(parsed)
}

fn event_time(timestamp_ms: i64) -> ExchangeResult<chrono::DateTime<Utc>> {
    chrono::DateTime::<Utc>::from_timestamp_millis(timestamp_ms).ok_or_else(|| {
        ExchangeError::Parse(format!("binance user ws invalid timestamp: {timestamp_ms}"))
    })
}

fn parse_timestamp_ms(timestamp_ms: i64, field: &str) -> ExchangeResult<i64> {
    if timestamp_ms <= 0 {
        return Err(ExchangeError::Parse(format!(
            "binance user ws invalid timestamp field {field}: {timestamp_ms}"
        )));
    }
    Ok(timestamp_ms)
}

fn parse_required_timestamp_ms(value: Option<i64>, field: &str) -> ExchangeResult<i64> {
    parse_timestamp_ms(value.unwrap_or_default(), field)
}

fn parse_order_transaction_time(outer: Option<i64>, inner: i64) -> ExchangeResult<i64> {
    match outer {
        Some(value) => parse_timestamp_ms(value, "T"),
        None => parse_timestamp_ms(inner, "o.T"),
    }
}

fn parse_order_id(order_id: i64) -> ExchangeResult<i64> {
    if order_id <= 0 {
        return Err(ExchangeError::Parse(format!(
            "binance user ws order id invalid: {order_id}"
        )));
    }
    Ok(order_id)
}

struct TradeEvidence<'a> {
    trade_id: i64,
    last_quantity: f64,
    last_price: f64,
    cumulative_quantity: f64,
    commission: f64,
    commission_asset: Option<&'a str>,
}

fn parse_trade_identity(
    execution_type: &str,
    evidence: &TradeEvidence<'_>,
) -> ExchangeResult<Option<i64>> {
    if execution_type != "TRADE" {
        return Ok(None);
    }
    if evidence.trade_id <= 0
        || evidence.last_quantity <= 0.0
        || evidence.last_price <= 0.0
        || evidence.cumulative_quantity < evidence.last_quantity
        || (evidence.commission.abs() > f64::EPSILON && evidence.commission_asset.is_none())
    {
        return Err(ExchangeError::Parse(format!(
            "binance user ws incoherent TRADE fill: trade_id={} last_quantity={} last_price={} cumulative_quantity={} commission={} commission_asset={:?}",
            evidence.trade_id,
            evidence.last_quantity,
            evidence.last_price,
            evidence.cumulative_quantity,
            evidence.commission,
            evidence.commission_asset,
        )));
    }
    Ok(Some(evidence.trade_id))
}

fn required_text(value: String, field: &str) -> ExchangeResult<String> {
    if value.trim().is_empty() {
        return Err(ExchangeError::Parse(format!(
            "binance user ws text field {field} missing"
        )));
    }
    Ok(value)
}

fn clean_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let trimmed = text.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

fn order_side(raw: &str) -> ExchangeResult<OrderSide> {
    match raw.to_ascii_uppercase().as_str() {
        "BUY" => Ok(OrderSide::Buy),
        "SELL" => Ok(OrderSide::Sell),
        _ => Err(ExchangeError::Parse(format!(
            "binance user ws order side invalid: {raw}"
        ))),
    }
}

fn order_type(raw_type: &str, time_in_force: &str) -> ExchangeResult<OrderType> {
    match (
        raw_type.to_ascii_uppercase().as_str(),
        time_in_force.to_ascii_uppercase().as_str(),
    ) {
        ("MARKET", _) => Ok(OrderType::Market),
        ("LIMIT", "GTX") => Ok(OrderType::PostOnly),
        ("LIMIT", "GTC" | "IOC" | "FOK") => Ok(OrderType::Limit),
        _ => Err(ExchangeError::Parse(format!(
            "binance user ws order type invalid: type={raw_type} tif={time_in_force}"
        ))),
    }
}

fn canonical_order_status(raw: &str) -> ExchangeResult<String> {
    match raw.to_ascii_uppercase().as_str() {
        "NEW" | "PARTIALLY_FILLED" | "FILLED" | "CANCELED" | "EXPIRED" | "EXPIRED_IN_MATCH" => {
            Ok(raw.to_ascii_uppercase())
        }
        _ => Err(ExchangeError::Parse(format!(
            "binance user ws order status invalid: {raw}"
        ))),
    }
}

fn order_status_from_canonical(raw: &str) -> OrderStatus {
    match raw {
        "NEW" => OrderStatus::Open,
        "PARTIALLY_FILLED" => OrderStatus::PartiallyFilled,
        "FILLED" => OrderStatus::Filled,
        "CANCELED" => OrderStatus::Canceled,
        "EXPIRED" | "EXPIRED_IN_MATCH" => OrderStatus::Expired,
        _ => unreachable!("canonical order status is exhaustively validated"),
    }
}

fn canonical_execution_type(raw: &str) -> ExchangeResult<String> {
    match raw.to_ascii_uppercase().as_str() {
        "NEW" | "CANCELED" | "CALCULATED" | "EXPIRED" | "TRADE" | "AMENDMENT" => {
            Ok(raw.to_ascii_uppercase())
        }
        _ => Err(ExchangeError::Parse(format!(
            "binance user ws execution type invalid: {raw}"
        ))),
    }
}

fn position_side(raw: &str) -> ExchangeResult<String> {
    match raw.to_ascii_uppercase().as_str() {
        "BOTH" | "LONG" | "SHORT" => Ok(raw.to_owned()),
        _ => Err(ExchangeError::Parse(format!(
            "binance user ws position side invalid: {raw}"
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
struct RawUserEnvelope {
    #[serde(rename = "e")]
    event_type: String,
    #[serde(default, rename = "E")]
    event_time_ms: i64,
    #[serde(default, rename = "T")]
    transaction_time_ms: Option<i64>,
    #[serde(default, rename = "a")]
    account: Option<RawAccountUpdate>,
    #[serde(default, rename = "o")]
    order: Option<RawOrderUpdate>,
}

#[derive(Debug, Deserialize)]
struct RawAccountUpdate {
    #[serde(default, rename = "m")]
    reason: String,
    #[serde(default, rename = "B")]
    balances: Vec<RawBalanceDelta>,
    #[serde(default, rename = "P")]
    positions: Vec<RawPositionDelta>,
}

#[derive(Debug, Deserialize)]
struct RawBalanceDelta {
    #[serde(default, rename = "a")]
    asset: String,
    #[serde(default, rename = "wb")]
    wallet_balance: String,
    #[serde(default, rename = "cw")]
    cross_wallet_balance: String,
    #[serde(default, rename = "bc")]
    balance_change: String,
}

#[derive(Debug, Deserialize)]
struct RawPositionDelta {
    #[serde(default, rename = "s")]
    symbol: String,
    #[serde(default, rename = "pa")]
    position_amount: String,
    #[serde(default, rename = "ep")]
    entry_price: String,
    #[serde(default, rename = "cr")]
    accumulated_realized: String,
    #[serde(default, rename = "up")]
    unrealized_pnl: String,
    #[serde(default, rename = "mt")]
    margin_type: String,
    #[serde(default, rename = "iw")]
    isolated_wallet: String,
    #[serde(default, rename = "ps")]
    position_side: String,
}

#[derive(Debug, Deserialize)]
struct RawOrderUpdate {
    #[serde(default, rename = "s")]
    symbol: String,
    #[serde(default, rename = "c")]
    client_order_id: String,
    #[serde(default, rename = "R")]
    reduce_only: Option<bool>,
    #[serde(default, rename = "S")]
    side: String,
    #[serde(default, rename = "o")]
    order_type: String,
    #[serde(default, rename = "f")]
    time_in_force: String,
    #[serde(default, rename = "q")]
    original_quantity: String,
    #[serde(default, rename = "p")]
    original_price: String,
    #[serde(default, rename = "ap")]
    average_price: String,
    #[serde(default, rename = "x")]
    execution_type: String,
    #[serde(default, rename = "X")]
    order_status: String,
    #[serde(default, rename = "r")]
    reject_reason: Option<String>,
    #[serde(default, rename = "i")]
    order_id: i64,
    #[serde(default, rename = "l")]
    last_filled_quantity: String,
    #[serde(default, rename = "z")]
    accumulated_filled_quantity: String,
    #[serde(default, rename = "L")]
    last_filled_price: String,
    #[serde(default, rename = "n")]
    commission: String,
    #[serde(default, rename = "N")]
    commission_asset: Option<String>,
    #[serde(default, rename = "T")]
    trade_time_ms: i64,
    #[serde(default, rename = "t")]
    trade_id: i64,
}
