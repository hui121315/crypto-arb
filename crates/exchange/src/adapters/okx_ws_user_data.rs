//! OKX V5 private WebSocket event parser internals.

use super::okx_ws_user::{
    OkxAccountSummaryDelta, OkxAccountUpdate, OkxBalanceDelta, OkxOrderFillUpdate, OkxOrderUpdate,
    OkxPositionDelta, OkxPositionUpdate, OkxUserEvent, CHANNEL_ACCOUNT, CHANNEL_ORDERS,
    CHANNEL_POSITIONS, EXCHANGE,
};
use crate::adapter::{client_order_id_from_str, strip_common_suffixes};
use crate::adapters::okx_read_parse::{
    parse_optional_reduce_only, parse_order_side, parse_order_status, parse_order_type,
    parse_position_side,
};
use crate::error::{ExchangeError, ExchangeResult};
use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;
use shared_types::{LiveOrderState, OrderInfo, OrderStatus};

pub(super) fn parse_user_event(text: &str) -> ExchangeResult<Option<OkxUserEvent>> {
    let envelope: RawEnvelope = serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!("okx user ws event: {error}; body={text}"))
    })?;
    let Some(rows) = envelope.data else {
        return Ok(None);
    };
    let channel = envelope.arg.map(|arg| arg.channel).unwrap_or_default();
    match channel.as_str() {
        CHANNEL_ACCOUNT => account_update(envelope.event_type, envelope.last_page, rows)
            .map(OkxUserEvent::Account)
            .map(Some),
        CHANNEL_POSITIONS => position_update(envelope.event_type, envelope.last_page, rows)
            .map(OkxUserEvent::Position)
            .map(Some),
        CHANNEL_ORDERS => order_updates(rows).map(OkxUserEvent::Order).map(Some),
        _ => Ok(None),
    }
}

fn account_update(
    event_type: String,
    last_page: bool,
    rows: Vec<Value>,
) -> ExchangeResult<OkxAccountUpdate> {
    let accounts: Vec<RawAccountData> = serde_json::from_value(Value::Array(rows))
        .map_err(|error| ExchangeError::Parse(format!("okx user ws account rows: {error}")))?;
    let mut balances = Vec::new();
    let mut summary = None;
    for account in accounts {
        if summary.is_none() {
            summary = account_summary(&account)?;
        }
        for detail in account.details {
            balances.push(balance_delta(detail)?);
        }
    }
    Ok(OkxAccountUpdate {
        event_type,
        last_page,
        summary,
        balances,
    })
}

fn account_summary(row: &RawAccountData) -> ExchangeResult<Option<OkxAccountSummaryDelta>> {
    if row.total_eq.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(OkxAccountSummaryDelta {
        total_equity_usd: parse_number(&row.total_eq, "account.totalEq")?,
        total_available_balance_usd: parse_number(&row.available_equity, "account.availEq")?,
        total_initial_margin_usd: parse_number(&row.imr, "account.imr")?,
        total_maintenance_margin_usd: parse_number(&row.mmr, "account.mmr")?,
        updated_time_ms: parse_i64(&row.u_time, "account.uTime")?,
    }))
}

fn position_update(
    event_type: String,
    last_page: bool,
    rows: Vec<Value>,
) -> ExchangeResult<OkxPositionUpdate> {
    let raw: Vec<RawPositionData> = serde_json::from_value(Value::Array(rows))
        .map_err(|error| ExchangeError::Parse(format!("okx user ws position rows: {error}")))?;
    Ok(OkxPositionUpdate {
        event_type,
        last_page,
        positions: raw
            .into_iter()
            .map(position_delta)
            .collect::<ExchangeResult<Vec<_>>>()?,
    })
}

fn order_updates(rows: Vec<Value>) -> ExchangeResult<Vec<OkxOrderUpdate>> {
    let raw: Vec<RawOrderData> = serde_json::from_value(Value::Array(rows))
        .map_err(|error| ExchangeError::Parse(format!("okx user ws order rows: {error}")))?;
    raw.into_iter().map(order_update).collect()
}

fn balance_delta(row: RawBalanceDetail) -> ExchangeResult<OkxBalanceDelta> {
    Ok(OkxBalanceDelta {
        currency: row.ccy,
        total: parse_number(&row.eq, "account.details.eq")?,
        available: parse_number(&row.avail_bal, "account.details.availBal")?,
        frozen: parse_number(&row.frozen_bal, "account.details.frozenBal")?,
        unrealized_pnl: parse_number(&row.upl, "account.details.upl")?,
        updated_time_ms: parse_i64(&row.u_time, "account.details.uTime")?,
    })
}

fn position_delta(row: RawPositionData) -> ExchangeResult<OkxPositionDelta> {
    let quantity = parse_number(&row.pos, "position.pos")?;
    Ok(OkxPositionDelta {
        symbol: strip_common_suffixes(&row.inst_id),
        inst_id: row.inst_id,
        inst_type: row.inst_type,
        side: parse_position_side("position", &row.pos_side, quantity)?,
        quantity: quantity.abs(),
        entry_price: parse_number(&row.avg_px, "position.avgPx")?,
        mark_price: parse_number(&row.mark_px, "position.markPx")?,
        unrealized_pnl: parse_number(&row.upl, "position.upl")?,
        leverage: parse_number(&row.lever, "position.lever")?,
        liquidation_price: parse_positive_option(&row.liq_px, "position.liqPx")?,
        margin: parse_number(&row.margin, "position.margin")?,
        initial_margin: parse_number(&row.imr, "position.imr")?,
        maintenance_margin_ratio: parse_number(&row.mgn_ratio, "position.mgnRatio")?,
        updated_time_ms: parse_i64(&row.u_time, "position.uTime")?,
    })
}

fn order_update(row: RawOrderData) -> ExchangeResult<OkxOrderUpdate> {
    let status = parse_order_status("order", &row.state)?;
    let timestamp_ms = parse_i64(&row.u_time, "order.uTime")?;
    let created_at_ms = if row.c_time.trim().is_empty() {
        timestamp_ms
    } else {
        parse_i64(&row.c_time, "order.cTime")?
    };
    let fill = order_fill(&row)?;
    let order = OrderInfo {
        execution_style: None,
        venue_time_in_force: Some(row.ord_type.clone()),
        client_order_id: client_order_id_from_str(&row.cl_ord_id),
        reduce_only: parse_optional_reduce_only("order", &row.reduce_only)?,
        order_id: row.ord_id,
        symbol: strip_common_suffixes(&row.inst_id),
        exchange: EXCHANGE.into(),
        side: parse_order_side("order", &row.side)?,
        order_type: parse_order_type("order", &row.ord_type)?,
        status,
        quantity: parse_number(&row.sz, "order.sz")?,
        price: parse_number(&row.px, "order.px")?,
        filled_quantity: parse_number(&row.acc_fill_sz, "order.accFillSz")?,
        filled_price: parse_number(&row.avg_px, "order.avgPx")?,
        fees: parse_okx_fee_cost(&row.fee, "order.fee")?,
        created_at: event_time(created_at_ms)?,
    };
    Ok(OkxOrderUpdate {
        client_order_id: row.cl_ord_id,
        live_state: live_state(status),
        updated_time_ms: timestamp_ms,
        fill,
        order,
    })
}

fn order_fill(row: &RawOrderData) -> ExchangeResult<Option<OkxOrderFillUpdate>> {
    let Some(trade_id) = clean_optional_text(&row.trade_id) else {
        return Ok(None);
    };
    Ok(Some(OkxOrderFillUpdate {
        trade_id,
        fill_price: parse_positive_number(&row.fill_px, "order.fillPx")?,
        fill_size: parse_positive_number(&row.fill_sz, "order.fillSz")?,
        fill_fee: parse_optional_okx_fee_cost(&row.fill_fee, "order.fillFee")?,
        fill_fee_currency: clean_optional_text(&row.fill_fee_ccy),
        fill_time_ms: parse_i64(&row.fill_time, "order.fillTime")?,
    }))
}

fn parse_number(raw: &str, field: &str) -> ExchangeResult<f64> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(0.0);
    }
    value.parse().map_err(|error| {
        ExchangeError::Parse(format!(
            "okx user ws numeric field {field}: {error}; value={raw}"
        ))
    })
}

fn parse_positive_number(raw: &str, field: &str) -> ExchangeResult<f64> {
    let value = parse_number(raw, field)?;
    if value <= 0.0 || !value.is_finite() {
        return Err(ExchangeError::Parse(format!(
            "okx user ws positive numeric field {field}: invalid value={raw}"
        )));
    }
    Ok(value)
}

fn parse_okx_fee_cost(raw: &str, field: &str) -> ExchangeResult<f64> {
    parse_number(raw, field).map(|value| -value)
}

fn parse_optional_okx_fee_cost(raw: &str, field: &str) -> ExchangeResult<Option<f64>> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(None);
    }
    parse_okx_fee_cost(value, field).map(Some)
}

fn parse_i64(raw: &str, field: &str) -> ExchangeResult<i64> {
    raw.trim().parse().map_err(|error| {
        ExchangeError::Parse(format!(
            "okx user ws integer field {field}: {error}; value={raw}"
        ))
    })
}

fn clean_optional_text(raw: &str) -> Option<String> {
    let value = raw.trim();
    (!value.is_empty() && value != "0").then(|| value.to_owned())
}

fn parse_positive_option(raw: &str, field: &str) -> ExchangeResult<Option<f64>> {
    let value = parse_number(raw, field)?;
    Ok((value > 0.0).then_some(value))
}

fn event_time(timestamp_ms: i64) -> ExchangeResult<chrono::DateTime<Utc>> {
    chrono::DateTime::<Utc>::from_timestamp_millis(timestamp_ms).ok_or_else(|| {
        ExchangeError::Parse(format!("okx user ws invalid timestamp: {timestamp_ms}"))
    })
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
    arg: Option<RawArg>,
    #[serde(default, rename = "eventType")]
    event_type: String,
    #[serde(default, rename = "lastPage")]
    last_page: bool,
    data: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
struct RawArg {
    #[serde(default)]
    channel: String,
}

#[derive(Debug, Deserialize)]
struct RawAccountData {
    #[serde(default, rename = "totalEq")]
    total_eq: String,
    #[serde(default, rename = "availEq")]
    available_equity: String,
    #[serde(default)]
    imr: String,
    #[serde(default)]
    mmr: String,
    #[serde(default, rename = "uTime")]
    u_time: String,
    #[serde(default)]
    details: Vec<RawBalanceDetail>,
}

#[derive(Debug, Deserialize)]
struct RawBalanceDetail {
    #[serde(default)]
    ccy: String,
    #[serde(default)]
    eq: String,
    #[serde(default, rename = "availBal")]
    avail_bal: String,
    #[serde(default, rename = "frozenBal")]
    frozen_bal: String,
    #[serde(default)]
    upl: String,
    #[serde(default, rename = "uTime")]
    u_time: String,
}

#[derive(Debug, Deserialize)]
struct RawPositionData {
    #[serde(default, rename = "instType")]
    inst_type: String,
    #[serde(default, rename = "instId")]
    inst_id: String,
    #[serde(default, rename = "posSide")]
    pos_side: String,
    #[serde(default)]
    pos: String,
    #[serde(default, rename = "avgPx")]
    avg_px: String,
    #[serde(default, rename = "markPx")]
    mark_px: String,
    #[serde(default)]
    upl: String,
    #[serde(default)]
    lever: String,
    #[serde(default, rename = "liqPx")]
    liq_px: String,
    #[serde(default)]
    margin: String,
    #[serde(default)]
    imr: String,
    #[serde(default, rename = "mgnRatio")]
    mgn_ratio: String,
    #[serde(default, rename = "uTime")]
    u_time: String,
}

#[derive(Debug, Deserialize)]
struct RawOrderData {
    #[serde(default, rename = "ordId")]
    ord_id: String,
    #[serde(default, rename = "clOrdId")]
    cl_ord_id: String,
    #[serde(default, rename = "reduceOnly")]
    reduce_only: String,
    #[serde(default, rename = "instId")]
    inst_id: String,
    #[serde(default)]
    side: String,
    #[serde(default, rename = "ordType")]
    ord_type: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    sz: String,
    #[serde(default)]
    px: String,
    #[serde(default, rename = "accFillSz")]
    acc_fill_sz: String,
    #[serde(default, rename = "avgPx")]
    avg_px: String,
    #[serde(default)]
    fee: String,
    #[serde(default, rename = "tradeId")]
    trade_id: String,
    #[serde(default, rename = "fillPx")]
    fill_px: String,
    #[serde(default, rename = "fillSz")]
    fill_sz: String,
    #[serde(default, rename = "fillFee")]
    fill_fee: String,
    #[serde(default, rename = "fillFeeCcy")]
    fill_fee_ccy: String,
    #[serde(default, rename = "fillTime")]
    fill_time: String,
    #[serde(default, rename = "cTime")]
    c_time: String,
    #[serde(default, rename = "uTime")]
    u_time: String,
}
