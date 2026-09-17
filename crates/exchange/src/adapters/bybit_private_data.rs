//! Bybit private read response parsing.
//!
//! Official Bybit V5 docs checked before moving these DTOs:
//! - GET /v5/account/wallet-balance
//! - GET /v5/position/list
//! - GET /v5/order/realtime

use crate::adapter::{client_order_id_from_str, strip_common_suffixes};
use crate::error::{ExchangeError, ExchangeResult};
use serde::Deserialize;
use shared_types::{
    AccountEquityScope, BalanceInfo, OrderInfo, OrderSide, OrderStatus, OrderType, PositionInfo,
    TimeInForce, VenueAccountSummary,
};
use std::collections::HashMap;

const NAME: &str = "bybit";
const ACCOUNT_SOURCE: &str = "bybit.GET /v5/account/wallet-balance";

#[derive(Debug, Deserialize)]
pub(super) struct WalletAccount {
    #[serde(rename = "accountType")]
    account_type: String,
    #[serde(rename = "totalAvailableBalance")]
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
    coin: Vec<CoinBalance>,
}

#[derive(Debug)]
pub(super) struct BybitAccountRead {
    pub(super) balances: HashMap<String, BalanceInfo>,
    pub(super) summary: VenueAccountSummary,
}

#[derive(Debug, Deserialize)]
struct CoinBalance {
    coin: String,
    equity: String,
    locked: String,
    #[serde(rename = "unrealisedPnl")]
    unrealised_pnl: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct PositionRow {
    symbol: String,
    side: String,
    size: String,
    #[serde(rename = "avgPrice")]
    avg_price: String,
    #[serde(rename = "markPrice")]
    mark_price: String,
    #[serde(rename = "unrealisedPnl")]
    unrealised_pnl: String,
    leverage: String,
    #[serde(rename = "liqPrice")]
    liq_price: String,
    #[serde(rename = "positionIM")]
    position_im: String,
    #[serde(rename = "positionIdx")]
    position_idx: i32,
}

/// The account-mode endpoint needs only Bybit's explicit position index.
/// Keep it separate from the full position read DTO so an account-mode
/// preflight cannot be rejected for unrelated position fields.
#[derive(Debug, Deserialize)]
pub(super) struct PositionModeRow {
    #[serde(rename = "positionIdx")]
    position_idx: i32,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenOrderRow {
    symbol: String,
    #[serde(rename = "orderId")]
    order_id: String,
    #[serde(rename = "orderStatus")]
    order_status: String,
    #[serde(rename = "orderType")]
    order_type: String,
    side: String,
    price: String,
    qty: String,
    #[serde(rename = "cumExecQty")]
    cum_exec_qty: String,
    #[serde(rename = "avgPrice")]
    avg_price: String,
    #[serde(rename = "createdTime")]
    created_time: String,
    #[serde(rename = "cumExecFee")]
    cum_exec_fee: String,
    #[serde(rename = "positionIdx")]
    position_idx: i32,
    #[serde(rename = "cancelType")]
    cancel_type: String,
    #[serde(rename = "rejectReason")]
    reject_reason: String,
    #[serde(rename = "leavesQty")]
    leaves_qty: String,
    #[serde(rename = "timeInForce")]
    time_in_force: String,
    #[serde(rename = "orderLinkId")]
    order_link_id: String,
    #[serde(rename = "reduceOnly")]
    reduce_only: bool,
}

pub(super) fn parse_balance_response(
    accounts: Vec<WalletAccount>,
    currency: Option<&str>,
) -> ExchangeResult<HashMap<String, BalanceInfo>> {
    let [account]: [WalletAccount; 1] = one_wallet_account(accounts)?;
    validate_unified_account(&account)?;
    let total_available_balance = parse_required_number(
        "wallet account",
        "totalAvailableBalance",
        &account.total_available_balance,
    )?;
    parse_coin_balance_rows(account.coin, currency, total_available_balance)
}

pub(super) fn parse_account_response(
    accounts: Vec<WalletAccount>,
    currency: Option<&str>,
    observed_at_ms: i64,
) -> ExchangeResult<BybitAccountRead> {
    let [account]: [WalletAccount; 1] = one_wallet_account(accounts)?;
    parse_account(account, currency, observed_at_ms)
}

fn one_wallet_account(accounts: Vec<WalletAccount>) -> ExchangeResult<[WalletAccount; 1]> {
    accounts.try_into().map_err(|accounts: Vec<WalletAccount>| {
        ExchangeError::Parse(format!(
            "bybit wallet expected one account row, got {}",
            accounts.len()
        ))
    })
}

pub(super) fn parse_positions(rows: &[PositionRow]) -> ExchangeResult<Vec<PositionInfo>> {
    let mut out: Vec<PositionInfo> = rows
        .iter()
        .map(parse_position)
        .filter_map(Result::transpose)
        .collect::<ExchangeResult<_>>()?;
    crate::adapter::pair_hedge_positions(&mut out);
    Ok(out)
}

pub(super) fn parse_open_orders(rows: Vec<OpenOrderRow>) -> ExchangeResult<Vec<OrderInfo>> {
    rows.into_iter().map(parse_open_order).collect()
}

#[cfg(test)]
pub(super) fn order_position_idx(
    rows: &[PositionRow],
    side: OrderSide,
    reduce_only: bool,
) -> ExchangeResult<u8> {
    let mode = parse_position_mode(rows)?;
    Ok(position_idx_for_mode(mode, side, reduce_only))
}

pub(super) fn order_position_idx_rows(
    rows: &[PositionModeRow],
    side: OrderSide,
    reduce_only: bool,
) -> ExchangeResult<u8> {
    let mode = parse_position_mode_rows(rows)?;
    Ok(position_idx_for_mode(mode, side, reduce_only))
}

fn position_idx_for_mode(mode: PositionModeEvidence, side: OrderSide, reduce_only: bool) -> u8 {
    match mode {
        PositionModeEvidence::OneWay => 0,
        PositionModeEvidence::Hedge => hedge_position_idx(side, reduce_only),
    }
}

pub(super) fn parse_open_order(order: OpenOrderRow) -> ExchangeResult<OrderInfo> {
    let OpenOrderRow {
        symbol,
        order_id,
        order_status,
        order_type,
        side,
        price,
        qty,
        cum_exec_qty,
        avg_price,
        created_time,
        cum_exec_fee,
        position_idx,
        cancel_type,
        reject_reason,
        leaves_qty,
        time_in_force,
        order_link_id,
        reduce_only,
    } = order;
    required_text("order", "orderId", &order_id)?;
    required_text("order", "symbol", &symbol)?;
    let scope = format!("order {order_id}");
    let venue_time_in_force = time_in_force.clone();
    let time_in_force = parse_time_in_force_evidence(&scope, &time_in_force)?;
    let parsed_order_type = parse_order_type(&scope, &order_type, time_in_force)?;
    parse_order_finality_evidence(
        &scope,
        position_idx,
        &leaves_qty,
        &cancel_type,
        &reject_reason,
    )?;
    let quantity = parse_positive_number(&scope, "qty", &qty)?;
    let filled_quantity = parse_non_negative_number(&scope, "cumExecQty", &cum_exec_qty)?;
    if filled_quantity > quantity {
        return Err(parse_error(&scope, "cumExecQty", &cum_exec_qty));
    }
    let price = parse_order_price(&scope, parsed_order_type, &price)?;
    let filled_price = parse_filled_price(&scope, &avg_price, filled_quantity)?;
    Ok(OrderInfo {
        execution_style: None,
        venue_time_in_force: Some(venue_time_in_force),
        client_order_id: client_order_id_from_str(&order_link_id),
        reduce_only: Some(reduce_only),
        order_id,
        symbol: strip_common_suffixes(&symbol),
        exchange: NAME.into(),
        side: parse_order_side(&scope, &side)?,
        order_type: parsed_order_type,
        status: parse_order_status(&scope, &order_status)?,
        quantity,
        price,
        filled_quantity,
        filled_price,
        fees: parse_required_number(&scope, "cumExecFee", &cum_exec_fee)?,
        created_at: parse_timestamp_ms(&scope, "createdTime", &created_time)?,
    })
}

fn parse_order_finality_evidence(
    scope: &str,
    position_idx: i32,
    leaves_qty: &str,
    cancel_type: &str,
    reject_reason: &str,
) -> ExchangeResult<()> {
    parse_position_idx_evidence(scope, position_idx)?;
    parse_non_negative_number(scope, "leavesQty", leaves_qty)?;
    parse_cancel_type(scope, cancel_type)?;
    parse_reject_reason(scope, reject_reason)?;
    Ok(())
}

fn parse_account(
    account: WalletAccount,
    requested_currency: Option<&str>,
    observed_at_ms: i64,
) -> ExchangeResult<BybitAccountRead> {
    validate_unified_account(&account)?;
    let total_available_balance = parse_required_number(
        "wallet account",
        "totalAvailableBalance",
        &account.total_available_balance,
    )?;
    let summary = VenueAccountSummary {
        venue: NAME.to_owned(),
        account_type: account.account_type,
        equity_scope: AccountEquityScope::Unified,
        total_equity_usd: parse_required_number(
            "wallet account",
            "totalEquity",
            &account.total_equity,
        )?,
        total_available_balance_usd: total_available_balance,
        withdrawable_balance_usd: None,
        total_initial_margin_usd: parse_non_negative_number(
            "wallet account",
            "totalInitialMargin",
            &account.total_initial_margin,
        )?,
        total_maintenance_margin_usd: parse_non_negative_number(
            "wallet account",
            "totalMaintenanceMargin",
            &account.total_maintenance_margin,
        )?,
        account_im_rate: parse_non_negative_number(
            "wallet account",
            "accountIMRate",
            &account.account_im_rate,
        )?,
        account_mm_rate: parse_non_negative_number(
            "wallet account",
            "accountMMRate",
            &account.account_mm_rate,
        )?,
        source: ACCOUNT_SOURCE.to_owned(),
        observed_at_ms,
        freshness_ms: Some(common::time::now_ms().saturating_sub(observed_at_ms).max(0)),
        problem: None,
    };
    let balances =
        parse_coin_balance_rows(account.coin, requested_currency, total_available_balance)?;
    Ok(BybitAccountRead { balances, summary })
}

fn validate_unified_account(account: &WalletAccount) -> ExchangeResult<()> {
    if account.account_type == "UNIFIED" {
        Ok(())
    } else {
        Err(parse_error(
            "wallet account",
            "accountType",
            &account.account_type,
        ))
    }
}

fn parse_coin_balance_rows(
    coins: Vec<CoinBalance>,
    requested_currency: Option<&str>,
    total_available_balance: f64,
) -> ExchangeResult<HashMap<String, BalanceInfo>> {
    let assigned_currency = available_currency(&coins, requested_currency);
    let mut balances = HashMap::new();
    for coin in coins {
        let currency = required_text("wallet coin", "coin", &coin.coin)?;
        let scope = format!("wallet coin {currency}");
        if requested_currency.is_some_and(|requested| !currency.eq_ignore_ascii_case(requested)) {
            continue;
        }
        let total = parse_required_number(&scope, "equity", &coin.equity)?;
        let frozen = parse_non_negative_number(&scope, "locked", &coin.locked)?;
        let unrealized_pnl = parse_required_number(&scope, "unrealisedPnl", &coin.unrealised_pnl)?;
        let available = assigned_currency
            .as_deref()
            .filter(|assigned| assigned.eq_ignore_ascii_case(currency))
            .map_or(0.0, |_| total_available_balance.max(0.0));
        balances.insert(
            currency.to_owned(),
            BalanceInfo {
                currency: currency.to_owned(),
                total,
                available,
                frozen,
                unrealized_pnl,
            },
        );
    }
    Ok(balances)
}

fn available_currency(coins: &[CoinBalance], filter: Option<&str>) -> Option<String> {
    let filtered = filter.filter(|value| is_stable_balance(value));
    filtered
        .and_then(|value| {
            coins
                .iter()
                .find(|coin| coin.coin.eq_ignore_ascii_case(value))
        })
        .or_else(|| {
            ["USDT", "USDC", "USD"].iter().find_map(|preferred| {
                coins
                    .iter()
                    .find(|coin| coin.coin.eq_ignore_ascii_case(preferred))
            })
        })
        .map(|coin| coin.coin.clone())
}

fn is_stable_balance(currency: &str) -> bool {
    matches!(
        currency.trim().to_ascii_uppercase().as_str(),
        "USDT" | "USDC" | "USD"
    )
}

fn parse_position(row: &PositionRow) -> ExchangeResult<Option<PositionInfo>> {
    let symbol = required_text("position", "symbol", &row.symbol)?;
    let scope = format!("position {symbol}");
    let qty = parse_non_negative_number(&scope, "size", &row.size)?;
    if qty == 0.0 {
        validate_empty_position(&scope, row)?;
        return Ok(None);
    }
    let side = parse_position_side(&scope, row)?;
    let liquidation_price = parse_optional_positive_number(&scope, "liqPrice", &row.liq_price)?;
    Ok(Some(PositionInfo {
        symbol: strip_common_suffixes(symbol),
        exchange: NAME.into(),
        side,
        quantity: qty,
        entry_price: parse_positive_number(&scope, "avgPrice", &row.avg_price)?,
        mark_price: parse_positive_number(&scope, "markPrice", &row.mark_price)?,
        unrealized_pnl: parse_required_number(&scope, "unrealisedPnl", &row.unrealised_pnl)?,
        leverage: parse_positive_number(&scope, "leverage", &row.leverage)?,
        liquidation_price,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin: parse_non_negative_number(&scope, "positionIM", &row.position_im)?,
        maintenance_margin_ratio: 0.0,
        position_mode: None,
        margin_mode: None,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }))
}

fn validate_empty_position(scope: &str, row: &PositionRow) -> ExchangeResult<()> {
    parse_position_idx_evidence(scope, row.position_idx)?;
    if !row.side.is_empty() {
        return Err(parse_error(scope, "side", &row.side));
    }
    parse_optional_non_negative_number(scope, "avgPrice", &row.avg_price)?;
    parse_optional_non_negative_number(scope, "markPrice", &row.mark_price)?;
    parse_optional_number(scope, "unrealisedPnl", &row.unrealised_pnl)?;
    parse_optional_positive_number(scope, "leverage", &row.leverage)?;
    parse_optional_positive_number(scope, "liqPrice", &row.liq_price)?;
    parse_optional_non_negative_number(scope, "positionIM", &row.position_im)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PositionModeEvidence {
    OneWay,
    Hedge,
}

impl PositionModeEvidence {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::OneWay => "one_way",
            Self::Hedge => "hedge",
        }
    }
}

#[cfg(test)]
pub(super) fn parse_position_mode(rows: &[PositionRow]) -> ExchangeResult<PositionModeEvidence> {
    parse_position_mode_indices(rows.iter().map(|row| row.position_idx))
}

pub(super) fn parse_position_mode_rows(
    rows: &[PositionModeRow],
) -> ExchangeResult<PositionModeEvidence> {
    parse_position_mode_indices(rows.iter().map(|row| row.position_idx))
}

fn parse_position_mode_indices(
    position_indices: impl IntoIterator<Item = i32>,
) -> ExchangeResult<PositionModeEvidence> {
    let mut found = false;
    let mut one_way = false;
    let mut hedge = false;
    for position_idx in position_indices {
        found = true;
        match position_idx {
            0 => one_way = true,
            1 | 2 => hedge = true,
            _ => {
                return Err(parse_error(
                    "position mode",
                    "positionIdx",
                    &position_idx.to_string(),
                ));
            }
        }
    }
    if !found {
        return Err(ExchangeError::Parse(
            "bybit position mode evidence empty".into(),
        ));
    }
    match (one_way, hedge) {
        (true, false) => Ok(PositionModeEvidence::OneWay),
        (false, true) => Ok(PositionModeEvidence::Hedge),
        _ => Err(ExchangeError::Parse(
            "bybit position mode evidence mixed".into(),
        )),
    }
}

fn hedge_position_idx(side: OrderSide, reduce_only: bool) -> u8 {
    match (side, reduce_only) {
        (OrderSide::Buy, false) | (OrderSide::Sell, true) => 1,
        (OrderSide::Sell, false) | (OrderSide::Buy, true) => 2,
    }
}

fn parse_required_number(scope: &str, field: &str, raw: &str) -> ExchangeResult<f64> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(parse_error(scope, field, raw));
    }
    parse_finite_number(scope, field, value)
}

fn parse_positive_number(scope: &str, field: &str, raw: &str) -> ExchangeResult<f64> {
    let value = parse_required_number(scope, field, raw)?;
    if value <= 0.0 {
        return Err(parse_error(scope, field, raw));
    }
    Ok(value)
}

fn parse_non_negative_number(scope: &str, field: &str, raw: &str) -> ExchangeResult<f64> {
    let value = parse_required_number(scope, field, raw)?;
    if value < 0.0 {
        return Err(parse_error(scope, field, raw));
    }
    Ok(value)
}

fn parse_optional_positive_number(
    scope: &str,
    field: &str,
    raw: &str,
) -> ExchangeResult<Option<f64>> {
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let value = parse_non_negative_number(scope, field, raw)?;
    Ok((value > 0.0).then_some(value))
}

fn parse_optional_number(scope: &str, field: &str, raw: &str) -> ExchangeResult<Option<f64>> {
    if raw.trim().is_empty() {
        return Ok(None);
    }
    parse_required_number(scope, field, raw).map(Some)
}

fn parse_optional_non_negative_number(
    scope: &str,
    field: &str,
    raw: &str,
) -> ExchangeResult<Option<f64>> {
    if raw.trim().is_empty() {
        return Ok(None);
    }
    parse_non_negative_number(scope, field, raw).map(Some)
}

fn parse_timestamp_ms(
    scope: &str,
    field: &str,
    raw: &str,
) -> ExchangeResult<chrono::DateTime<chrono::Utc>> {
    let timestamp = raw
        .trim()
        .parse::<i64>()
        .map_err(|_| parse_error(scope, field, raw))?;
    if timestamp <= 0 {
        return Err(parse_error(scope, field, raw));
    }
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp)
        .ok_or_else(|| parse_error(scope, field, raw))
}

fn parse_order_side(scope: &str, raw: &str) -> ExchangeResult<OrderSide> {
    match raw {
        "Buy" => Ok(OrderSide::Buy),
        "Sell" => Ok(OrderSide::Sell),
        _ => Err(parse_error(scope, "side", raw)),
    }
}

fn parse_order_type(
    scope: &str,
    raw: &str,
    time_in_force: TimeInForce,
) -> ExchangeResult<OrderType> {
    match (raw, time_in_force) {
        ("Market", _) => Ok(OrderType::Market),
        ("Limit", TimeInForce::Gtx) => Ok(OrderType::PostOnly),
        ("Limit", _) => Ok(OrderType::Limit),
        _ => Err(parse_error(scope, "orderType", raw)),
    }
}

fn parse_time_in_force_evidence(scope: &str, raw: &str) -> ExchangeResult<TimeInForce> {
    match raw {
        "GTC" => Ok(TimeInForce::Gtc),
        "IOC" => Ok(TimeInForce::Ioc),
        "FOK" => Ok(TimeInForce::Fok),
        "PostOnly" => Ok(TimeInForce::Gtx),
        "RPI" => Err(ExchangeError::UnsupportedCapability(
            "bybit rpi order read timeInForce",
        )),
        value => Err(parse_error(scope, "timeInForce", value)),
    }
}

fn parse_position_idx_evidence(scope: &str, value: i32) -> ExchangeResult<()> {
    match value {
        0..=2 => Ok(()),
        _ => Err(parse_error(scope, "positionIdx", &value.to_string())),
    }
}

fn parse_cancel_type(scope: &str, raw: &str) -> ExchangeResult<()> {
    if matches!(
        raw,
        "UNKNOWN"
            | "CancelByUser"
            | "CancelByReduceOnly"
            | "CancelByPrepareLiq"
            | "CancelAllBeforeLiq"
            | "CancelByPrepareAdl"
            | "CancelAllBeforeAdl"
            | "CancelByAdmin"
            | "CancelBySettle"
            | "CancelByTpSlTsClear"
            | "CancelBySmp"
            | "CancelByDCP"
            | "CancelByRebalance"
            | "CancelByOCOTpCanceledBySlTriggered"
            | "CancelByOCOSlCanceledByTpTriggered"
            | "CancelByCannotAffordOrderCost"
            | "CancelByPmTrialMmOverEquity"
            | "CancelByAccountBlocking"
            | "CancelByDelivery"
            | "CancelByMmpTriggered"
            | "CancelByCrossSelfMuch"
            | "CancelByCrossReachMaxTradeNum"
    ) {
        Ok(())
    } else {
        Err(parse_error(scope, "cancelType", raw))
    }
}

fn parse_reject_reason(scope: &str, raw: &str) -> ExchangeResult<()> {
    if matches!(
        raw,
        "EC_NoError"
            | "EC_Others"
            | "EC_UnknownMessageType"
            | "EC_MissingClOrdID"
            | "EC_MissingOrigClOrdID"
            | "EC_ClOrdIDOrigClOrdIDAreTheSame"
            | "EC_DuplicatedClOrdID"
            | "EC_OrigClOrdIDDoesNotExist"
            | "EC_TooLateToCancel"
            | "EC_UnknownOrderType"
            | "EC_UnknownSide"
            | "EC_UnknownTimeInForce"
            | "EC_WronglyRouted"
            | "EC_MarketOrderPriceIsNotZero"
            | "EC_LimitOrderInvalidPrice"
            | "EC_NoEnoughQtyToFill"
            | "EC_NoImmediateQtyToFill"
            | "EC_PerCancelRequest"
            | "EC_MarketOrderCannotBePostOnly"
            | "EC_PostOnlyWillTakeLiquidity"
            | "EC_CancelReplaceOrder"
            | "EC_InvalidSymbolStatus"
            | "EC_CancelForNoFullFill"
            | "EC_BySelfMatch"
            | "EC_InCallAuctionStatus"
            | "EC_QtyCannotBeZero"
            | "EC_MarketOrderNoSupportTIF"
            | "EC_ReachMaxTradeNum"
            | "EC_InvalidPriceScale"
            | "EC_BitIndexInvalid"
            | "EC_StopBySelfMatch"
            | "EC_InvalidSmpType"
            | "EC_CancelByMMP"
            | "EC_InvalidUserType"
            | "EC_InvalidMirrorOid"
            | "EC_InvalidMirrorUid"
            | "EC_EcInvalidQty"
            | "EC_InvalidAmount"
            | "EC_LoadOrderCancel"
            | "EC_MarketQuoteNoSuppSell"
            | "EC_DisorderOrderID"
            | "EC_InvalidBaseValue"
            | "EC_LoadOrderCanMatch"
            | "EC_SecurityStatusFail"
            | "EC_ReachRiskPriceLimit"
            | "EC_OrderNotExist"
            | "EC_CancelByOrderValueZero"
            | "EC_CancelByMatchValueZero"
            | "EC_ReachMarketPriceLimit"
    ) {
        Ok(())
    } else {
        Err(parse_error(scope, "rejectReason", raw))
    }
}

fn parse_order_status(scope: &str, raw: &str) -> ExchangeResult<OrderStatus> {
    match raw {
        "New" => Ok(OrderStatus::Open),
        "Untriggered" => Ok(OrderStatus::Pending),
        "Triggered" => Err(ExchangeError::UnsupportedCapability(
            "bybit transient triggered order read status",
        )),
        "PartiallyFilled" => Ok(OrderStatus::PartiallyFilled),
        "Filled" => Ok(OrderStatus::Filled),
        "Cancelled" | "PartiallyFilledCanceled" | "Deactivated" => Ok(OrderStatus::Canceled),
        "Rejected" => Ok(OrderStatus::Rejected),
        "Expired" => Ok(OrderStatus::Expired),
        _ => Err(parse_error(scope, "orderStatus", raw)),
    }
}

fn parse_position_side(scope: &str, row: &PositionRow) -> ExchangeResult<String> {
    match (row.position_idx, row.side.as_str()) {
        (0 | 1, "Buy") => Ok("long".to_owned()),
        (0 | 2, "Sell") => Ok("short".to_owned()),
        _ => Err(parse_error(scope, "positionIdx/side", &row.side)),
    }
}

fn parse_order_price(scope: &str, order_type: OrderType, raw: &str) -> ExchangeResult<f64> {
    match order_type {
        OrderType::Market => parse_blank_as_zero(scope, "price", raw),
        OrderType::Limit | OrderType::PostOnly => parse_positive_number(scope, "price", raw),
    }
}

fn parse_filled_price(scope: &str, raw: &str, filled_quantity: f64) -> ExchangeResult<f64> {
    if raw.trim().is_empty() {
        return (filled_quantity == 0.0)
            .then_some(0.0)
            .ok_or_else(|| parse_error(scope, "avgPrice", raw));
    }
    let price = parse_non_negative_number(scope, "avgPrice", raw)?;
    if filled_quantity > 0.0 && price <= 0.0 {
        return Err(parse_error(scope, "avgPrice", raw));
    }
    Ok(price)
}

fn parse_blank_as_zero(scope: &str, field: &str, raw: &str) -> ExchangeResult<f64> {
    if raw.trim().is_empty() {
        return Ok(0.0);
    }
    parse_non_negative_number(scope, field, raw)
}

fn required_text<'a>(scope: &str, field: &str, raw: &'a str) -> ExchangeResult<&'a str> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(parse_error(scope, field, raw));
    }
    Ok(value)
}

fn parse_finite_number(scope: &str, field: &str, value: &str) -> ExchangeResult<f64> {
    let parsed = value
        .parse::<f64>()
        .map_err(|_| parse_error(scope, field, value))?;
    if !parsed.is_finite() {
        return Err(parse_error(scope, field, value));
    }
    Ok(parsed)
}

fn parse_error(scope: &str, field: &str, value: &str) -> ExchangeError {
    ExchangeError::Parse(format!("bybit {scope} field {field} invalid: {value}"))
}

#[cfg(test)]
#[path = "bybit_private_data_tests.rs"]
mod tests;
