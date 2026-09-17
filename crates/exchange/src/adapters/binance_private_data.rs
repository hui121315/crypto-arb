//! Binance private REST response parsing.

use crate::adapter::{client_order_id_from_str, pair_hedge_positions, strip_common_suffixes};
use crate::error::{ExchangeError, ExchangeResult};
use serde::Deserialize;
use shared_types::{
    AccountEquityScope, BalanceInfo, OrderInfo, OrderIntent, OrderSide, OrderStatus, OrderType,
    PositionInfo, VenueAccountSummary,
};
use std::collections::HashMap;

const NAME: &str = "binance";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BalanceItem {
    asset: String,
    balance: String,
    available_balance: String,
    cross_un_pnl: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AccountInfoV3 {
    total_initial_margin: String,
    total_maint_margin: String,
    total_margin_balance: String,
    available_balance: String,
    max_withdraw_amount: String,
    assets: Vec<AccountAssetV3>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountAssetV3 {
    asset: String,
    wallet_balance: String,
    unrealized_profit: String,
    initial_margin: String,
    available_balance: String,
}

pub(super) struct ParsedAccountRead {
    pub(super) balances: HashMap<String, BalanceInfo>,
    pub(super) summary: VenueAccountSummary,
}

#[derive(Debug)]
pub(super) struct ParsedPositions {
    pub(super) rows: Vec<PositionInfo>,
    pub(super) mode: Option<BinancePositionMode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PositionItem {
    symbol: String,
    position_amt: String,
    entry_price: String,
    mark_price: String,
    un_realized_profit: String,
    liquidation_price: String,
    notional: String,
    position_initial_margin: String,
    maint_margin: String,
    /// `BOTH` in one-way mode, `LONG` / `SHORT` in hedge mode.
    position_side: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OpenOrderItem {
    pub(super) order_id: u64,
    pub(super) symbol: String,
    pub(super) status: String,
    #[serde(rename = "type")]
    pub(super) order_type: String,
    pub(super) side: String,
    pub(super) price: String,
    pub(super) orig_qty: String,
    pub(super) executed_qty: String,
    pub(super) avg_price: String,
    pub(super) time: Option<i64>,
    pub(super) update_time: Option<i64>,
    /// Binance USDM post-only is `type=LIMIT, timeInForce=GTX`.
    pub(super) time_in_force: String,
    pub(super) client_order_id: String,
    pub(super) reduce_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub(super) enum BinancePositionMode {
    OneWay,
    Hedge,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PositionSideDualResponse {
    pub(super) dual_side_position: bool,
}

impl BinancePositionMode {
    pub(super) fn from_dual_side_position(value: bool) -> Self {
        if value {
            Self::Hedge
        } else {
            Self::OneWay
        }
    }

    pub(super) fn from_code(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::OneWay),
            1 => Some(Self::Hedge),
            _ => None,
        }
    }

    pub(super) fn code(self) -> i64 {
        match self {
            Self::OneWay => 0,
            Self::Hedge => 1,
        }
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::OneWay => "one_way",
            Self::Hedge => "hedge",
        }
    }

    pub(super) fn position_side_for_intent(self, intent: &OrderIntent) -> &'static str {
        match self {
            Self::OneWay => "BOTH",
            Self::Hedge if intent.reduce_only => match intent.side {
                OrderSide::Buy => "SHORT",
                OrderSide::Sell => "LONG",
            },
            Self::Hedge => match intent.side {
                OrderSide::Buy => "LONG",
                OrderSide::Sell => "SHORT",
            },
        }
    }
}

pub(super) fn parse_position_mode(row: &PositionSideDualResponse) -> BinancePositionMode {
    BinancePositionMode::from_dual_side_position(row.dual_side_position)
}

pub(super) fn parse_balances(
    items: Vec<BalanceItem>,
    currency: Option<&str>,
) -> ExchangeResult<HashMap<String, BalanceInfo>> {
    let mut out = HashMap::new();
    for item in items {
        let asset = required_text("asset", &item.asset, "balance row")?;
        let total = non_negative_number("balance", &item.balance, asset)?;
        let available = non_negative_number("availableBalance", &item.available_balance, asset)?;
        let unrealized_pnl = parse_number("crossUnPnl", &item.cross_un_pnl, asset)?;
        let non_available_total = total - available;
        if non_available_total < 0.0 {
            return Err(parse_error(
                "availableBalance",
                &item.available_balance,
                asset,
            ));
        }
        if currency.is_some_and(|c| !c.eq_ignore_ascii_case(&item.asset)) {
            continue;
        }
        out.insert(
            item.asset.clone(),
            BalanceInfo {
                currency: item.asset,
                total,
                available,
                frozen: non_available_total,
                unrealized_pnl,
            },
        );
    }
    Ok(out)
}

pub(super) fn parse_account_info_v3(
    item: AccountInfoV3,
    currency: Option<&str>,
    observed_at_ms: i64,
) -> ExchangeResult<ParsedAccountRead> {
    parse_account_info(
        item,
        currency,
        observed_at_ms,
        "binance.GET /fapi/v3/account",
    )
}

pub(super) fn parse_account_info(
    item: AccountInfoV3,
    currency: Option<&str>,
    observed_at_ms: i64,
    source: &str,
) -> ExchangeResult<ParsedAccountRead> {
    let total_equity =
        non_negative_number("totalMarginBalance", &item.total_margin_balance, "account")?;
    let initial_margin =
        non_negative_number("totalInitialMargin", &item.total_initial_margin, "account")?;
    let maintenance_margin =
        non_negative_number("totalMaintMargin", &item.total_maint_margin, "account")?;
    let available_balance =
        non_negative_number("availableBalance", &item.available_balance, "account")?;
    let withdrawable_balance =
        non_negative_number("maxWithdrawAmount", &item.max_withdraw_amount, "account")?;
    let balances = parse_account_assets(item.assets, currency)?;
    Ok(ParsedAccountRead {
        balances,
        summary: VenueAccountSummary {
            venue: NAME.to_owned(),
            account_type: "usds_m_futures".to_owned(),
            equity_scope: AccountEquityScope::Perpetuals,
            total_equity_usd: total_equity,
            total_available_balance_usd: available_balance,
            withdrawable_balance_usd: Some(withdrawable_balance),
            total_initial_margin_usd: initial_margin,
            total_maintenance_margin_usd: maintenance_margin,
            account_im_rate: margin_rate(initial_margin, total_equity),
            account_mm_rate: margin_rate(maintenance_margin, total_equity),
            source: source.to_owned(),
            observed_at_ms,
            freshness_ms: Some(0),
            problem: None,
        },
    })
}

fn parse_account_assets(
    items: Vec<AccountAssetV3>,
    currency: Option<&str>,
) -> ExchangeResult<HashMap<String, BalanceInfo>> {
    let mut out = HashMap::new();
    for item in items {
        let asset = required_text("asset", &item.asset, "account asset")?;
        let total = non_negative_number("walletBalance", &item.wallet_balance, asset)?;
        let available = non_negative_number("availableBalance", &item.available_balance, asset)?;
        let frozen = non_negative_number("initialMargin", &item.initial_margin, asset)?;
        let unrealized_pnl = parse_number("unrealizedProfit", &item.unrealized_profit, asset)?;
        if currency.is_some_and(|want| !want.eq_ignore_ascii_case(asset)) {
            continue;
        }
        out.insert(
            item.asset.clone(),
            BalanceInfo {
                currency: item.asset,
                total,
                available,
                frozen,
                unrealized_pnl,
            },
        );
    }
    Ok(out)
}

fn margin_rate(margin: f64, equity: f64) -> f64 {
    if equity > 0.0 {
        margin / equity
    } else {
        0.0
    }
}

pub(super) fn parse_positions(
    items: Vec<PositionItem>,
    target_exchange_symbol: Option<&str>,
) -> ExchangeResult<Vec<PositionInfo>> {
    let mut out = Vec::new();
    for item in items {
        let symbol = required_text("symbol", &item.symbol, "position row")?;
        let qty = parse_number("positionAmt", &item.position_amt, symbol)?;
        let parsed = position_info(&item, qty)?;
        if target_exchange_symbol.is_some_and(|target| item.symbol != target) {
            continue;
        }
        if qty == 0.0 {
            continue;
        }
        out.push(parsed);
    }
    pair_hedge_positions(&mut out);
    Ok(out)
}

pub(super) fn parse_positions_with_mode(
    items: Vec<PositionItem>,
    target_exchange_symbol: Option<&str>,
) -> ExchangeResult<ParsedPositions> {
    let mode = position_mode_from_positions(&items)?;
    let rows = parse_positions(items, target_exchange_symbol)?;
    Ok(ParsedPositions { rows, mode })
}

fn position_mode_from_positions(
    items: &[PositionItem],
) -> ExchangeResult<Option<BinancePositionMode>> {
    let mut active_mode = None;
    let mut placeholder_mode = None;
    let mut placeholder_modes_conflict = false;
    for item in items {
        let row_mode = match item.position_side.as_str() {
            "BOTH" => BinancePositionMode::OneWay,
            "LONG" | "SHORT" => BinancePositionMode::Hedge,
            other => return Err(parse_error("positionSide", other, &item.symbol)),
        };
        let quantity = parse_number("positionAmt", &item.position_amt, &item.symbol)?;
        if quantity != 0.0 {
            if active_mode.is_some_and(|mode| mode != row_mode) {
                return Err(parse_error(
                    "positionSide",
                    &item.position_side,
                    "mixed active account position modes",
                ));
            }
            active_mode = Some(row_mode);
            continue;
        }
        if placeholder_mode.is_some_and(|mode| mode != row_mode) {
            placeholder_modes_conflict = true;
        } else {
            placeholder_mode = Some(row_mode);
        }
    }
    if let Some(mode) = active_mode {
        return Ok(Some(mode));
    }
    if placeholder_modes_conflict {
        // An all-flat mixed response cannot prove the account-wide mode. Leave it uncached so
        // the signed position-mode endpoint remains the fail-closed fallback for the next write.
        return Ok(None);
    }
    Ok(placeholder_mode)
}

fn position_info(item: &PositionItem, qty: f64) -> ExchangeResult<PositionInfo> {
    let side = position_side_label(&item.position_side, qty, &item.symbol)?;
    let entry_price = non_negative_number("entryPrice", &item.entry_price, &item.symbol)?;
    let mark_price = non_negative_number("markPrice", &item.mark_price, &item.symbol)?;
    if qty != 0.0 && entry_price <= 0.0 {
        return Err(parse_error("entryPrice", &item.entry_price, &item.symbol));
    }
    if qty != 0.0 && mark_price <= 0.0 {
        return Err(parse_error("markPrice", &item.mark_price, &item.symbol));
    }
    let notional = parse_number("notional", &item.notional, &item.symbol)?.abs();
    let position_margin = non_negative_number(
        "positionInitialMargin",
        &item.position_initial_margin,
        &item.symbol,
    )?;
    let maintenance_margin = non_negative_number("maintMargin", &item.maint_margin, &item.symbol)?;
    let (leverage, maintenance_margin_ratio) = if qty == 0.0 {
        (1.0, 0.0)
    } else {
        if notional <= 0.0 {
            return Err(parse_error("notional", &item.notional, &item.symbol));
        }
        if position_margin <= 0.0 {
            return Err(parse_error(
                "positionInitialMargin",
                &item.position_initial_margin,
                &item.symbol,
            ));
        }
        if maintenance_margin <= 0.0 {
            return Err(parse_error("maintMargin", &item.maint_margin, &item.symbol));
        }
        // Position V3 omits the old raw leverage field. Its documented notional,
        // position initial margin and maintenance margin retain the exact risk facts.
        (
            notional / position_margin,
            maintenance_margin / position_margin,
        )
    };
    Ok(PositionInfo {
        symbol: strip_common_suffixes(&item.symbol),
        exchange: NAME.into(),
        side: side.to_owned(),
        quantity: qty.abs(),
        entry_price,
        mark_price,
        unrealized_pnl: parse_number("unRealizedProfit", &item.un_realized_profit, &item.symbol)?,
        leverage,
        // Binance documents `"0"` for an open position when its UI renders
        // liquidation price as `--`. Preserve that explicit venue value so the
        // account projection can distinguish it from a missing field.
        liquidation_price: Some(non_negative_number(
            "liquidationPrice",
            &item.liquidation_price,
            &item.symbol,
        )?),
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin: position_margin,
        maintenance_margin_ratio,
        position_mode: None,
        margin_mode: None,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    })
}

fn position_side_label(
    position_side: &str,
    quantity: f64,
    context: &str,
) -> ExchangeResult<&'static str> {
    match position_side {
        "LONG" => Ok("long"),
        "SHORT" => Ok("short"),
        "BOTH" if quantity > 0.0 => Ok("long"),
        "BOTH" if quantity < 0.0 => Ok("short"),
        "BOTH" => Ok("flat"),
        _ => Err(parse_error("positionSide", position_side, context)),
    }
}

pub(super) fn parse_open_order(item: &OpenOrderItem) -> ExchangeResult<OrderInfo> {
    if item.order_id == 0 {
        return Err(parse_error("orderId", "0", "order row"));
    }
    let scope = item.order_id.to_string();
    required_text("symbol", &item.symbol, &scope)?;
    let side = match item.side.as_str() {
        "BUY" => OrderSide::Buy,
        "SELL" => OrderSide::Sell,
        _ => return Err(parse_error("side", &item.side, &scope)),
    };
    let time_in_force = parse_time_in_force(&item.time_in_force, &scope)?;
    let order_type = match (item.order_type.as_str(), time_in_force) {
        ("MARKET", _) => OrderType::Market,
        ("LIMIT", "GTX") => OrderType::PostOnly,
        ("LIMIT", _) => OrderType::Limit,
        _ => {
            return Err(parse_error(
                "type/timeInForce",
                &format!("{}/{}", item.order_type, item.time_in_force),
                &scope,
            ))
        }
    };
    let status = match item.status.as_str() {
        "NEW" => OrderStatus::Open,
        "PARTIALLY_FILLED" => OrderStatus::PartiallyFilled,
        "FILLED" => OrderStatus::Filled,
        "CANCELED" => OrderStatus::Canceled,
        "REJECTED" => OrderStatus::Rejected,
        "EXPIRED" => OrderStatus::Expired,
        _ => return Err(parse_error("status", &item.status, &scope)),
    };
    let timestamp_ms = item
        .time
        .ok_or_else(|| parse_error("time", "missing", &scope))?;
    let created_at = parse_timestamp(timestamp_ms, &scope)?;
    if let Some(update_time) = item.update_time {
        parse_timestamp(update_time, &scope)?;
    }
    let quantity = positive_number("origQty", &item.orig_qty, &scope)?;
    let filled_quantity = non_negative_number("executedQty", &item.executed_qty, &scope)?;
    if filled_quantity > quantity {
        return Err(parse_error("executedQty", &item.executed_qty, &scope));
    }
    let price = match order_type {
        OrderType::Market => non_negative_number("price", &item.price, &scope)?,
        OrderType::Limit | OrderType::PostOnly => positive_number("price", &item.price, &scope)?,
    };
    let filled_price = non_negative_number("avgPrice", &item.avg_price, &scope)?;
    Ok(OrderInfo {
        execution_style: None,
        venue_time_in_force: Some(item.time_in_force.clone()),
        client_order_id: client_order_id_from_str(&item.client_order_id),
        reduce_only: Some(item.reduce_only),
        order_id: item.order_id.to_string(),
        symbol: strip_common_suffixes(&item.symbol),
        exchange: NAME.into(),
        side,
        order_type,
        status,
        quantity,
        price,
        filled_quantity,
        filled_price,
        fees: 0.0,
        created_at,
    })
}

fn parse_time_in_force<'a>(value: &'a str, context: &str) -> ExchangeResult<&'a str> {
    match value {
        "GTC" | "IOC" | "FOK" | "GTX" | "GTD" => Ok(value),
        _ => Err(parse_error("timeInForce", value, context)),
    }
}

fn parse_number(field: &str, value: &str, context: &str) -> ExchangeResult<f64> {
    let parsed = value
        .parse::<f64>()
        .map_err(|_| parse_error(field, value, context))?;
    if !parsed.is_finite() {
        return Err(parse_error(field, value, context));
    }
    Ok(parsed)
}

fn non_negative_number(field: &str, value: &str, context: &str) -> ExchangeResult<f64> {
    let parsed = parse_number(field, value, context)?;
    if parsed < 0.0 {
        return Err(parse_error(field, value, context));
    }
    Ok(parsed)
}

fn positive_number(field: &str, value: &str, context: &str) -> ExchangeResult<f64> {
    let parsed = parse_number(field, value, context)?;
    if parsed <= 0.0 {
        return Err(parse_error(field, value, context));
    }
    Ok(parsed)
}

fn required_text<'a>(field: &str, value: &'a str, context: &str) -> ExchangeResult<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        return Err(parse_error(field, value, context));
    }
    Ok(value)
}

fn parse_timestamp(
    timestamp_ms: i64,
    context: &str,
) -> ExchangeResult<chrono::DateTime<chrono::Utc>> {
    if timestamp_ms <= 0 {
        return Err(parse_error("time", &timestamp_ms.to_string(), context));
    }
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| parse_error("time", &timestamp_ms.to_string(), context))
}

fn parse_error(field: &str, value: &str, context: &str) -> ExchangeError {
    ExchangeError::Parse(format!(
        "binance private field {field} invalid for {context}: {value}"
    ))
}

#[cfg(test)]
#[path = "binance_private_data_tests.rs"]
mod tests;
