//! OKX live/demo private read response parsing.
//!
//! Official OKX V5 docs checked before moving these DTOs:
//! - `GET /api/v5/account/balance`
//! - `GET /api/v5/account/positions`
//! - `GET /api/v5/trade/order`
//! - `GET /api/v5/trade/orders-pending`

use crate::adapter::{client_order_id_from_str, strip_common_suffixes};
use crate::adapters::okx_read_parse::{
    parse_optional_reduce_only, parse_optional_zero_number, parse_order_side, parse_order_status,
    parse_order_type, parse_position_side, parse_positive_number_option, parse_required_number,
    parse_timestamp_ms,
};
use crate::error::ExchangeResult;
use crate::live::{venue_balance_rows, VenueAccountRead};
use serde::Deserialize;
use shared_types::{
    AccountEquityScope, BalanceInfo, OrderInfo, OrderType, PositionInfo, VenueAccountSummary,
    VenueBalanceInfo,
};

const NAME: &str = "okx";

#[derive(Debug, Deserialize)]
pub(super) struct OrderRow {
    #[serde(default, rename = "instId")]
    inst_id: String,
    #[serde(default, rename = "ordId")]
    ord_id: String,
    #[serde(default, rename = "clOrdId")]
    cl_ord_id: String,
    #[serde(default, rename = "reduceOnly")]
    reduce_only: String,
    #[serde(default)]
    state: String,
    #[serde(default, rename = "ordType")]
    ord_type: String,
    #[serde(default)]
    side: String,
    #[serde(default)]
    px: String,
    #[serde(default)]
    sz: String,
    #[serde(default, rename = "accFillSz")]
    acc_fill_sz: String,
    #[serde(default, rename = "avgPx")]
    avg_px: String,
    #[serde(default, rename = "cTime")]
    c_time: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct AccountBalanceItem {
    #[serde(default, rename = "totalEq")]
    total_eq: String,
    #[serde(default, rename = "availEq")]
    available_equity: String,
    #[serde(default)]
    imr: String,
    #[serde(default)]
    mmr: String,
    #[serde(default)]
    details: Vec<BalanceDetail>,
}

#[derive(Debug, Deserialize)]
struct BalanceDetail {
    ccy: String,
    #[serde(default)]
    eq: String,
    #[serde(default, rename = "availBal")]
    avail_bal: String,
    #[serde(default, rename = "frozenBal")]
    frozen_bal: String,
    #[serde(default)]
    upl: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct PositionRow {
    #[serde(rename = "instId")]
    inst_id: String,
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
    imr: String,
    #[serde(default, rename = "posSide")]
    pos_side: String,
}

pub(super) fn parse_balances(
    rows: Vec<AccountBalanceItem>,
    currency: Option<&str>,
) -> ExchangeResult<Vec<VenueBalanceInfo>> {
    let mut balances = std::collections::HashMap::new();
    for detail in rows.into_iter().flat_map(|row| row.details) {
        if !balance_currency_matches(currency, &detail.ccy) {
            continue;
        }
        let balance = balance_from_detail(detail)?;
        balances.insert(balance.currency.clone(), balance);
    }
    Ok(venue_balance_rows(NAME, balances))
}

pub(super) fn parse_account_read(
    rows: Vec<AccountBalanceItem>,
    currency: Option<&str>,
    observed_at_ms: i64,
) -> ExchangeResult<VenueAccountRead> {
    let [account]: [AccountBalanceItem; 1] =
        rows.try_into().map_err(|rows: Vec<AccountBalanceItem>| {
            crate::error::ExchangeError::Parse(format!(
                "okx account balance expected one account row, got {}",
                rows.len()
            ))
        })?;
    let total_equity_usd = parse_required_number("account balance", "totalEq", &account.total_eq)?;
    let total_available_balance_usd =
        parse_optional_zero_number("account balance", "availEq", &account.available_equity)?;
    let total_initial_margin_usd =
        parse_optional_zero_number("account balance", "imr", &account.imr)?;
    let total_maintenance_margin_usd =
        parse_optional_zero_number("account balance", "mmr", &account.mmr)?;
    let summary = VenueAccountSummary {
        venue: NAME.to_owned(),
        account_type: "trading_account".to_owned(),
        equity_scope: AccountEquityScope::Unified,
        total_equity_usd,
        total_available_balance_usd,
        withdrawable_balance_usd: None,
        total_initial_margin_usd,
        total_maintenance_margin_usd,
        account_im_rate: account_ratio(total_initial_margin_usd, total_equity_usd),
        account_mm_rate: account_ratio(total_maintenance_margin_usd, total_equity_usd),
        source: "okx.GET /api/v5/account/balance".to_owned(),
        observed_at_ms,
        freshness_ms: Some(0),
        problem: None,
    };
    Ok(VenueAccountRead {
        balances: parse_balances(vec![account], currency)?,
        summaries: vec![summary],
        asset_valuations: Vec::new(),
        issues: Vec::new(),
    })
}

fn account_ratio(value: f64, equity: f64) -> f64 {
    if equity > 0.0 {
        value / equity
    } else {
        0.0
    }
}

pub(super) fn parse_positions(
    rows: &[PositionRow],
    target: Option<&str>,
) -> ExchangeResult<Vec<PositionInfo>> {
    rows.iter()
        .filter(|row| target.map(|wanted| row.inst_id == wanted).unwrap_or(true))
        .map(parse_position)
        .filter_map(Result::transpose)
        .collect()
}

pub(super) fn parse_order(row: OrderRow) -> ExchangeResult<OrderInfo> {
    let scope = format!("order {}", row.ord_id);
    let order_type = parse_order_type(&scope, &row.ord_type)?;
    let price = parse_order_price(&scope, order_type, &row.px)?;
    Ok(OrderInfo {
        execution_style: None,
        venue_time_in_force: Some(row.ord_type.clone()),
        client_order_id: client_order_id_from_str(&row.cl_ord_id),
        reduce_only: parse_optional_reduce_only(&scope, &row.reduce_only)?,
        order_id: row.ord_id,
        symbol: strip_common_suffixes(&row.inst_id),
        exchange: NAME.into(),
        side: parse_order_side(&scope, &row.side)?,
        order_type,
        status: parse_order_status(&scope, &row.state)?,
        quantity: parse_required_number(&scope, "sz", &row.sz)?,
        price,
        filled_quantity: parse_required_number(&scope, "accFillSz", &row.acc_fill_sz)?,
        filled_price: parse_optional_zero_number(&scope, "avgPx", &row.avg_px)?,
        fees: 0.0,
        created_at: parse_timestamp_ms(&scope, "cTime", &row.c_time)?,
    })
}

fn balance_currency_matches(currency: Option<&str>, asset: &str) -> bool {
    currency
        .map(|want| want.eq_ignore_ascii_case(asset))
        .unwrap_or(true)
}

fn balance_from_detail(detail: BalanceDetail) -> ExchangeResult<BalanceInfo> {
    let scope = format!("balance {}", detail.ccy);
    Ok(BalanceInfo {
        currency: detail.ccy,
        total: parse_required_number(&scope, "eq", &detail.eq)?,
        available: parse_required_number(&scope, "availBal", &detail.avail_bal)?,
        frozen: parse_required_number(&scope, "frozenBal", &detail.frozen_bal)?,
        unrealized_pnl: parse_optional_zero_number(&scope, "upl", &detail.upl)?,
    })
}

fn parse_position(row: &PositionRow) -> ExchangeResult<Option<PositionInfo>> {
    let scope = format!("position {}", row.inst_id);
    let qty = parse_required_number(&scope, "pos", &row.pos)?;
    if qty == 0.0 {
        return Ok(None);
    }
    let side = parse_position_side(&scope, &row.pos_side, qty)?;
    let leverage = parse_optional_zero_number(&scope, "lever", &row.lever)?.max(1.0);
    Ok(Some(PositionInfo {
        symbol: strip_common_suffixes(&row.inst_id),
        exchange: NAME.into(),
        side,
        quantity: qty.abs(),
        entry_price: parse_required_number(&scope, "avgPx", &row.avg_px)?,
        mark_price: parse_required_number(&scope, "markPx", &row.mark_px)?,
        unrealized_pnl: parse_required_number(&scope, "upl", &row.upl)?,
        leverage,
        liquidation_price: parse_positive_number_option(&scope, "liqPx", &row.liq_px)?,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin: parse_optional_zero_number(&scope, "imr", &row.imr)?,
        maintenance_margin_ratio: 0.0,
        position_mode: None,
        margin_mode: None,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }))
}

fn parse_order_price(scope: &str, order_type: OrderType, raw: &str) -> ExchangeResult<f64> {
    match order_type {
        OrderType::Market => parse_optional_zero_number(scope, "px", raw),
        OrderType::Limit | OrderType::PostOnly => parse_required_number(scope, "px", raw),
    }
}
