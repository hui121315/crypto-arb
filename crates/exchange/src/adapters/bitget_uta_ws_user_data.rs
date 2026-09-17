//! Bitget V3 / UTA private WebSocket event parser internals.
//!
//! Differences from the V2 parser (`bitget_ws_user_data.rs`):
//! - Envelope uses `arg.topic` (V2 used `arg.channel`).
//! - Order rows expose `qty` (V2 `size`) plus a new `accFillSize` for the
//!   running filled quantity. Both names are accepted via serde alias to keep
//!   the parser robust if Bitget keeps the V2 alias around for a while.
//! - Account payloads carry one aggregate UTA row with nested `coin[]` assets.
//! - Position payloads use the V3 `size/avgPrice/liqPrice/mmr/updatedTime`
//!   names and retain mode/finality fields.
//!
//! `Fill` parsing consumes the official UTA fill channel `exec*` fields and is
//! wired through the private WS mapper.

use super::bitget_uta_ws_user::{
    is_heartbeat_pong, BitgetAccountDelta, BitgetAccountUpdate, BitgetFillUpdate,
    BitgetOrderUpdate, BitgetPositionDelta, BitgetPositionUpdate, BitgetUserEvent, EXCHANGE,
    TOPIC_ACCOUNT, TOPIC_FILL, TOPIC_ORDER, TOPIC_POSITION,
};
use crate::adapter::{client_order_id_from_str, reduce_only_from_yes_no, strip_common_suffixes};
use crate::error::{ExchangeError, ExchangeResult};
use chrono::Utc;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;
use shared_types::{LiveOrderState, OrderInfo, OrderSide, OrderStatus, OrderType};

pub(super) fn parse_user_event(text: &str) -> ExchangeResult<Option<BitgetUserEvent>> {
    if is_heartbeat_pong(text) {
        return Ok(None);
    }
    let envelope: RawEnvelope = serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!("bitget uta user ws event: {error}; body={text}"))
    })?;
    // Subscribe/login ack frames have no action or data. An empty position
    // snapshot is different: Bitget sends it on first subscription to prove
    // that the account currently has no positions, and the cache must retain
    // that evidence.
    if envelope.data.is_empty()
        && !(envelope.arg.topic == TOPIC_POSITION && envelope.action == "snapshot")
    {
        return Ok(None);
    }
    match envelope.arg.topic.as_str() {
        TOPIC_ACCOUNT => account_update(envelope)
            .map(BitgetUserEvent::Account)
            .map(Some),
        TOPIC_ORDER => parse_event_rows(envelope, order_update)
            .map(BitgetUserEvent::Order)
            .map(Some),
        TOPIC_POSITION => position_update(envelope)
            .map(BitgetUserEvent::Position)
            .map(Some),
        TOPIC_FILL => parse_event_rows(envelope, fill_update)
            .map(BitgetUserEvent::Fill)
            .map(Some),
        _ => Ok(None),
    }
}

fn account_update(envelope: RawEnvelope) -> ExchangeResult<BitgetAccountUpdate> {
    let action = envelope.action.clone();
    let mut rows: Vec<RawAccountState> = serde_json::from_value(Value::Array(envelope.data))
        .map_err(|error| {
            ExchangeError::Parse(format!("bitget uta user ws account rows: {error}"))
        })?;
    if rows.len() != 1 {
        return Err(ExchangeError::Parse(format!(
            "bitget uta user ws account expected one aggregate row, got {}",
            rows.len()
        )));
    }
    let row = rows.pop().ok_or_else(|| {
        ExchangeError::Parse("bitget uta user ws account aggregate missing".to_owned())
    })?;
    let unrealized_pnl = parse_required_number(&row.unrealized_pnl, "account.unrealisedPnL")?;
    let accounts = row
        .coins
        .iter()
        .map(|coin| account_delta(coin, unrealized_pnl))
        .collect::<ExchangeResult<Vec<_>>>()?;
    Ok(BitgetAccountUpdate {
        action,
        total_equity: parse_required_number(&row.total_equity, "account.totalEquity")?,
        effective_equity: parse_required_number(&row.effective_equity, "account.effEquity")?,
        initial_margin: parse_required_number(&row.imr, "account.imr")?,
        maintenance_margin: parse_required_number(&row.mmr, "account.mmr")?,
        margin_ratio: parse_required_number(&row.margin_ratio, "account.mgnRatio")?,
        position_margin_ratio: parse_required_number(
            &row.position_margin_ratio,
            "account.positionMgnRatio",
        )?,
        unrealized_pnl,
        accounts,
    })
}

fn position_update(envelope: RawEnvelope) -> ExchangeResult<BitgetPositionUpdate> {
    let action = envelope.action.clone();
    let positions = parse_event_rows(envelope, |row| position_delta(&row))?;
    Ok(BitgetPositionUpdate { action, positions })
}

fn parse_event_rows<T, O>(
    envelope: RawEnvelope,
    convert: fn(T) -> ExchangeResult<O>,
) -> ExchangeResult<Vec<O>>
where
    T: DeserializeOwned,
{
    let rows: Vec<T> = serde_json::from_value(Value::Array(envelope.data)).map_err(|error| {
        ExchangeError::Parse(format!(
            "bitget uta user ws {} rows: {error}",
            envelope.arg.topic
        ))
    })?;
    rows.into_iter().map(convert).collect()
}

fn account_delta(
    row: &RawAccountDelta,
    account_unrealized_pnl: f64,
) -> ExchangeResult<BitgetAccountDelta> {
    let coin = required_text(&row.coin, "account.coin")?;
    Ok(BitgetAccountDelta {
        unrealized_pnl: if coin.eq_ignore_ascii_case("USDT") {
            account_unrealized_pnl
        } else {
            0.0
        },
        coin,
        frozen: parse_required_number(&row.frozen, "account.frozen")?,
        available: parse_required_number(&row.available, "account.available")?,
        equity: parse_required_number(&row.equity, "account.equity")?,
        usdt_equity: parse_required_number(&row.usdt_equity, "account.usdtEquity")?,
    })
}

fn order_update(row: RawOrderUpdate) -> ExchangeResult<BitgetOrderUpdate> {
    let category = known_category(&row.category, "order.category")?;
    let hold_mode = known_hold_mode(&row.hold_mode, category == "SPOT")?;
    let hold_side = known_hold_side(&row.hold_side, category == "SPOT")?;
    let trade_side = known_trade_side(&row.trade_side, category == "SPOT")?;
    let filled_quantity = parse_required_number(&row.acc_filled_size, "order.cumExecQty")?;
    let status = order_status(&row.status)?;
    let created_at = event_time(parse_required_i64(&row.created_time, "order.createdTime")?)?;
    let updated_time_ms = parse_required_i64(&row.updated_time, "order.updatedTime")?;
    let reduce_only = reduce_only_from_yes_no(&row.reduce_only)
        .map_err(|raw| ExchangeError::Parse(format!("bitget order.reduceOnly invalid: {raw}")))?;
    let client_order_id = client_order_id_from_str(&row.client_oid);
    let order = OrderInfo {
        execution_style: None,
        venue_time_in_force: Some(row.force.clone()),
        client_order_id,
        reduce_only,
        order_id: required_text(&row.order_id, "order.orderId")?,
        symbol: strip_common_suffixes(&required_text(&row.symbol, "order.symbol")?),
        exchange: EXCHANGE.into(),
        side: order_side(&row.side)?,
        order_type: order_type(&row.order_type, &row.force)?,
        status,
        quantity: parse_positive_number(&row.qty, "order.qty")?,
        price: parse_number(&row.price, "order.price")?,
        filled_quantity,
        filled_price: parse_number(&row.price_avg, "order.avgPrice")?,
        fees: fee_sum(&row.fee_detail)?,
        created_at,
    };
    Ok(BitgetOrderUpdate {
        client_order_id: row.client_oid,
        live_state: live_state(status),
        category,
        hold_mode,
        hold_side,
        trade_side,
        updated_time_ms,
        cancel_reason: non_empty_string(&row.cancel_reason),
        total_profit: parse_required_number(&row.total_profit, "order.totalProfit")?,
        order,
    })
}

fn position_delta(row: &RawPositionDelta) -> ExchangeResult<BitgetPositionDelta> {
    Ok(BitgetPositionDelta {
        symbol: strip_common_suffixes(&required_text(&row.symbol, "position.symbol")?),
        margin_coin: required_text(&row.margin_coin, "position.marginCoin")?,
        margin_size: parse_required_number(&row.margin_size, "position.marginSize")?,
        margin_mode: known_margin_mode(&row.margin_mode)?,
        hold_mode: known_hold_mode(&row.hold_mode, false)?,
        position_status: known_position_status(&row.position_status)?,
        side: known_hold_side(&row.hold_side, false)?,
        size: parse_required_number(&row.qty, "position.size")?,
        available: parse_required_number(&row.available, "position.available")?,
        frozen: parse_required_number(&row.frozen, "position.frozen")?,
        entry_price: parse_required_number(&row.open_price_avg, "position.avgPrice")?,
        leverage: parse_positive_number(&row.leverage, "position.leverage")?,
        unrealized_pnl: parse_required_number(&row.unrealized_pnl, "position.unrealisedPnl")?,
        liquidation_price: parse_positive_option(&row.liquidation_price, "position.liqPrice")?,
        // The official `ended` position example carries `mmr: ""`.
        maintenance_margin_rate: parse_number(&row.keep_margin_rate, "position.mmr")?,
        mark_price: parse_positive_number(&row.mark_price, "position.markPrice")?,
        updated_time_ms: parse_required_i64(&row.updated_time, "position.updatedTime")?,
    })
}

fn fill_update(row: RawFillUpdate) -> ExchangeResult<BitgetFillUpdate> {
    let category = known_category(&row.category, "fill.category")?;
    let spot = category == "SPOT";
    let fee = fee_sum(&row.fee_detail)?;
    let fee_currency = first_fee_currency(&row.fee_detail);
    Ok(BitgetFillUpdate {
        order_id: required_text(&row.order_id, "fill.orderId")?,
        client_order_id: row.client_oid,
        exec_id: required_text(&row.exec_id, "fill.execId")?,
        category,
        symbol: strip_common_suffixes(&required_text(&row.symbol, "fill.symbol")?),
        side: known_side(&row.side, "fill.side")?,
        hold_side: known_hold_side(&row.hold_side, spot)?,
        trade_side: known_trade_side(&row.trade_side, spot)?,
        price: parse_positive_number(&row.price, "fill.execPrice")?,
        size: parse_positive_number(&row.size, "fill.execQty")?,
        value: parse_positive_number(&row.value, "fill.execValue")?,
        realized_pnl: parse_required_number(&row.realized_pnl, "fill.execPnl")?,
        fee,
        fee_currency,
        trade_time_ms: parse_required_i64(&row.trade_time, "fill.execTime")?,
        updated_time_ms: parse_required_i64(&row.updated_time, "fill.updatedTime")?,
        is_rpi: optional_bool(&row.is_rpi, "fill.isRPI")?,
    })
}

fn fee_sum(rows: &[RawFeeDetail]) -> ExchangeResult<f64> {
    rows.iter()
        .map(|row| parse_number(&row.fee, "order.feeDetail.fee"))
        .try_fold(0.0, |total, fee| fee.map(|value| total + value))
}

fn first_fee_currency(rows: &[RawFeeDetail]) -> Option<String> {
    rows.iter()
        .find_map(|row| non_empty_string(row.fee_coin.as_str()))
}

fn required_text(raw: &str, field: &str) -> ExchangeResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ExchangeError::Parse(format!(
            "bitget uta user ws missing field {field}"
        )));
    }
    Ok(trimmed.to_owned())
}

fn non_empty_string(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn parse_positive_number(raw: &str, field: &str) -> ExchangeResult<f64> {
    let value = parse_number(raw, field)?;
    if value.is_finite() && value > 0.0 {
        return Ok(value);
    }
    Err(ExchangeError::Parse(format!(
        "bitget uta user ws non-positive field {field}: value={raw}"
    )))
}

fn parse_number(raw: &str, field: &str) -> ExchangeResult<f64> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(0.0);
    }
    let parsed = value.parse::<f64>().map_err(|error| {
        ExchangeError::Parse(format!(
            "bitget uta user ws numeric field {field}: {error}; value={raw}"
        ))
    })?;
    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(ExchangeError::Parse(format!(
            "bitget uta user ws non-finite field {field}: value={raw}"
        )))
    }
}

fn parse_required_number(raw: &str, field: &str) -> ExchangeResult<f64> {
    if raw.trim().is_empty() {
        return Err(ExchangeError::Parse(format!(
            "bitget uta user ws missing numeric field {field}"
        )));
    }
    parse_number(raw, field)
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
    raw.trim().parse().map_err(|error| {
        ExchangeError::Parse(format!(
            "bitget uta user ws integer field {field}: {error}; value={raw}"
        ))
    })
}

fn optional_bool(raw: &Value, field: &str) -> ExchangeResult<Option<bool>> {
    match raw {
        Value::Null => Ok(None),
        Value::Bool(value) => Ok(Some(*value)),
        Value::String(value)
            if value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("yes") =>
        {
            Ok(Some(true))
        }
        Value::String(value)
            if value.eq_ignore_ascii_case("false") || value.eq_ignore_ascii_case("no") =>
        {
            Ok(Some(false))
        }
        value => Err(ExchangeError::Parse(format!(
            "bitget uta user ws boolean field {field}: value={value}"
        ))),
    }
}

fn known_category(raw: &str, field: &str) -> ExchangeResult<String> {
    let value = required_text(raw, field)?.to_ascii_uppercase();
    if matches!(
        value.as_str(),
        "SPOT" | "MARGIN" | "USDT-FUTURES" | "USDC-FUTURES" | "COIN-FUTURES"
    ) {
        Ok(value)
    } else {
        Err(ExchangeError::Parse(format!(
            "bitget uta user ws unknown {field}: {raw}"
        )))
    }
}

fn known_side(raw: &str, field: &str) -> ExchangeResult<String> {
    match raw.to_ascii_lowercase().as_str() {
        "buy" | "sell" => Ok(raw.to_ascii_lowercase()),
        _ => Err(ExchangeError::Parse(format!(
            "bitget uta user ws unknown {field}: {raw}"
        ))),
    }
}

fn known_hold_mode(raw: &str, optional: bool) -> ExchangeResult<String> {
    match raw.to_ascii_lowercase().as_str() {
        "one_way_mode" | "hedge_mode" => Ok(raw.to_ascii_lowercase()),
        "" if optional => Ok(String::new()),
        _ => Err(ExchangeError::Parse(format!(
            "bitget uta user ws unknown holdMode: {raw}"
        ))),
    }
}

fn known_hold_side(raw: &str, optional: bool) -> ExchangeResult<String> {
    match raw.to_ascii_lowercase().as_str() {
        "long" | "short" => Ok(raw.to_ascii_lowercase()),
        "" if optional => Ok(String::new()),
        _ => Err(ExchangeError::Parse(format!(
            "bitget uta user ws unknown holdSide/posSide: {raw}"
        ))),
    }
}

fn known_trade_side(raw: &str, optional: bool) -> ExchangeResult<String> {
    match raw.to_ascii_lowercase().as_str() {
        "open" | "close" => Ok(raw.to_ascii_lowercase()),
        "" if optional => Ok(String::new()),
        _ => Err(ExchangeError::Parse(format!(
            "bitget uta user ws unknown tradeSide: {raw}"
        ))),
    }
}

fn known_margin_mode(raw: &str) -> ExchangeResult<String> {
    match raw.to_ascii_lowercase().as_str() {
        "crossed" | "isolated" | "cross" => Ok(raw.to_ascii_lowercase()),
        _ => Err(ExchangeError::Parse(format!(
            "bitget uta user ws unknown marginMode: {raw}"
        ))),
    }
}

fn known_position_status(raw: &str) -> ExchangeResult<String> {
    match raw.to_ascii_lowercase().as_str() {
        // UTA V3 documents `opening` and `ended`. Retain the earlier
        // contract-state values because Bitget can replay legacy-shaped rows
        // while an account is migrated to UTA.
        "opening" | "ended" | "normal" | "liquidation" | "adl" => Ok(raw.to_ascii_lowercase()),
        _ => Err(ExchangeError::Parse(format!(
            "bitget uta user ws unknown positionStatus: {raw}"
        ))),
    }
}

fn event_time(timestamp_ms: i64) -> ExchangeResult<chrono::DateTime<Utc>> {
    chrono::DateTime::<Utc>::from_timestamp_millis(timestamp_ms).ok_or_else(|| {
        ExchangeError::Parse(format!(
            "bitget uta user ws invalid timestamp: {timestamp_ms}"
        ))
    })
}

fn order_side(raw: &str) -> ExchangeResult<OrderSide> {
    match raw.to_ascii_lowercase().as_str() {
        "buy" => Ok(OrderSide::Buy),
        "sell" => Ok(OrderSide::Sell),
        _ => Err(ExchangeError::Parse(format!(
            "bitget uta user ws order side invalid: {raw}"
        ))),
    }
}

fn order_type(raw_type: &str, force: &str) -> ExchangeResult<OrderType> {
    match (
        raw_type.to_ascii_lowercase().as_str(),
        force.to_ascii_lowercase().as_str(),
    ) {
        ("market", _) => Ok(OrderType::Market),
        ("limit", "post_only") | ("limit", "postonly") => Ok(OrderType::PostOnly),
        ("limit", _) => Ok(OrderType::Limit),
        _ => Err(ExchangeError::Parse(format!(
            "bitget uta user ws order type invalid: orderType={raw_type} force={force}"
        ))),
    }
}

fn order_status(raw: &str) -> ExchangeResult<OrderStatus> {
    match raw.to_ascii_lowercase().as_str() {
        "live" | "new" => Ok(OrderStatus::Open),
        "partially_filled" => Ok(OrderStatus::PartiallyFilled),
        "filled" => Ok(OrderStatus::Filled),
        "canceled" | "cancelled" => Ok(OrderStatus::Canceled),
        _ => Err(ExchangeError::Parse(format!(
            "bitget uta user ws order status invalid: {raw}"
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

// -- Raw payload DTOs -------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawEnvelope {
    arg: RawArg,
    /// V3 envelope `action` 字段（`"snapshot"` / `"update"`）。账户/订单/成交不
    /// 区分使用，仅 `position` 走 D-5 snapshot 全量替换路径需要保留。
    #[serde(default)]
    action: String,
    #[serde(default)]
    data: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct RawArg {
    /// V3 uses `topic`; alias `channel` so a V2-shaped frame still parses.
    #[serde(default, alias = "channel")]
    topic: String,
}

#[derive(Debug, Deserialize)]
struct RawAccountState {
    #[serde(default, rename = "totalEquity")]
    total_equity: String,
    #[serde(default, rename = "effEquity")]
    effective_equity: String,
    #[serde(default)]
    mmr: String,
    #[serde(default)]
    imr: String,
    #[serde(default, rename = "mgnRatio")]
    margin_ratio: String,
    #[serde(default, rename = "positionMgnRatio")]
    position_margin_ratio: String,
    #[serde(default, rename = "unrealisedPnL", alias = "unrealizedPnl")]
    unrealized_pnl: String,
    #[serde(default, rename = "coin")]
    coins: Vec<RawAccountDelta>,
}

#[derive(Debug, Deserialize)]
struct RawAccountDelta {
    #[serde(default)]
    coin: String,
    #[serde(default, rename = "locked", alias = "frozen")]
    frozen: String,
    #[serde(default)]
    available: String,
    #[serde(default)]
    equity: String,
    #[serde(default, rename = "usdValue", alias = "usdtEquity")]
    usdt_equity: String,
}

#[derive(Debug, Deserialize)]
struct RawOrderUpdate {
    #[serde(default)]
    category: String,
    #[serde(default, rename = "orderId")]
    order_id: String,
    #[serde(default, rename = "clientOid")]
    client_oid: String,
    #[serde(default, rename = "reduceOnly")]
    reduce_only: String,
    #[serde(default, rename = "holdMode")]
    hold_mode: String,
    #[serde(default, rename = "holdSide")]
    hold_side: String,
    #[serde(default, rename = "tradeSide")]
    trade_side: String,
    #[serde(default, alias = "instId")]
    symbol: String,
    #[serde(default)]
    side: String,
    #[serde(default, rename = "orderType")]
    order_type: String,
    #[serde(default, alias = "timeInForce")]
    force: String,
    #[serde(default, alias = "orderStatus")]
    status: String,
    #[serde(default, alias = "size")]
    qty: String,
    #[serde(default)]
    price: String,
    /// V3 uses `accFillSize` (V2 used `accBaseVolume`).
    #[serde(
        default,
        rename = "accFillSize",
        alias = "accBaseVolume",
        alias = "cumExecQty"
    )]
    acc_filled_size: String,
    #[serde(default, rename = "priceAvg", alias = "avgPrice")]
    price_avg: String,
    #[serde(default, rename = "feeDetail")]
    fee_detail: Vec<RawFeeDetail>,
    #[serde(default, rename = "cTime", alias = "createdTime")]
    created_time: String,
    #[serde(default, rename = "updatedTime")]
    updated_time: String,
    #[serde(default, rename = "cancelReason")]
    cancel_reason: String,
    #[serde(default, rename = "totalProfit")]
    total_profit: String,
}

#[derive(Debug, Deserialize)]
struct RawFeeDetail {
    #[serde(default, rename = "feeCoin")]
    fee_coin: String,
    #[serde(default)]
    fee: String,
}

#[derive(Debug, Deserialize)]
struct RawPositionDelta {
    #[serde(default, alias = "instId")]
    symbol: String,
    #[serde(default, rename = "marginCoin")]
    margin_coin: String,
    #[serde(default, rename = "marginSize")]
    margin_size: String,
    #[serde(default, rename = "marginMode")]
    margin_mode: String,
    #[serde(default, rename = "holdMode")]
    hold_mode: String,
    #[serde(default, rename = "positionStatus")]
    position_status: String,
    #[serde(default, rename = "posSide", alias = "holdSide")]
    hold_side: String,
    #[serde(default, rename = "size", alias = "qty", alias = "total")]
    qty: String,
    #[serde(default)]
    available: String,
    #[serde(default)]
    frozen: String,
    #[serde(default, rename = "avgPrice", alias = "openPriceAvg")]
    open_price_avg: String,
    #[serde(default)]
    leverage: String,
    #[serde(
        default,
        rename = "unrealisedPnl",
        alias = "unrealizedPnl",
        alias = "unrealizedPL"
    )]
    unrealized_pnl: String,
    #[serde(default, rename = "liqPrice", alias = "liquidationPrice")]
    liquidation_price: String,
    #[serde(default, rename = "mmr", alias = "keepMarginRate")]
    keep_margin_rate: String,
    #[serde(default, rename = "markPrice")]
    mark_price: String,
    #[serde(default, rename = "updatedTime", alias = "uTime")]
    updated_time: String,
}

#[derive(Debug, Deserialize)]
struct RawFillUpdate {
    #[serde(default)]
    category: String,
    #[serde(default, rename = "orderId")]
    order_id: String,
    #[serde(default, rename = "clientOid")]
    client_oid: String,
    #[serde(default, rename = "execId")]
    exec_id: String,
    #[serde(default, alias = "instId")]
    symbol: String,
    #[serde(default)]
    side: String,
    #[serde(default, rename = "holdSide")]
    hold_side: String,
    #[serde(default, rename = "tradeSide")]
    trade_side: String,
    #[serde(default, rename = "execPrice", alias = "price")]
    price: String,
    #[serde(default, rename = "execQty", alias = "size")]
    size: String,
    #[serde(default, rename = "execValue")]
    value: String,
    #[serde(default, rename = "execPnl")]
    realized_pnl: String,
    #[serde(default, rename = "feeDetail")]
    fee_detail: Vec<RawFeeDetail>,
    #[serde(default, rename = "execTime", alias = "tradeTime", alias = "cTime")]
    trade_time: String,
    #[serde(default, rename = "updatedTime")]
    updated_time: String,
    #[serde(default, rename = "isRPI")]
    is_rpi: Value,
}
