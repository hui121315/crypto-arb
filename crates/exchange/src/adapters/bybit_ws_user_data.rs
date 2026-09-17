//! Bybit V5 private WebSocket event parser internals.

use super::bybit_ws_user::{
    BybitExecutionExtraFee, BybitExecutionUpdate, BybitOrderFinalityEvidence, BybitOrderUpdate,
    BybitPositionDelta, BybitPositionUpdate, BybitUserEvent, BybitWalletAccount,
    BybitWalletCoinDelta, BybitWalletUpdate, EXCHANGE, TOPIC_EXECUTION, TOPIC_ORDER,
    TOPIC_POSITION, TOPIC_WALLET,
};
use crate::adapter::{client_order_id_from_str, strip_common_suffixes};
use crate::error::{ExchangeError, ExchangeResult};
use chrono::Utc;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;
use shared_types::{LiveOrderState, OrderInfo, OrderSide, OrderStatus, OrderType};

pub(super) fn parse_user_event(text: &str) -> ExchangeResult<Option<BybitUserEvent>> {
    let envelope: RawEnvelope = serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!("bybit user ws event: {error}; body={text}"))
    })?;
    match base_topic(&envelope.topic) {
        TOPIC_ORDER => parse_event_rows(envelope, order_update)
            .map(BybitUserEvent::Order)
            .map(Some),
        TOPIC_EXECUTION => parse_event_rows(envelope, execution_update)
            .map(BybitUserEvent::Execution)
            .map(Some),
        TOPIC_POSITION => position_update(envelope)
            .map(BybitUserEvent::Position)
            .map(Some),
        TOPIC_WALLET => wallet_update_event(envelope)
            .map(BybitUserEvent::Wallet)
            .map(Some),
        _ => Ok(None),
    }
}

fn position_update(envelope: RawEnvelope) -> ExchangeResult<BybitPositionUpdate> {
    let update_type = envelope.update_type.clone();
    let positions = parse_event_rows(envelope, position_delta)?;
    Ok(BybitPositionUpdate {
        update_type,
        positions,
    })
}

fn parse_event_rows<T, O>(
    envelope: RawEnvelope,
    convert: fn(T) -> ExchangeResult<O>,
) -> ExchangeResult<Vec<O>>
where
    T: DeserializeOwned,
{
    let data = envelope.data.ok_or_else(|| {
        ExchangeError::Parse(format!(
            "bybit user ws {} missing data array",
            envelope.topic
        ))
    })?;
    let rows: Vec<T> = serde_json::from_value(Value::Array(data)).map_err(|error| {
        ExchangeError::Parse(format!("bybit user ws {} rows: {error}", envelope.topic))
    })?;
    rows.into_iter().map(convert).collect()
}

fn order_update(row: RawOrderUpdate) -> ExchangeResult<BybitOrderUpdate> {
    let order_type = order_type(&row.order_type, &row.time_in_force)?;
    let status = order_status(&row.order_status)?;
    let created_at = event_time(parse_required_i64(&row.created_time, "order.createdTime")?)?;
    let finality = order_finality_evidence(&row)?;
    let order = OrderInfo {
        execution_style: None,
        venue_time_in_force: Some(row.time_in_force.clone()),
        client_order_id: client_order_id_from_str(&row.order_link_id),
        reduce_only: row.reduce_only,
        order_id: parse_required_text(&row.order_id, "order.orderId")?,
        symbol: strip_common_suffixes(&parse_required_text(&row.symbol, "order.symbol")?),
        exchange: EXCHANGE.into(),
        side: order_side(&row.side)?,
        order_type,
        status,
        quantity: parse_number(&row.qty, "order.qty")?,
        price: parse_order_price(order_type, &row.price)?,
        filled_quantity: parse_number(&row.cum_exec_qty, "order.cumExecQty")?,
        filled_price: parse_optional_zero_number(&row.avg_price, "order.avgPrice")?,
        fees: parse_optional_zero_number(&row.cum_exec_fee, "order.cumExecFee")?,
        created_at,
    };
    Ok(BybitOrderUpdate {
        client_order_id: row.order_link_id,
        live_state: live_state(status),
        order,
        finality,
    })
}

fn order_finality_evidence(row: &RawOrderUpdate) -> ExchangeResult<BybitOrderFinalityEvidence> {
    Ok(BybitOrderFinalityEvidence {
        position_idx: row.position_idx,
        cancel_type: optional_text(&row.cancel_type),
        reject_reason: optional_text(&row.reject_reason),
        leaves_quantity: parse_option_number(&row.leaves_qty, "order.leavesQty")?,
        reduce_only: row.reduce_only,
        time_in_force: row.time_in_force.clone(),
    })
}

fn execution_update(row: RawExecutionUpdate) -> ExchangeResult<BybitExecutionUpdate> {
    Ok(BybitExecutionUpdate {
        order_id: parse_required_text(&row.order_id, "execution.orderId")?,
        client_order_id: row.order_link_id,
        exec_id: parse_required_text(&row.exec_id, "execution.execId")?,
        symbol: strip_common_suffixes(&parse_required_text(&row.symbol, "execution.symbol")?),
        category: parse_required_text(&row.category, "execution.category")?,
        side: parse_required_text(&row.side, "execution.side")?,
        price: parse_positive_number(&row.exec_price, "execution.execPrice")?,
        size: parse_positive_number(&row.exec_qty, "execution.execQty")?,
        fee: parse_option_number(&row.exec_fee, "execution.execFee")?,
        fee_currency: optional_text(&row.fee_currency),
        fee_rate: parse_option_number(&row.fee_rate, "execution.feeRate")?,
        extra_fees: execution_extra_fees(row.extra_fees)?,
        trade_time_ms: parse_required_i64(&row.exec_time, "execution.execTime")?,
        is_maker: row.is_maker,
        seq: row.seq,
    })
}

fn execution_extra_fees(raw: Option<Value>) -> ExchangeResult<Vec<BybitExecutionExtraFee>> {
    match raw {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::String(value)) if value.trim().is_empty() => Ok(Vec::new()),
        Some(Value::Array(rows)) => rows
            .into_iter()
            .map(execution_extra_fee_from_value)
            .collect(),
        Some(value) => Err(ExchangeError::Parse(format!(
            "bybit user ws field execution.extraFees invalid: {value}"
        ))),
    }
}

fn execution_extra_fee_from_value(value: Value) -> ExchangeResult<BybitExecutionExtraFee> {
    let row: RawExecutionExtraFee = serde_json::from_value(value).map_err(|error| {
        ExchangeError::Parse(format!("bybit user ws execution.extraFees row: {error}"))
    })?;
    Ok(BybitExecutionExtraFee {
        fee_coin: parse_required_text(&row.fee_coin, "execution.extraFees.feeCoin")?,
        fee_type: parse_required_text(&row.fee_type, "execution.extraFees.feeType")?,
        sub_fee_type: parse_required_text(&row.sub_fee_type, "execution.extraFees.subFeeType")?,
        fee_rate: parse_option_number(&row.fee_rate, "execution.extraFees.feeRate")?,
        fee: parse_option_number(&row.fee, "execution.extraFees.fee")?,
    })
}

fn position_delta(row: RawPositionDelta) -> ExchangeResult<BybitPositionDelta> {
    let RawPositionDelta {
        category,
        symbol,
        side,
        size,
        entry_price,
        mark_price,
        unrealized_pnl,
        leverage,
        liquidation_price,
        updated_time,
    } = row;
    let size = parse_number(&size, "position.size")?;
    Ok(BybitPositionDelta {
        symbol: strip_common_suffixes(&parse_required_text(&symbol, "position.symbol")?),
        category: parse_required_text(&category, "position.category")?,
        side: position_side(&side, size)?,
        size,
        entry_price: parse_number(&entry_price, "position.entryPrice")?,
        mark_price: parse_number(&mark_price, "position.markPrice")?,
        unrealized_pnl: parse_number(&unrealized_pnl, "position.unrealisedPnl")?,
        leverage: parse_optional_zero_number(&leverage, "position.leverage")?.max(1.0),
        liquidation_price: parse_positive_option(&liquidation_price, "position.liqPrice")?,
        updated_time_ms: parse_required_i64(&updated_time, "position.updatedTime")?,
    })
}

fn wallet_update_event(envelope: RawEnvelope) -> ExchangeResult<BybitWalletUpdate> {
    let update_type = envelope.update_type.clone();
    let observed_at_ms = envelope.creation_time_ms;
    if observed_at_ms <= 0 {
        return Err(parse_error(
            "wallet.creationTime",
            &observed_at_ms.to_string(),
        ));
    }
    let accounts = parse_event_rows(envelope, wallet_account)?;
    Ok(BybitWalletUpdate {
        update_type,
        observed_at_ms,
        accounts,
    })
}

fn wallet_account(row: RawWalletUpdate) -> ExchangeResult<BybitWalletAccount> {
    if row.coin.is_empty() {
        return Err(ExchangeError::Parse(
            "bybit user ws wallet.coin missing or empty".into(),
        ));
    }
    Ok(BybitWalletAccount {
        account_type: parse_wallet_account_type(&row.account_type)?,
        total_equity: parse_number(&row.total_equity, "wallet.totalEquity")?,
        total_available_balance: parse_number(
            &row.total_available_balance,
            "wallet.totalAvailableBalance",
        )?,
        total_initial_margin: parse_non_negative_number(
            &row.total_initial_margin,
            "wallet.totalInitialMargin",
        )?,
        total_maintenance_margin: parse_non_negative_number(
            &row.total_maintenance_margin,
            "wallet.totalMaintenanceMargin",
        )?,
        account_im_rate: parse_non_negative_number(&row.account_im_rate, "wallet.accountIMRate")?,
        account_mm_rate: parse_non_negative_number(&row.account_mm_rate, "wallet.accountMMRate")?,
        coins: row
            .coin
            .into_iter()
            .map(wallet_coin_delta)
            .collect::<ExchangeResult<Vec<_>>>()?,
    })
}

fn wallet_coin_delta(row: RawWalletCoinDelta) -> ExchangeResult<BybitWalletCoinDelta> {
    let RawWalletCoinDelta {
        coin,
        equity,
        usd_value,
        wallet_balance,
        available_to_withdraw,
        locked,
        unrealized_pnl,
    } = row;
    Ok(BybitWalletCoinDelta {
        coin: parse_required_text(&coin, "wallet.coin.coin")?,
        equity: parse_number(&equity, "wallet.coin.equity")?,
        usd_value: parse_number(&usd_value, "wallet.coin.usdValue")?,
        wallet_balance: parse_number(&wallet_balance, "wallet.coin.walletBalance")?,
        available_to_withdraw: parse_option_number(
            &available_to_withdraw,
            "wallet.coin.availableToWithdraw",
        )?,
        locked: parse_number(&locked, "wallet.coin.locked")?,
        unrealized_pnl: parse_number(&unrealized_pnl, "wallet.coin.unrealisedPnl")?,
    })
}

fn base_topic(topic: &str) -> &str {
    topic.split('.').next().unwrap_or(topic)
}

fn parse_number(raw: &str, field: &str) -> ExchangeResult<f64> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(parse_error(field, raw));
    }
    parse_finite_number(field, value)
}

fn parse_optional_zero_number(raw: &str, field: &str) -> ExchangeResult<f64> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(0.0);
    }
    parse_finite_number(field, value)
}

fn parse_finite_number(field: &str, value: &str) -> ExchangeResult<f64> {
    value
        .parse()
        .map_err(|error| {
            ExchangeError::Parse(format!(
                "bybit user ws numeric field {field}: {error}; value={value}"
            ))
        })
        .and_then(|parsed: f64| {
            if parsed.is_finite() {
                Ok(parsed)
            } else {
                Err(parse_error(field, value))
            }
        })
}

fn parse_positive_number(raw: &str, field: &str) -> ExchangeResult<f64> {
    let value = parse_number(raw, field)?;
    if value > 0.0 {
        Ok(value)
    } else {
        Err(parse_error(field, raw))
    }
}

fn parse_non_negative_number(raw: &str, field: &str) -> ExchangeResult<f64> {
    let value = parse_number(raw, field)?;
    if value >= 0.0 {
        Ok(value)
    } else {
        Err(parse_error(field, raw))
    }
}

fn parse_option_number(raw: &str, field: &str) -> ExchangeResult<Option<f64>> {
    if raw.trim().is_empty() {
        return Ok(None);
    }
    parse_number(raw, field).map(Some)
}

fn parse_positive_option(raw: &str, field: &str) -> ExchangeResult<Option<f64>> {
    let Some(value) = parse_option_number(raw, field)? else {
        return Ok(None);
    };
    Ok((value > 0.0).then_some(value))
}

fn parse_required_i64(raw: &str, field: &str) -> ExchangeResult<i64> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(parse_error(field, raw));
    }
    let parsed = value.parse().map_err(|error| {
        ExchangeError::Parse(format!(
            "bybit user ws integer field {field}: {error}; value={raw}"
        ))
    })?;
    if parsed <= 0 {
        return Err(parse_error(field, raw));
    }
    Ok(parsed)
}

fn event_time(timestamp_ms: i64) -> ExchangeResult<chrono::DateTime<Utc>> {
    chrono::DateTime::<Utc>::from_timestamp_millis(timestamp_ms).ok_or_else(|| {
        ExchangeError::Parse(format!("bybit user ws invalid timestamp: {timestamp_ms}"))
    })
}

fn order_side(raw: &str) -> ExchangeResult<OrderSide> {
    match raw {
        "Buy" => Ok(OrderSide::Buy),
        "Sell" => Ok(OrderSide::Sell),
        _ => Err(parse_error("order.side", raw)),
    }
}

fn order_type(raw_type: &str, time_in_force: &str) -> ExchangeResult<OrderType> {
    let tif = parse_time_in_force(time_in_force)?;
    match (raw_type, tif) {
        ("Market", TimeInForce::Gtc | TimeInForce::Ioc | TimeInForce::Fok) => Ok(OrderType::Market),
        ("Limit", TimeInForce::Gtc | TimeInForce::Ioc | TimeInForce::Fok) => Ok(OrderType::Limit),
        ("Limit", TimeInForce::PostOnly) => Ok(OrderType::PostOnly),
        ("Limit", TimeInForce::Rpi) => Err(ExchangeError::UnsupportedCapability(
            "bybit rpi order stream order type",
        )),
        ("Market", TimeInForce::PostOnly | TimeInForce::Rpi) => {
            Err(parse_error("order.orderType/timeInForce", raw_type))
        }
        _ => Err(parse_error("order.orderType", raw_type)),
    }
}

fn parse_time_in_force(raw: &str) -> ExchangeResult<TimeInForce> {
    match raw {
        "GTC" => Ok(TimeInForce::Gtc),
        "IOC" => Ok(TimeInForce::Ioc),
        "FOK" => Ok(TimeInForce::Fok),
        "PostOnly" => Ok(TimeInForce::PostOnly),
        "RPI" => Ok(TimeInForce::Rpi),
        _ => Err(parse_error("order.timeInForce", raw)),
    }
}

fn order_status(raw: &str) -> ExchangeResult<OrderStatus> {
    match raw {
        "New" | "Untriggered" => Ok(OrderStatus::Open),
        "Triggered" => Ok(OrderStatus::Pending),
        "PartiallyFilled" => Ok(OrderStatus::PartiallyFilled),
        "Filled" => Ok(OrderStatus::Filled),
        "Cancelled" | "PartiallyFilledCanceled" | "Deactivated" => Ok(OrderStatus::Canceled),
        "Rejected" => Ok(OrderStatus::Rejected),
        "Expired" => Ok(OrderStatus::Expired),
        _ => Err(parse_error("order.orderStatus", raw)),
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

fn position_side(raw: &str, size: f64) -> ExchangeResult<String> {
    match raw {
        "Buy" => Ok("long".to_owned()),
        "Sell" => Ok("short".to_owned()),
        "" if size == 0.0 => Ok(String::new()),
        _ => Err(parse_error("position.side", raw)),
    }
}

fn parse_order_price(order_type: OrderType, raw: &str) -> ExchangeResult<f64> {
    match order_type {
        OrderType::Market => parse_optional_zero_number(raw, "order.price"),
        OrderType::Limit | OrderType::PostOnly => parse_number(raw, "order.price"),
    }
}

fn parse_required_text(raw: &str, field: &str) -> ExchangeResult<String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(parse_error(field, raw));
    }
    Ok(value.to_owned())
}

fn optional_text(raw: &str) -> Option<String> {
    let value = raw.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn parse_wallet_account_type(raw: &str) -> ExchangeResult<String> {
    match raw {
        "UNIFIED" => Ok(raw.to_owned()),
        _ => Err(parse_error("wallet.accountType", raw)),
    }
}

fn parse_error(field: &str, value: &str) -> ExchangeError {
    ExchangeError::Parse(format!("bybit user ws field {field} invalid: {value}"))
}

enum TimeInForce {
    Gtc,
    Ioc,
    Fok,
    PostOnly,
    Rpi,
}

#[derive(Debug, Deserialize)]
struct RawEnvelope {
    #[serde(default)]
    topic: String,
    /// Bybit V5 envelope `type` field: `"snapshot"` for the first full push
    /// per category, `"delta"` for incremental updates. Preserved so the
    /// mapper can run safe full-cache replacement only on a snapshot.
    #[serde(default, rename = "type")]
    update_type: String,
    #[serde(default)]
    data: Option<Vec<Value>>,
    #[serde(default, rename = "creationTime")]
    creation_time_ms: i64,
}

#[derive(Debug, Deserialize)]
struct RawOrderUpdate {
    #[serde(default, rename = "orderId")]
    order_id: String,
    #[serde(default, rename = "orderLinkId")]
    order_link_id: String,
    #[serde(default)]
    symbol: String,
    #[serde(default)]
    side: String,
    #[serde(default, rename = "orderType")]
    order_type: String,
    #[serde(default, rename = "timeInForce")]
    time_in_force: String,
    #[serde(default, rename = "orderStatus")]
    order_status: String,
    #[serde(default)]
    qty: String,
    #[serde(default)]
    price: String,
    #[serde(default, rename = "cumExecQty")]
    cum_exec_qty: String,
    #[serde(default, rename = "avgPrice")]
    avg_price: String,
    #[serde(default, rename = "cumExecFee")]
    cum_exec_fee: String,
    #[serde(default, rename = "createdTime")]
    created_time: String,
    #[serde(default, rename = "positionIdx")]
    position_idx: Option<i32>,
    #[serde(default, rename = "cancelType")]
    cancel_type: String,
    #[serde(default, rename = "rejectReason")]
    reject_reason: String,
    #[serde(default, rename = "leavesQty")]
    leaves_qty: String,
    #[serde(default, rename = "reduceOnly")]
    reduce_only: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct RawExecutionUpdate {
    #[serde(default)]
    category: String,
    #[serde(default, rename = "orderId")]
    order_id: String,
    #[serde(default, rename = "orderLinkId")]
    order_link_id: String,
    #[serde(default, rename = "execId")]
    exec_id: String,
    #[serde(default)]
    symbol: String,
    #[serde(default)]
    side: String,
    #[serde(default, rename = "execPrice")]
    exec_price: String,
    #[serde(default, rename = "execQty")]
    exec_qty: String,
    #[serde(default, rename = "execFee")]
    exec_fee: String,
    #[serde(default, rename = "feeCurrency")]
    fee_currency: String,
    #[serde(default, rename = "feeRate")]
    fee_rate: String,
    #[serde(default, rename = "extraFees")]
    extra_fees: Option<Value>,
    #[serde(default, rename = "execTime")]
    exec_time: String,
    #[serde(default, rename = "isMaker")]
    is_maker: Option<bool>,
    #[serde(default)]
    seq: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct RawExecutionExtraFee {
    #[serde(default, rename = "feeCoin")]
    fee_coin: String,
    #[serde(default, rename = "feeType")]
    fee_type: String,
    #[serde(default, rename = "subFeeType")]
    sub_fee_type: String,
    #[serde(default, rename = "feeRate")]
    fee_rate: String,
    #[serde(default)]
    fee: String,
}

#[derive(Debug, Deserialize)]
struct RawPositionDelta {
    #[serde(default)]
    category: String,
    #[serde(default)]
    symbol: String,
    #[serde(default)]
    side: String,
    #[serde(default)]
    size: String,
    #[serde(default, rename = "entryPrice")]
    entry_price: String,
    #[serde(default, rename = "markPrice")]
    mark_price: String,
    #[serde(default, rename = "unrealisedPnl")]
    unrealized_pnl: String,
    #[serde(default)]
    leverage: String,
    #[serde(default, rename = "liqPrice")]
    liquidation_price: String,
    #[serde(default, rename = "updatedTime")]
    updated_time: String,
}

#[derive(Debug, Deserialize)]
struct RawWalletUpdate {
    #[serde(default, rename = "accountType")]
    account_type: String,
    #[serde(default, rename = "totalAvailableBalance")]
    total_available_balance: String,
    #[serde(default, rename = "totalEquity")]
    total_equity: String,
    #[serde(default, rename = "totalInitialMargin")]
    total_initial_margin: String,
    #[serde(default, rename = "totalMaintenanceMargin")]
    total_maintenance_margin: String,
    #[serde(default, rename = "accountIMRate")]
    account_im_rate: String,
    #[serde(default, rename = "accountMMRate")]
    account_mm_rate: String,
    #[serde(default)]
    coin: Vec<RawWalletCoinDelta>,
}

#[derive(Debug, Deserialize)]
struct RawWalletCoinDelta {
    #[serde(default)]
    coin: String,
    #[serde(default)]
    equity: String,
    #[serde(default, rename = "usdValue")]
    usd_value: String,
    #[serde(default, rename = "walletBalance")]
    wallet_balance: String,
    #[serde(default, rename = "availableToWithdraw")]
    available_to_withdraw: String,
    #[serde(default)]
    locked: String,
    #[serde(default, rename = "unrealisedPnl")]
    unrealized_pnl: String,
}
