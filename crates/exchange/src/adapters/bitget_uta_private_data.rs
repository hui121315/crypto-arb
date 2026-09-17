//! Bitget V3 / UTA private read response parsing.
//!
//! V3 differences vs V2 (`bitget_private_data.rs`):
//! - `account/assets` is a unified wallet endpoint whose `data` is an account
//!   object with an `assets[]` list; rows expose `coin / equity / balance /
//!   available / locked / usdValue`.
//! - `position/current-position` and `trade/unfilled-orders` wrap rows in a
//!   `{list: [...], cursor: ""}` object instead of returning a bare array.
//! - Order quantity field is `qty` (V2 used `size`).
//!
//! Official references:
//! - Account assets: <https://www.bitget.com/api-doc/uta/account/Get-Account-Assets>
//! - Current position: <https://www.bitget.com/api-doc/uta/trade/Get-Position>
//! - Unfilled orders: <https://www.bitget.com/api-doc/uta/trade/Get-Order-Pending>
//! - Order info: <https://www.bitget.com/api-doc/uta/trade/Get-Order-Details>

use crate::adapter::strip_common_suffixes;
use crate::error::{ExchangeError, ExchangeResult};
use serde::Deserialize;
use shared_types::{
    AccountEquityScope, BalanceInfo, OrderInfo, OrderSide, OrderStatus, OrderType, PositionInfo,
    VenueAccountSummary,
};
use std::collections::HashMap;

mod order;

pub(super) use order::{parse_open_order, UtaOrderRow};

const NAME: &str = "bitget";

// -- DTOs -------------------------------------------------------------------

/// V3 `GET /api/v3/account/assets` payload.
#[derive(Debug, Deserialize)]
pub(super) struct UtaAccountAssetsPayload {
    #[serde(rename = "accountEquity")]
    pub(super) account_equity: String,
    #[serde(rename = "effEquity")]
    pub(super) effective_equity: String,
    pub(super) imr: String,
    pub(super) mmr: String,
    #[serde(rename = "mgnRatio")]
    pub(super) margin_ratio: String,
    #[serde(rename = "positionMgnRatio")]
    pub(super) position_margin_ratio: String,
    #[serde(rename = "usdtUnrealisedPnl")]
    pub(super) usdt_unrealized_pl: String,
    pub(super) assets: Vec<UtaAccountAssetItem>,
}

/// V3 `GET /api/v3/account/assets` row (one entry per coin in the unified
/// wallet).
#[derive(Debug, Deserialize)]
pub(super) struct UtaAccountAssetItem {
    pub(super) coin: String,
    pub(super) available: String,
    pub(super) locked: String,
    pub(super) equity: String,
}

/// V3 `GET /api/v3/position/current-position` row (futures position).
#[derive(Debug, Deserialize)]
pub(super) struct UtaPositionRow {
    pub(super) symbol: String,
    #[serde(rename = "posSide")]
    pub(super) hold_side: String,
    #[serde(rename = "holdMode")]
    pub(super) hold_mode: String,
    #[serde(rename = "total")]
    pub(super) qty: String,
    #[serde(rename = "avgPrice")]
    pub(super) avg_price: String,
    #[serde(rename = "markPrice")]
    pub(super) mark_price: String,
    #[serde(rename = "unrealisedPnl")]
    pub(super) unrealized_pl: String,
    pub(super) leverage: String,
    #[serde(rename = "liquidationPrice")]
    pub(super) liquidation_price: String,
    #[serde(rename = "positionBalance")]
    pub(super) margin_size: String,
    #[serde(rename = "mmr")]
    pub(super) maintenance_margin_rate: String,
}

/// V3 paginated response wrapper for `unfilled-orders / history-orders /
/// fills / current-position`.
#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub(super) struct UtaListPayload<T> {
    #[serde(deserialize_with = "deserialize_nullable_list")]
    pub(super) list: Vec<T>,
}

fn deserialize_nullable_list<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<Vec<T>>::deserialize(deserializer).map(Option::unwrap_or_default)
}

// -- Parsers ----------------------------------------------------------------

pub(super) fn parse_account_balances(
    payload: UtaAccountAssetsPayload,
    currency: Option<&str>,
) -> ExchangeResult<HashMap<String, BalanceInfo>> {
    let account_unrealized = parse_required_decimal(
        "account assets",
        "usdtUnrealisedPnl",
        &payload.usdt_unrealized_pl,
    )?;
    parse_balance_items(payload.assets, currency, account_unrealized)
}

pub(super) fn parse_account_summary(
    payload: &UtaAccountAssetsPayload,
    observed_at_ms: i64,
) -> ExchangeResult<VenueAccountSummary> {
    Ok(VenueAccountSummary {
        venue: NAME.to_owned(),
        account_type: "uta".to_owned(),
        equity_scope: AccountEquityScope::Unified,
        total_equity_usd: parse_required_decimal(
            "account assets",
            "accountEquity",
            &payload.account_equity,
        )?,
        total_available_balance_usd: parse_required_decimal(
            "account assets",
            "effEquity",
            &payload.effective_equity,
        )?,
        withdrawable_balance_usd: None,
        total_initial_margin_usd: parse_required_decimal("account assets", "imr", &payload.imr)?,
        total_maintenance_margin_usd: parse_required_decimal(
            "account assets",
            "mmr",
            &payload.mmr,
        )?,
        account_im_rate: parse_required_decimal(
            "account assets",
            "mgnRatio",
            &payload.margin_ratio,
        )?,
        account_mm_rate: parse_required_decimal(
            "account assets",
            "positionMgnRatio",
            &payload.position_margin_ratio,
        )?,
        source: "bitget.GET /api/v3/account/assets".to_owned(),
        observed_at_ms,
        freshness_ms: Some(0),
        problem: None,
    })
}

fn parse_balance_items(
    items: Vec<UtaAccountAssetItem>,
    currency: Option<&str>,
    account_unrealized_pnl: f64,
) -> ExchangeResult<HashMap<String, BalanceInfo>> {
    let mut out = HashMap::new();
    for item in items {
        let coin = parse_required_text("account assets", "coin", &item.coin)?;
        if let Some(want) = currency {
            if !want.eq_ignore_ascii_case(coin) {
                continue;
            }
        }
        let available = parse_required_decimal("account assets", "available", &item.available)?;
        let frozen = parse_required_decimal("account assets", "locked", &item.locked)?;
        let total = parse_required_decimal("account assets", "equity", &item.equity)?;
        let unrealized_pnl = if coin.eq_ignore_ascii_case("USDT") {
            account_unrealized_pnl
        } else {
            0.0
        };
        out.insert(
            coin.to_owned(),
            BalanceInfo {
                currency: coin.to_owned(),
                total,
                available,
                frozen,
                unrealized_pnl,
            },
        );
    }
    Ok(out)
}

pub(super) fn parse_positions(
    rows: &[UtaPositionRow],
    target: Option<&str>,
) -> ExchangeResult<Vec<PositionInfo>> {
    let mut hedge_present = false;
    let mut out = Vec::new();
    for row in rows {
        if let Some(want) = target {
            if row.symbol != want {
                continue;
            }
        }
        let hold_mode = parse_hold_mode(&row.hold_mode)?;
        if let Some(position) = parse_position(row)? {
            if hold_mode == BitgetHoldMode::Hedge {
                hedge_present = true;
            }
            out.push(position);
        }
    }
    if hedge_present {
        crate::adapter::pair_hedge_positions(&mut out);
    }
    Ok(out)
}

pub(super) fn parse_open_orders(rows: Vec<UtaOrderRow>) -> ExchangeResult<Vec<OrderInfo>> {
    rows.into_iter().map(parse_open_order).collect()
}

fn parse_position(row: &UtaPositionRow) -> ExchangeResult<Option<PositionInfo>> {
    let symbol = parse_required_text("current position", "symbol", &row.symbol)?;
    let qty = parse_required_decimal("current position", "total", &row.qty)?;
    if qty == 0.0 {
        return Ok(None);
    }
    let liquidation_price = parse_required_decimal(
        "current position",
        "liquidationPrice",
        &row.liquidation_price,
    )?;
    let liquidation_price = (liquidation_price > 0.0).then_some(liquidation_price);
    Ok(Some(PositionInfo {
        symbol: strip_common_suffixes(symbol),
        exchange: NAME.into(),
        side: parse_position_side(&row.hold_side)?,
        quantity: qty,
        entry_price: parse_required_decimal("current position", "avgPrice", &row.avg_price)?,
        mark_price: parse_required_decimal("current position", "markPrice", &row.mark_price)?,
        unrealized_pnl: parse_required_decimal(
            "current position",
            "unrealisedPnl",
            &row.unrealized_pl,
        )?,
        leverage: parse_positive_required_decimal("current position", "leverage", &row.leverage)?,
        liquidation_price,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin: parse_required_decimal("current position", "positionBalance", &row.margin_size)?,
        maintenance_margin_ratio: parse_required_non_negative_decimal(
            "current position",
            "mmr",
            &row.maintenance_margin_rate,
        )?,
        position_mode: None,
        margin_mode: None,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }))
}

fn parse_required_decimal(scope: &str, field: &str, value: &str) -> ExchangeResult<f64> {
    let value = value.trim();
    if value.is_empty() {
        return Err(bitget_parse(scope, field, value));
    }
    value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| bitget_parse(scope, field, value))
}

fn parse_required_non_negative_decimal(
    scope: &str,
    field: &str,
    value: &str,
) -> ExchangeResult<f64> {
    let value = parse_required_decimal(scope, field, value)?;
    if value >= 0.0 {
        Ok(value)
    } else {
        Err(bitget_parse(scope, field, &value.to_string()))
    }
}

fn parse_positive_required_decimal(scope: &str, field: &str, value: &str) -> ExchangeResult<f64> {
    let value = parse_required_decimal(scope, field, value)?;
    if value > 0.0 {
        Ok(value)
    } else {
        Err(bitget_parse(scope, field, &value.to_string()))
    }
}

fn parse_required_text<'a>(scope: &str, field: &str, value: &'a str) -> ExchangeResult<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        Err(bitget_parse(scope, field, value))
    } else {
        Ok(value)
    }
}

fn parse_required_timestamp(
    scope: &str,
    field: &str,
    value: &str,
) -> ExchangeResult<chrono::DateTime<chrono::Utc>> {
    let raw = value
        .trim()
        .parse::<i64>()
        .map_err(|_| bitget_parse(scope, field, value))?;
    if raw <= 0 {
        return Err(bitget_parse(scope, field, value));
    }
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(raw)
        .ok_or_else(|| bitget_parse(scope, field, value))
}

fn parse_reduce_only(value: &str) -> ExchangeResult<Option<bool>> {
    match value.trim().to_ascii_uppercase().as_str() {
        "YES" => Ok(Some(true)),
        "NO" => Ok(Some(false)),
        _ => Err(bitget_parse("unfilled orders", "reduceOnly", value)),
    }
}

fn parse_order_side(side: &str) -> ExchangeResult<OrderSide> {
    match side.to_ascii_lowercase().as_str() {
        "buy" => Ok(OrderSide::Buy),
        "sell" => Ok(OrderSide::Sell),
        _ => Err(bitget_parse("unfilled orders", "side", side)),
    }
}

fn parse_order_type(order_type: &str) -> ExchangeResult<OrderType> {
    match order_type.to_ascii_lowercase().as_str() {
        "market" => Ok(OrderType::Market),
        "limit" => Ok(OrderType::Limit),
        _ => Err(bitget_parse("unfilled orders", "orderType", order_type)),
    }
}

fn parse_order_status(status: &str) -> ExchangeResult<OrderStatus> {
    match status.to_ascii_lowercase().as_str() {
        "live" | "new" => Ok(OrderStatus::Open),
        "partially_filled" => Ok(OrderStatus::PartiallyFilled),
        "filled" => Ok(OrderStatus::Filled),
        "canceled" | "cancelled" => Ok(OrderStatus::Canceled),
        _ => Err(bitget_parse("unfilled orders", "orderStatus", status)),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BitgetTimeInForce {
    Gtc,
    Ioc,
    Fok,
    PostOnly,
    Rpi,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BitgetHoldMode {
    OneWay,
    Hedge,
}

fn parse_hold_mode(value: &str) -> ExchangeResult<BitgetHoldMode> {
    match value.to_ascii_lowercase().as_str() {
        "one_way_mode" => Ok(BitgetHoldMode::OneWay),
        "hedge_mode" => Ok(BitgetHoldMode::Hedge),
        _ => Err(bitget_parse("current position", "holdMode", value)),
    }
}

fn parse_time_in_force(value: &str) -> ExchangeResult<BitgetTimeInForce> {
    match value.to_ascii_lowercase().as_str() {
        "gtc" => Ok(BitgetTimeInForce::Gtc),
        "ioc" => Ok(BitgetTimeInForce::Ioc),
        "fok" => Ok(BitgetTimeInForce::Fok),
        "post_only" => Ok(BitgetTimeInForce::PostOnly),
        "rpi" => Ok(BitgetTimeInForce::Rpi),
        _ => Err(bitget_parse("unfilled orders", "timeInForce", value)),
    }
}

fn parse_position_side(side: &str) -> ExchangeResult<String> {
    match side.to_ascii_lowercase().as_str() {
        "long" => Ok("long".to_owned()),
        "short" => Ok("short".to_owned()),
        _ => Err(bitget_parse("current position", "posSide", side)),
    }
}

fn bitget_parse(scope: &str, field: &str, value: &str) -> ExchangeError {
    ExchangeError::Parse(format!("bitget {scope} field {field} invalid: {value}"))
}

#[cfg(test)]
#[path = "bitget_uta_private_data_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "bitget_uta_private_data_pr_en_tests.rs"]
mod pr_en_tests;
