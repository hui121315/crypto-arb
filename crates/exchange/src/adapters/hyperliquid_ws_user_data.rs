//! Hyperliquid user WebSocket event parser internals.

use super::hyperliquid_ws_user::{
    HyperliquidClearinghouseState, HyperliquidDexClearinghouseState, HyperliquidFill,
    HyperliquidFillLiquidation, HyperliquidFunding, HyperliquidLiquidation,
    HyperliquidLiquidationMethod, HyperliquidNonUserCancel, HyperliquidOpenOrdersSnapshot,
    HyperliquidOrderUpdate, HyperliquidPositionDelta, HyperliquidSpotBalance,
    HyperliquidUserWsEvent,
};
use super::hyperliquid_ws_user_raw::{
    RawAllDexsClearinghouse, RawClearinghouseEnvelope, RawEnvelope, RawFill, RawFillLiquidation,
    RawFunding, RawInnerClearinghouseState, RawLiquidation, RawNonUserCancel,
    RawOpenOrdersEnvelope, RawOrderUpdate, RawPosition, RawSpotBalance, RawSpotStateEnvelope,
    RawUserFills, RawUserFundings,
};
use crate::adapter::{client_order_id_from_str, strip_common_suffixes};
use crate::error::{ExchangeError, ExchangeResult};
use chrono::Utc;
use serde::de::DeserializeOwned;
use serde_json::Value;
use shared_types::{LiveOrderState, OrderInfo, OrderSide, OrderStatus, OrderType};

const EXCHANGE: &str = "hyperliquid";

mod open_orders;
use open_orders::parse_open_orders;

pub(super) fn parse_user_event(text: &str) -> ExchangeResult<Option<HyperliquidUserWsEvent>> {
    let envelope: RawEnvelope = serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!("hyperliquid user ws event: {error}; body={text}"))
    })?;
    match envelope.channel.as_str() {
        "orderUpdates" => parse_rows(envelope.data, order_update)
            .map(HyperliquidUserWsEvent::Order)
            .map(Some),
        "openOrders" => parse_open_orders(envelope.data)
            .map(HyperliquidUserWsEvent::OpenOrders)
            .map(Some),
        "userFills" => parse_user_fills(envelope.data)
            .map(HyperliquidUserWsEvent::Fill)
            .map(Some),
        "userFundings" => parse_user_fundings(envelope.data)
            .map(HyperliquidUserWsEvent::Funding)
            .map(Some),
        "user" => parse_user_event_payload(&envelope.data),
        "clearinghouseState" => parse_clearinghouse(envelope.data)
            .map(HyperliquidUserWsEvent::Clearinghouse)
            .map(Some),
        "allDexsClearinghouseState" => parse_all_dexs_clearinghouse(envelope.data)
            .map(HyperliquidUserWsEvent::AllDexsClearinghouse)
            .map(Some),
        "spotState" => parse_spot_state(envelope.data)
            .map(HyperliquidUserWsEvent::SpotState)
            .map(Some),
        _ => Ok(None),
    }
}

fn parse_rows<T, O>(data: Value, convert: fn(T) -> ExchangeResult<O>) -> ExchangeResult<Vec<O>>
where
    T: DeserializeOwned,
{
    let rows: Vec<T> = serde_json::from_value(data)
        .map_err(|error| ExchangeError::Parse(format!("hyperliquid ws rows: {error}")))?;
    rows.into_iter().map(convert).collect()
}

fn parse_user_fills(data: Value) -> ExchangeResult<Vec<HyperliquidFill>> {
    let rows: RawUserFills = serde_json::from_value(data)
        .map_err(|error| ExchangeError::Parse(format!("hyperliquid userFills: {error}")))?;
    rows.fills.into_iter().map(fill_update).collect()
}

fn parse_user_fundings(data: Value) -> ExchangeResult<Vec<HyperliquidFunding>> {
    let rows: RawUserFundings = serde_json::from_value(data)
        .map_err(|error| ExchangeError::Parse(format!("hyperliquid userFundings: {error}")))?;
    rows.fundings.iter().map(funding_update).collect()
}

fn parse_user_event_payload(data: &Value) -> ExchangeResult<Option<HyperliquidUserWsEvent>> {
    if let Some(fills) = data.get("fills").cloned() {
        return parse_rows(fills, fill_update)
            .map(HyperliquidUserWsEvent::Fill)
            .map(Some);
    }
    if let Some(funding) = data.get("funding").cloned() {
        let row: RawFunding = parse_value(funding, "funding")?;
        return funding_update(&row)
            .map(|funding| HyperliquidUserWsEvent::Funding(vec![funding]))
            .map(Some);
    }
    if let Some(liquidation) = data.get("liquidation").cloned() {
        let row: RawLiquidation = parse_value(liquidation, "liquidation")?;
        return liquidation_update(row)
            .map(HyperliquidUserWsEvent::Liquidation)
            .map(Some);
    }
    if let Some(non_user_cancel) = data.get("nonUserCancel").cloned() {
        return parse_non_user_cancels(non_user_cancel)
            .map(HyperliquidUserWsEvent::NonUserCancel)
            .map(Some);
    }
    Ok(None)
}

fn parse_clearinghouse(data: Value) -> ExchangeResult<HyperliquidClearinghouseState> {
    let row: RawClearinghouseEnvelope = parse_value(data, "clearinghouseState")?;
    clearinghouse_state(row.dex, row.user, row.clearinghouse_state)
}

fn parse_all_dexs_clearinghouse(
    data: Value,
) -> ExchangeResult<Vec<HyperliquidDexClearinghouseState>> {
    let row: RawAllDexsClearinghouse = parse_value(data, "allDexsClearinghouseState")?;
    row.clearinghouse_states
        .into_iter()
        .map(|(dex, state)| {
            clearinghouse_state(Some(dex.clone()), row.user.clone(), state)
                .map(|state| HyperliquidDexClearinghouseState { dex, state })
        })
        .collect()
}

fn parse_spot_state(data: Value) -> ExchangeResult<Vec<HyperliquidSpotBalance>> {
    let row: RawSpotStateEnvelope = parse_value(data, "spotState")?;
    row.spot_state
        .balances
        .into_iter()
        .map(spot_balance)
        .collect()
}

fn parse_non_user_cancels(data: Value) -> ExchangeResult<Vec<HyperliquidNonUserCancel>> {
    let rows: Vec<RawNonUserCancel> = parse_value(data, "nonUserCancel")?;
    rows.iter().map(non_user_cancel_update).collect()
}

fn parse_value<T>(data: Value, label: &str) -> ExchangeResult<T>
where
    T: DeserializeOwned,
{
    serde_json::from_value(data)
        .map_err(|error| ExchangeError::Parse(format!("hyperliquid ws {label}: {error}")))
}

fn order_update(row: RawOrderUpdate) -> ExchangeResult<HyperliquidOrderUpdate> {
    let RawOrderUpdate {
        order,
        status,
        status_timestamp,
    } = row;
    let original_size = number(&order.orig_sz, "order.origSz")?;
    let remaining_size = number(&order.sz, "order.sz")?;
    if original_size <= 0.0 || remaining_size < 0.0 || remaining_size > original_size {
        return Err(ExchangeError::Parse(format!(
            "hyperliquid ws invalid order sizes: original={original_size} remaining={remaining_size}"
        )));
    }
    let filled_quantity = original_size - remaining_size;
    let status = order_status(&status, filled_quantity)?;
    let status_timestamp_ms = positive_timestamp_ms("order.statusTimestamp", status_timestamp)?;
    let symbol = normalized_coin(&order.coin, "order.coin")?;
    let exchange = exchange_for_coin(&order.coin);
    let client_order_id = order.cloid.as_deref().and_then(client_order_id_from_str);
    let order_id = positive_order_id("order.oid", order.oid)?;
    let order = OrderInfo {
        execution_style: None,
        venue_time_in_force: None,
        client_order_id: client_order_id.clone(),
        reduce_only: order.reduce_only,
        order_id: order_id.to_string(),
        symbol,
        exchange,
        side: order_side(&order.side)?,
        order_type: OrderType::Limit,
        status,
        quantity: original_size,
        price: number(&order.limit_px, "order.limitPx")?,
        filled_quantity,
        filled_price: 0.0,
        fees: 0.0,
        created_at: event_time(order.timestamp)?,
    };
    Ok(HyperliquidOrderUpdate {
        client_order_id: client_order_id.unwrap_or_default(),
        live_state: live_state(status),
        status_timestamp_ms,
        order,
    })
}

fn fill_update(row: RawFill) -> ExchangeResult<HyperliquidFill> {
    Ok(HyperliquidFill {
        venue: exchange_for_coin(&row.coin),
        coin: normalized_coin(&row.coin, "fill.coin")?,
        trade_id: row
            .tid
            .map(|trade_id| positive_order_id("fill.tid", trade_id))
            .transpose()?,
        order_id: positive_order_id("fill.oid", row.oid)?.to_string(),
        side: required_text(&row.side, "fill.side")?,
        price: number(&row.px, "fill.px")?,
        size: number(&row.sz, "fill.sz")?,
        fee: number(&row.fee, "fill.fee")?,
        fee_token: required_text(&row.fee_token, "fill.feeToken")?,
        closed_pnl: number(&row.closed_pnl, "fill.closedPnl")?,
        liquidation: row.liquidation.as_ref().map(fill_liquidation).transpose()?,
        crossed: row.crossed,
        time_ms: positive_timestamp_ms("fill.time", row.time)?,
        tx_hash: row.hash,
    })
}

fn fill_liquidation(row: &RawFillLiquidation) -> ExchangeResult<HyperliquidFillLiquidation> {
    let mark_price = number(&row.mark_px, "fill.liquidation.markPx")?;
    if mark_price <= 0.0 {
        return Err(ExchangeError::Parse(format!(
            "hyperliquid ws invalid positive fill.liquidation.markPx: {mark_price}"
        )));
    }
    let method = match row.method.as_str() {
        "market" => HyperliquidLiquidationMethod::Market,
        "backstop" => HyperliquidLiquidationMethod::Backstop,
        method => {
            return Err(ExchangeError::Parse(format!(
                "hyperliquid ws unsupported fill liquidation method: {method}"
            )));
        }
    };
    Ok(HyperliquidFillLiquidation {
        liquidated_user: row
            .liquidated_user
            .as_deref()
            .map(|user| required_text(user, "fill.liquidation.liquidatedUser"))
            .transpose()?,
        mark_price,
        method,
    })
}

fn funding_update(row: &RawFunding) -> ExchangeResult<HyperliquidFunding> {
    Ok(HyperliquidFunding {
        venue: exchange_for_coin(&row.coin),
        coin: normalized_coin(&row.coin, "funding.coin")?,
        usdc: number(&row.usdc, "funding.usdc")?,
        size: number(&row.szi, "funding.szi")?,
        funding_rate: number(&row.funding_rate, "funding.fundingRate")?,
        time_ms: positive_timestamp_ms("funding.time", row.time)?,
    })
}

fn liquidation_update(row: RawLiquidation) -> ExchangeResult<HyperliquidLiquidation> {
    Ok(HyperliquidLiquidation {
        id: row.lid,
        liquidator: row.liquidator,
        liquidated_user: row.liquidated_user,
        notional_position: number(&row.liquidated_ntl_pos, "liquidation.liquidated_ntl_pos")?,
        account_value: number(
            &row.liquidated_account_value,
            "liquidation.liquidated_account_value",
        )?,
    })
}

fn non_user_cancel_update(row: &RawNonUserCancel) -> ExchangeResult<HyperliquidNonUserCancel> {
    Ok(HyperliquidNonUserCancel {
        venue: exchange_for_coin(&row.coin),
        coin: normalized_coin(&row.coin, "nonUserCancel.coin")?,
        order_id: positive_order_id("nonUserCancel.oid", row.oid)?.to_string(),
    })
}

fn clearinghouse_state(
    dex: Option<String>,
    user: String,
    row: RawInnerClearinghouseState,
) -> ExchangeResult<HyperliquidClearinghouseState> {
    Ok(HyperliquidClearinghouseState {
        dex,
        user,
        account_value: number(&row.margin_summary.account_value, "margin.accountValue")?,
        total_margin_used: number(
            &row.margin_summary.total_margin_used,
            "margin.totalMarginUsed",
        )?,
        withdrawable: number(&row.withdrawable, "withdrawable")?,
        positions: row
            .asset_positions
            .into_iter()
            .map(|entry| position_delta(entry.position))
            .collect::<ExchangeResult<Vec<_>>>()?,
    })
}

fn position_delta(row: RawPosition) -> ExchangeResult<HyperliquidPositionDelta> {
    let size = number(&row.szi, "position.szi")?;
    let entry_price = required_position_entry_price(size, &row.entry_px)?;
    Ok(HyperliquidPositionDelta {
        coin: clean_coin(&row.coin),
        side: signed_side(size),
        size: size.abs(),
        entry_price,
        liquidation_price: option_number(&row.liquidation_px, "position.liquidationPx")?,
        margin_used: number(&row.margin_used, "position.marginUsed")?,
        unrealized_pnl: number(&row.unrealized_pnl, "position.unrealizedPnl")?,
        leverage: row
            .leverage
            .map(|leverage| option_number(&leverage.value, "position.leverage.value"))
            .transpose()?
            .flatten()
            .unwrap_or(1.0),
    })
}

fn spot_balance(row: RawSpotBalance) -> ExchangeResult<HyperliquidSpotBalance> {
    Ok(HyperliquidSpotBalance {
        coin: row.coin,
        token: row.token,
        total: number(&row.total, "spot.total")?,
        hold: number(&row.hold, "spot.hold")?,
        entry_notional: number(&row.entry_ntl, "spot.entryNtl")?,
    })
}

fn required_position_entry_price(size: f64, value: &Value) -> ExchangeResult<f64> {
    let entry_price = option_number(value, "position.entryPx")?;
    if size == 0.0 {
        Ok(entry_price.unwrap_or_default())
    } else {
        entry_price
            .ok_or_else(|| ExchangeError::Parse("hyperliquid ws missing position.entryPx".into()))
    }
}

fn number(value: &Value, field: &str) -> ExchangeResult<f64> {
    let parsed = match value {
        Value::Number(number) => number.as_f64().ok_or_else(|| {
            ExchangeError::Parse(format!("hyperliquid ws non-f64 field {field}: {number}"))
        }),
        Value::String(text) => {
            if text.trim().is_empty() {
                Err(ExchangeError::Parse(format!(
                    "hyperliquid ws missing numeric field {field}"
                )))
            } else {
                text.parse().map_err(|error| {
                    ExchangeError::Parse(format!(
                        "hyperliquid ws numeric field {field}: {error}; value={text}"
                    ))
                })
            }
        }
        Value::Null => Err(ExchangeError::Parse(format!(
            "hyperliquid ws missing numeric field {field}"
        ))),
        other => Err(ExchangeError::Parse(format!(
            "hyperliquid ws unsupported numeric field {field}: {other}"
        ))),
    }?;
    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(ExchangeError::Parse(format!(
            "hyperliquid ws non-finite numeric field {field}: {parsed}"
        )))
    }
}

fn option_number(value: &Value, field: &str) -> ExchangeResult<Option<f64>> {
    match value {
        Value::Null => Ok(None),
        Value::String(text) if text.trim().is_empty() => Ok(None),
        _ => number(value, field).map(|value| (value > 0.0).then_some(value)),
    }
}

fn clean_coin(coin: &str) -> String {
    strip_common_suffixes(coin.split(':').next_back().unwrap_or(coin))
}

fn normalized_coin(coin: &str, field: &str) -> ExchangeResult<String> {
    let coin = clean_coin(coin);
    if coin.trim().is_empty() {
        Err(ExchangeError::Parse(format!(
            "hyperliquid ws missing {field}"
        )))
    } else {
        Ok(coin)
    }
}

fn required_text(value: &str, field: &str) -> ExchangeResult<String> {
    let value = value.trim();
    if value.is_empty() {
        Err(ExchangeError::Parse(format!(
            "hyperliquid ws missing {field}"
        )))
    } else {
        Ok(value.to_owned())
    }
}

fn exchange_for_coin(coin: &str) -> String {
    coin.split_once(':')
        .and_then(|(dex, _)| (!dex.trim().is_empty()).then_some(dex))
        .map(|dex| format!("{EXCHANGE}:{dex}"))
        .unwrap_or_else(|| EXCHANGE.to_owned())
}

fn event_time(timestamp_ms: i64) -> ExchangeResult<chrono::DateTime<Utc>> {
    let timestamp_ms = positive_timestamp_ms("timestamp", timestamp_ms)?;
    chrono::DateTime::<Utc>::from_timestamp_millis(timestamp_ms).ok_or_else(|| {
        ExchangeError::Parse(format!(
            "hyperliquid user ws invalid timestamp: {timestamp_ms}"
        ))
    })
}

fn order_side(side: &str) -> ExchangeResult<OrderSide> {
    if side.eq_ignore_ascii_case("A") || side.eq_ignore_ascii_case("sell") {
        Ok(OrderSide::Sell)
    } else if side.eq_ignore_ascii_case("B") || side.eq_ignore_ascii_case("buy") {
        Ok(OrderSide::Buy)
    } else {
        Err(ExchangeError::Parse(format!(
            "hyperliquid user ws unsupported order side: {side}"
        )))
    }
}

fn signed_side(size: f64) -> String {
    if size < 0.0 {
        "short".to_owned()
    } else {
        "long".to_owned()
    }
}

fn order_status(status: &str, filled_quantity: f64) -> ExchangeResult<OrderStatus> {
    let normalized = status.to_ascii_lowercase();
    if normalized == "filled" {
        Ok(OrderStatus::Filled)
    } else if normalized == "scheduledcancel"
        || normalized.contains("canceled")
        || normalized.contains("cancelled")
    {
        Ok(OrderStatus::Canceled)
    } else if normalized.contains("rejected") {
        Ok(OrderStatus::Rejected)
    } else if normalized == "open" || normalized == "triggered" {
        if filled_quantity > 0.0 {
            Ok(OrderStatus::PartiallyFilled)
        } else {
            Ok(OrderStatus::Open)
        }
    } else {
        Err(ExchangeError::Parse(format!(
            "hyperliquid user ws unsupported order status: {status}"
        )))
    }
}

fn positive_timestamp_ms(field: &str, value: i64) -> ExchangeResult<i64> {
    if value > 0 {
        Ok(value)
    } else {
        Err(ExchangeError::Parse(format!(
            "hyperliquid user ws invalid timestamp field {field}: {value}"
        )))
    }
}

fn positive_order_id(field: &str, value: i64) -> ExchangeResult<i64> {
    if value > 0 {
        Ok(value)
    } else {
        Err(ExchangeError::Parse(format!(
            "hyperliquid ws invalid order id field {field}: {value}"
        )))
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
