//! Hyperliquid private read response parsing.
//!
//! Official Hyperliquid docs checked before moving these DTOs:
//! - <https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint>
//! - <https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/subscriptions>
//!
//! Account reads use the configured account address, never the API/agent signer.
//! The configured REST `clearinghouseState` reader receives its DEX scope from
//! the request. The live `allDexsClearinghouseState` WebSocket path is parsed
//! by `hyperliquid_ws_user_data`, so this module does not maintain a duplicate
//! account-state decoder.

use crate::adapter::{client_order_id_from_str, strip_common_suffixes};
use crate::adapters::hyperliquid_market_data::clean_hyperliquid_symbol;
use crate::error::{ExchangeError, ExchangeResult};
use serde::Deserialize;
use shared_types::{
    AccountEquityScope, BalanceInfo, OrderInfo, OrderSide, OrderStatus, OrderType, PositionInfo,
    VenueAccountSummary,
};
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub(super) struct ClearinghouseState {
    #[serde(rename = "marginSummary")]
    margin_summary: MarginSummary,
    #[serde(rename = "crossMarginSummary")]
    cross_margin_summary: MarginSummary,
    #[serde(rename = "crossMaintenanceMarginUsed")]
    cross_maintenance_margin_used: String,
    withdrawable: String,
    #[serde(rename = "assetPositions")]
    asset_positions: Vec<AssetPositionEntry>,
}

impl ClearinghouseState {
    pub(super) fn has_position_rows(&self) -> bool {
        !self.asset_positions.is_empty()
    }
}

#[derive(Debug, Deserialize, Clone)]
struct MarginSummary {
    #[serde(rename = "accountValue")]
    account_value: String,
    #[serde(rename = "totalNtlPos")]
    total_ntl_pos: String,
    #[serde(rename = "totalRawUsd")]
    total_raw_usd: String,
    #[serde(rename = "totalMarginUsed")]
    total_margin_used: String,
}

#[derive(Debug, Deserialize)]
struct AssetPositionEntry {
    #[serde(rename = "type")]
    position_type: String,
    position: AssetPosition,
}

#[derive(Debug, Deserialize)]
struct AssetPosition {
    coin: String,
    szi: String,
    #[serde(default, rename = "entryPx")]
    entry_px: Option<String>,
    #[serde(default, rename = "liquidationPx")]
    liquidation_px: Option<String>,
    #[serde(rename = "marginUsed")]
    margin_used: String,
    #[serde(rename = "unrealizedPnl")]
    unrealized_pnl: String,
    leverage: Option<LeverageEntry>,
}

#[derive(Debug, Deserialize)]
struct LeverageEntry {
    #[serde(rename = "type")]
    margin_mode: String,
    value: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct HyperliquidMarginEvidence {
    pub(super) account_value: f64,
    pub(super) total_ntl_pos: f64,
    pub(super) total_raw_usd: f64,
    pub(super) total_margin_used: f64,
}

/// Exact account-level margin facts exposed by Hyperliquid. These remain
/// account-scoped rather than being invented as a per-position ratio.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct HyperliquidAccountMarginEvidence {
    pub(super) margin_summary: HyperliquidMarginEvidence,
    pub(super) cross_margin_summary: HyperliquidMarginEvidence,
    pub(super) cross_maintenance_margin_used: f64,
    pub(super) withdrawable: f64,
}

#[derive(Debug, Deserialize)]
pub(super) struct SpotClearinghouseState {
    balances: Vec<SpotBalance>,
}

#[derive(Debug, Deserialize)]
struct SpotBalance {
    coin: String,
    total: String,
    hold: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenOrderItem {
    coin: String,
    side: String,
    oid: i64,
    #[serde(rename = "limitPx")]
    limit_px: String,
    #[serde(rename = "origSz")]
    orig_sz: String,
    sz: String,
    timestamp: i64,
    #[serde(default, rename = "orderType")]
    order_type: Option<String>,
    #[serde(default)]
    tif: Option<String>,
    #[serde(rename = "isTrigger")]
    is_trigger: bool,
    #[serde(rename = "reduceOnly")]
    reduce_only: bool,
    #[serde(default)]
    cloid: Option<String>,
}

/// REST `userFills` evidence used to complete partial-fill economics by `oid`.
/// Official schema: <https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint#retrieve-a-users-fills>
#[derive(Debug, Deserialize)]
pub(super) struct UserFillItem {
    coin: String,
    oid: i64,
    px: String,
    sz: String,
    fee: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct OrderStatusPayload {
    status: String,
    #[serde(default)]
    order: Option<OrderStatusEntry>,
}

#[derive(Debug, Deserialize)]
struct OrderStatusEntry {
    order: OpenOrderItem,
    status: String,
    #[serde(rename = "statusTimestamp")]
    status_timestamp: i64,
}

pub(super) fn parse_spot_balances(
    state: SpotClearinghouseState,
    currency: Option<&str>,
) -> ExchangeResult<HashMap<String, BalanceInfo>> {
    let mut out = HashMap::new();
    for balance in state.balances {
        let coin = required_text("spot.coin", &balance.coin)?;
        if let Some(want) = currency {
            if !want.eq_ignore_ascii_case(coin) {
                continue;
            }
        }
        let total = required_non_negative_number("spot.total", &balance.total)?;
        let frozen = required_non_negative_number("spot.hold", &balance.hold)?;
        if frozen > total {
            return Err(ExchangeError::Parse(format!(
                "hyperliquid spot hold exceeds total: coin={} hold={frozen} total={total}",
                balance.coin
            )));
        }
        out.insert(
            coin.to_owned(),
            BalanceInfo {
                currency: coin.to_owned(),
                total,
                available: total - frozen,
                frozen,
                unrealized_pnl: 0.0,
            },
        );
    }
    Ok(out)
}

pub(super) fn parse_perp_balance(
    state: &ClearinghouseState,
    currency: Option<&str>,
) -> ExchangeResult<HashMap<String, BalanceInfo>> {
    let key = "USDC";
    if let Some(want) = currency {
        if !want.eq_ignore_ascii_case(key) {
            return Ok(HashMap::new());
        }
    }
    let margin = parse_account_margin_evidence(state)?;
    let unrealized_pnl = state
        .asset_positions
        .iter()
        .map(|entry| {
            let _ = hyperliquid_position_mode(&entry.position_type)?;
            required_text("position.coin", &entry.position.coin)?;
            required_number("position.unrealizedPnl", &entry.position.unrealized_pnl)
        })
        .sum::<ExchangeResult<f64>>()?;
    Ok(HashMap::from([(
        key.to_owned(),
        BalanceInfo {
            currency: key.to_owned(),
            total: margin.margin_summary.account_value,
            available: margin.withdrawable,
            frozen: margin.margin_summary.total_margin_used,
            unrealized_pnl,
        },
    )]))
}

pub(super) fn parse_account_margin_evidence(
    state: &ClearinghouseState,
) -> ExchangeResult<HyperliquidAccountMarginEvidence> {
    Ok(HyperliquidAccountMarginEvidence {
        margin_summary: parse_margin_summary(&state.margin_summary, "marginSummary")?,
        cross_margin_summary: parse_margin_summary(
            &state.cross_margin_summary,
            "crossMarginSummary",
        )?,
        cross_maintenance_margin_used: required_non_negative_number(
            "crossMaintenanceMarginUsed",
            &state.cross_maintenance_margin_used,
        )?,
        withdrawable: required_non_negative_number("withdrawable", &state.withdrawable)?,
    })
}

pub(super) fn parse_account_summary(
    state: &ClearinghouseState,
    venue: &str,
    observed_at_ms: i64,
) -> ExchangeResult<VenueAccountSummary> {
    let evidence = parse_account_margin_evidence(state)?;
    let equity = evidence.margin_summary.account_value;
    Ok(VenueAccountSummary {
        venue: venue.to_owned(),
        account_type: "perpetuals".to_owned(),
        equity_scope: AccountEquityScope::Perpetuals,
        total_equity_usd: equity,
        total_available_balance_usd: evidence.withdrawable,
        withdrawable_balance_usd: Some(evidence.withdrawable),
        total_initial_margin_usd: evidence.margin_summary.total_margin_used,
        total_maintenance_margin_usd: evidence.cross_maintenance_margin_used,
        account_im_rate: account_ratio(evidence.margin_summary.total_margin_used, equity),
        account_mm_rate: account_ratio(evidence.cross_maintenance_margin_used, equity),
        source: "hyperliquid.POST /info clearinghouseState".to_owned(),
        observed_at_ms,
        freshness_ms: Some(common::time::now_ms().saturating_sub(observed_at_ms).max(0)),
        problem: None,
    })
}

pub(super) fn parse_positions(
    state: ClearinghouseState,
    target: Option<&str>,
    mark_map: &HashMap<String, f64>,
    exchange: &str,
) -> ExchangeResult<Vec<PositionInfo>> {
    // Ensure account-wide equity, margin, maintenance, and withdrawable facts
    // exist before returning any position derived from the same snapshot.
    let _margin = parse_account_margin_evidence(&state)?;
    state
        .asset_positions
        .into_iter()
        .filter_map(|entry| parse_position(entry, target, mark_map, exchange).transpose())
        .collect()
}

#[cfg(test)]
pub(super) fn parse_open_orders(
    orders: Vec<OpenOrderItem>,
    target: Option<&str>,
    exchange: &str,
) -> ExchangeResult<Vec<OrderInfo>> {
    parse_open_orders_with_fills(orders, &[], target, exchange)
}

pub(super) fn parse_open_orders_with_fills(
    orders: Vec<OpenOrderItem>,
    fills: &[UserFillItem],
    target: Option<&str>,
    exchange: &str,
) -> ExchangeResult<Vec<OrderInfo>> {
    // 部分成交的 resting 单是正常运行中必然出现的状态，但本端点缺 fill
    // price/fee 证据：只跳过该行并告警，不让整个 get_open_orders 失败——
    // 此前账户里只要有一张部分成交挂单，该 venue 的对账在部分成交存续期间
    // 整体停摆（恰是最需要对账的窗口）。被跳过的行以 RemoteMissing diff 触发
    // 按单 orderStatus/userFills 修复路径。结构性坏数据（bad side、缺
    // orderType 等 schema 漂移信号）仍整表 fail-closed。
    let mut parsed = Vec::with_capacity(orders.len());
    for order in orders.into_iter().filter(|order| match target {
        Some(want) => order.coin == want,
        None => true,
    }) {
        match parse_open_order(&order, fills, exchange)? {
            OpenOrderRow::Ready(info) => parsed.push(*info),
            OpenOrderRow::NeedsFillEvidence => tracing::warn!(
                exchange,
                coin = %order.coin,
                oid = ?order.oid,
                "hyperliquid partially filled open order skipped pending userFills evidence; remaining rows stay reconcilable"
            ),
        }
    }
    Ok(parsed)
}

pub(super) fn open_orders_need_fill_evidence(orders: &[OpenOrderItem]) -> ExchangeResult<bool> {
    orders
        .iter()
        .map(open_order_filled_quantity)
        .try_fold(false, |needed, filled| {
            filled.map(|filled| needed || filled > 0.0)
        })
}

/// 单行解析结果：结构性错误走 `Err`（整表 fail-closed），
/// 部分成交缺 fill 证据是可跳过的良性行。
enum OpenOrderRow {
    Ready(Box<OrderInfo>),
    NeedsFillEvidence,
}

struct FillEvidence {
    average_price: f64,
    fee: f64,
}

#[cfg(test)]
pub(super) fn order_status_to_info(
    payload: OrderStatusPayload,
    target: &str,
    exchange: &str,
) -> ExchangeResult<Option<OrderInfo>> {
    order_status_to_info_with_fills(payload, &[], target, exchange)
}

pub(super) fn order_status_to_info_with_fills(
    payload: OrderStatusPayload,
    fills: &[UserFillItem],
    target: &str,
    exchange: &str,
) -> ExchangeResult<Option<OrderInfo>> {
    if payload.status == "unknownOid" && payload.order.is_none() {
        return Ok(None);
    }
    if payload.status != "order" {
        return Err(ExchangeError::Parse(format!(
            "hyperliquid orderStatus unsupported status: {}",
            payload.status
        )));
    }
    let Some(entry) = payload.order else {
        return Err(ExchangeError::Parse(
            "hyperliquid orderStatus missing order payload".into(),
        ));
    };
    if entry.order.coin.as_str() != target {
        return Ok(None);
    }
    parse_order_status(&entry, fills, exchange).map(Some)
}

pub(super) fn order_status_needs_fill_evidence(
    payload: &OrderStatusPayload,
) -> ExchangeResult<bool> {
    payload
        .order
        .as_ref()
        .map(|entry| open_order_filled_quantity(&entry.order).map(|filled| filled > 0.0))
        .transpose()
        .map(Option::unwrap_or_default)
}

fn parse_position(
    entry: AssetPositionEntry,
    target: Option<&str>,
    mark_map: &HashMap<String, f64>,
    exchange: &str,
) -> ExchangeResult<Option<PositionInfo>> {
    let position_mode = hyperliquid_position_mode(&entry.position_type)?;
    let position = entry.position;
    if target.is_some_and(|want| position.coin != want) {
        return Ok(None);
    }
    let coin = required_text("position.coin", &position.coin)?;
    let quantity = required_number("position.szi", &position.szi)?;
    if quantity == 0.0 {
        return Ok(None);
    }
    let side = if quantity > 0.0 { "long" } else { "short" };
    let entry_price = position
        .entry_px
        .as_deref()
        .ok_or_else(|| missing_field("position.entryPx"))
        .and_then(|value| required_positive_number("position.entryPx", value))?;
    let leverage = position
        .leverage
        .as_ref()
        .ok_or_else(|| missing_field("position.leverage"))?;
    let margin_mode = match leverage.margin_mode.as_str() {
        "cross" | "isolated" => leverage.margin_mode.clone(),
        other => {
            return Err(ExchangeError::Parse(format!(
                "hyperliquid unsupported leverage type: {other}"
            )));
        }
    };
    let leverage_value = required_positive_f64("position.leverage.value", leverage.value)?;
    Ok(Some(PositionInfo {
        symbol: normalize_symbol(coin),
        exchange: exchange.to_owned(),
        side: side.to_owned(),
        quantity: quantity.abs(),
        entry_price,
        mark_price: mark_price(coin, mark_map)?,
        unrealized_pnl: required_number("position.unrealizedPnl", &position.unrealized_pnl)?,
        leverage: leverage_value,
        liquidation_price: positive_optional(
            "position.liquidationPx",
            position.liquidation_px.as_deref(),
        )?,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin: required_non_negative_number("position.marginUsed", &position.margin_used)?,
        maintenance_margin_ratio: 0.0,
        position_mode: Some(position_mode.to_owned()),
        margin_mode: Some(margin_mode),
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }))
}

fn parse_open_order(
    order: &OpenOrderItem,
    fills: &[UserFillItem],
    exchange: &str,
) -> ExchangeResult<OpenOrderRow> {
    let coin = required_text("openOrder.coin", &order.coin)?;
    let side = order_side(&order.side)?;
    let orig = required_positive_number("openOrder.origSz", &order.orig_sz)?;
    let filled = open_order_filled_quantity(order)?;
    let fill_evidence = if filled > 0.0 {
        match fill_evidence(order, fills, filled)? {
            Some(evidence) => Some(evidence),
            None => return Ok(OpenOrderRow::NeedsFillEvidence),
        }
    } else {
        None
    };
    let created_at = timestamp_ms("openOrder.timestamp", order.timestamp)?;
    let order_type = classify_hyperliquid_order_type(order)?;
    Ok(OpenOrderRow::Ready(Box::new(OrderInfo {
        execution_style: None,
        venue_time_in_force: order.tif.clone(),
        client_order_id: parse_client_order_id(order.cloid.as_deref())?,
        reduce_only: Some(order.reduce_only),
        order_id: order_id(order.oid)?.to_string(),
        symbol: coin.to_owned(),
        exchange: exchange.into(),
        side,
        order_type,
        status: if fill_evidence.is_some() {
            OrderStatus::PartiallyFilled
        } else {
            OrderStatus::Open
        },
        quantity: orig,
        price: required_positive_number("openOrder.limitPx", &order.limit_px)?,
        filled_quantity: filled,
        filled_price: fill_evidence
            .as_ref()
            .map_or(0.0, |evidence| evidence.average_price),
        fees: fill_evidence.map_or(0.0, |evidence| evidence.fee),
        created_at,
    })))
}

fn open_order_filled_quantity(order: &OpenOrderItem) -> ExchangeResult<f64> {
    let orig = required_positive_number("openOrder.origSz", &order.orig_sz)?;
    let remaining = required_non_negative_number("openOrder.sz", &order.sz)?;
    if remaining > orig {
        return Err(ExchangeError::Parse(format!(
            "hyperliquid open order remaining greater than original: remaining={remaining} orig={orig}"
        )));
    }
    Ok(orig - remaining)
}

fn fill_evidence(
    order: &OpenOrderItem,
    fills: &[UserFillItem],
    expected_quantity: f64,
) -> ExchangeResult<Option<FillEvidence>> {
    let mut quantity = 0.0;
    let mut notional = 0.0;
    let mut fee = 0.0;
    for fill in fills.iter().filter(|fill| fill.oid == order.oid) {
        if fill.coin != order.coin {
            return Err(ExchangeError::Parse(format!(
                "hyperliquid userFills coin mismatch for oid {}: order={} fill={}",
                order.oid, order.coin, fill.coin
            )));
        }
        let fill_quantity = required_positive_number("userFills.sz", &fill.sz)?;
        let fill_price = required_positive_number("userFills.px", &fill.px)?;
        quantity += fill_quantity;
        notional += fill_quantity * fill_price;
        fee += required_number("userFills.fee", &fill.fee)?;
    }
    if quantity == 0.0 {
        return Ok(None);
    }
    let tolerance = expected_quantity.abs().max(1.0) * 1e-9;
    if (quantity - expected_quantity).abs() > tolerance {
        return Err(ExchangeError::Parse(format!(
            "hyperliquid userFills quantity mismatch for oid {}: expected={expected_quantity} actual={quantity}",
            order.oid
        )));
    }
    Ok(Some(FillEvidence {
        average_price: notional / quantity,
        fee,
    }))
}

fn parse_order_status(
    entry: &OrderStatusEntry,
    fills: &[UserFillItem],
    exchange: &str,
) -> ExchangeResult<OrderInfo> {
    let status = hyperliquid_order_status(&entry.status)?;
    let _status_timestamp = timestamp_ms("orderStatus.statusTimestamp", entry.status_timestamp)?;
    // 按单查询路径保持严格：部分成交缺 fill 证据必须报错（修复循环需要明确
    // 错误而非伪造成交经济学），只有列表路径允许按行跳过。
    let mut order = match parse_open_order(&entry.order, fills, exchange)? {
        OpenOrderRow::Ready(order) => *order,
        OpenOrderRow::NeedsFillEvidence => {
            return Err(ExchangeError::Parse(
                "hyperliquid order has fills but frontendOpenOrders/orderStatus does not provide actual fill price and fee; require userFills evidence".into(),
            ));
        }
    };
    if matches!(status, OrderStatus::Filled) && order.filled_quantity == 0.0 {
        return Err(ExchangeError::Parse(
            "hyperliquid orderStatus marked filled without filled quantity evidence".into(),
        ));
    }
    order.status = if matches!(status, OrderStatus::Open) && order.filled_quantity > 0.0 {
        OrderStatus::PartiallyFilled
    } else {
        status
    };
    Ok(order)
}

fn classify_hyperliquid_order_type(order: &OpenOrderItem) -> ExchangeResult<OrderType> {
    let Some(order_type) = order.order_type.as_deref() else {
        return Err(ExchangeError::Parse(
            "hyperliquid open order missing orderType".into(),
        ));
    };
    let normalized = order_type.to_ascii_lowercase();
    let order_type = match (normalized.as_str(), order.is_trigger) {
        ("limit", _) => OrderType::Limit,
        ("market", _) => OrderType::Market,
        ("stop limit", true) => OrderType::Limit,
        ("stop market", true) => OrderType::Market,
        ("stop limit" | "stop market", _) => {
            return Err(ExchangeError::Parse(format!(
                "hyperliquid open order {order_type} requires isTrigger=true"
            )));
        }
        _ => {
            return Err(ExchangeError::Parse(format!(
                "hyperliquid open order unsupported orderType: {order_type}"
            )));
        }
    };
    match order.tif.as_deref() {
        Some("Alo") => Ok(OrderType::PostOnly),
        Some("Ioc" | "Gtc") | None => Ok(order_type),
        Some("FrontendMarket") if matches!(order_type, OrderType::Market) => Ok(order_type),
        Some(tif) => Err(ExchangeError::Parse(format!(
            "hyperliquid open order unsupported tif: {tif}"
        ))),
    }
}

fn order_side(side: &str) -> ExchangeResult<OrderSide> {
    if side.eq_ignore_ascii_case("A") || side.eq_ignore_ascii_case("sell") {
        Ok(OrderSide::Sell)
    } else if side.eq_ignore_ascii_case("B") || side.eq_ignore_ascii_case("buy") {
        Ok(OrderSide::Buy)
    } else {
        Err(ExchangeError::Parse(format!(
            "hyperliquid open order unsupported side: {side}"
        )))
    }
}

fn hyperliquid_order_status(status: &str) -> ExchangeResult<OrderStatus> {
    let normalized = status.to_ascii_lowercase();
    match normalized.as_str() {
        "filled" => Ok(OrderStatus::Filled),
        "canceled"
        | "cancelled"
        | "margincanceled"
        | "vaultwithdrawalcanceled"
        | "openinterestcapcanceled"
        | "selftradecanceled"
        | "reduceonlycanceled"
        | "siblingfilledcanceled"
        | "delistedcanceled"
        | "liquidatedcanceled"
        | "scheduledcancel" => Ok(OrderStatus::Canceled),
        "rejected"
        | "tickrejected"
        | "mintradentlrejected"
        | "perpmarginrejected"
        | "reduceonlyrejected"
        | "badalopxrejected"
        | "ioccancelrejected"
        | "badtriggerpxrejected"
        | "marketordernoliquidityrejected"
        | "positionincreaseatopeninterestcaprejected"
        | "positionflipatopeninterestcaprejected"
        | "tooaggressiveatopeninterestcaprejected"
        | "openinterestincreaserejected"
        | "insufficientspotbalancerejected"
        | "oraclerejected"
        | "perpmaxpositionrejected" => Ok(OrderStatus::Rejected),
        "triggered" | "open" => Ok(OrderStatus::Open),
        _ => Err(ExchangeError::Parse(format!(
            "hyperliquid orderStatus unsupported order status: {status}"
        ))),
    }
}

fn hyperliquid_position_mode(position_type: &str) -> ExchangeResult<&'static str> {
    match position_type {
        "oneWay" => Ok("one_way"),
        other => Err(ExchangeError::Parse(format!(
            "hyperliquid unsupported position type: {other}"
        ))),
    }
}

fn normalize_symbol(symbol: &str) -> String {
    strip_common_suffixes(&clean_hyperliquid_symbol(symbol))
}

fn positive_optional(field: &str, value: Option<&str>) -> ExchangeResult<Option<f64>> {
    required_optional_number(field, value)?.map_or(Ok(None), |value| {
        (value > 0.0)
            .then_some(value)
            .ok_or_else(|| {
                ExchangeError::Parse(format!(
                    "hyperliquid private expected positive field {field}: {value}"
                ))
            })
            .map(Some)
    })
}

fn required_optional_number(field: &str, value: Option<&str>) -> ExchangeResult<Option<f64>> {
    value.map(|text| required_number(field, text)).transpose()
}

fn required_text<'a>(field: &str, value: &'a str) -> ExchangeResult<&'a str> {
    if value.trim().is_empty() {
        Err(missing_field(field))
    } else {
        Ok(value)
    }
}

fn required_number(field: &str, value: &str) -> ExchangeResult<f64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(missing_field(field));
    }
    let parsed = trimmed.parse::<f64>().map_err(|error| {
        ExchangeError::Parse(format!(
            "hyperliquid private numeric field {field}: {error}; value={trimmed}"
        ))
    })?;
    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(ExchangeError::Parse(format!(
            "hyperliquid private non-finite numeric field {field}: {trimmed}"
        )))
    }
}

fn required_positive_number(field: &str, value: &str) -> ExchangeResult<f64> {
    let parsed = required_number(field, value)?;
    required_positive_f64(field, parsed)
}

fn required_non_negative_number(field: &str, value: &str) -> ExchangeResult<f64> {
    let parsed = required_number(field, value)?;
    if parsed >= 0.0 {
        Ok(parsed)
    } else {
        Err(ExchangeError::Parse(format!(
            "hyperliquid private expected non-negative field {field}: {parsed}"
        )))
    }
}

fn required_positive_f64(field: &str, value: f64) -> ExchangeResult<f64> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(ExchangeError::Parse(format!(
            "hyperliquid private expected positive field {field}: {value}"
        )))
    }
}

fn parse_margin_summary(
    summary: &MarginSummary,
    prefix: &str,
) -> ExchangeResult<HyperliquidMarginEvidence> {
    Ok(HyperliquidMarginEvidence {
        account_value: required_non_negative_number(
            &format!("{prefix}.accountValue"),
            &summary.account_value,
        )?,
        total_ntl_pos: required_non_negative_number(
            &format!("{prefix}.totalNtlPos"),
            &summary.total_ntl_pos,
        )?,
        total_raw_usd: required_non_negative_number(
            &format!("{prefix}.totalRawUsd"),
            &summary.total_raw_usd,
        )?,
        total_margin_used: required_non_negative_number(
            &format!("{prefix}.totalMarginUsed"),
            &summary.total_margin_used,
        )?,
    })
}

fn account_ratio(value: f64, equity: f64) -> f64 {
    if equity > f64::EPSILON {
        value / equity
    } else {
        0.0
    }
}

fn parse_client_order_id(value: Option<&str>) -> ExchangeResult<Option<String>> {
    match value {
        None => Ok(None),
        Some(value) => client_order_id_from_str(value).map(Some).ok_or_else(|| {
            ExchangeError::Parse(format!("hyperliquid invalid client order id: {value}"))
        }),
    }
}

fn order_id(value: i64) -> ExchangeResult<i64> {
    if value > 0 {
        Ok(value)
    } else {
        Err(ExchangeError::Parse(format!(
            "hyperliquid private invalid order id: {value}"
        )))
    }
}

fn mark_price(coin: &str, mark_map: &HashMap<String, f64>) -> ExchangeResult<f64> {
    mark_map
        .get(coin)
        .or_else(|| mark_map.get(&normalize_symbol(coin)))
        .copied()
        .filter(|price| price.is_finite() && *price > 0.0)
        .ok_or_else(|| {
            ExchangeError::Parse(format!(
                "hyperliquid position missing positive mark price for {coin}"
            ))
        })
}

fn timestamp_ms(field: &str, value: i64) -> ExchangeResult<chrono::DateTime<chrono::Utc>> {
    if value <= 0 {
        return Err(ExchangeError::Parse(format!(
            "hyperliquid private invalid timestamp field {field}: {value}"
        )));
    }
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(value).ok_or_else(|| {
        ExchangeError::Parse(format!(
            "hyperliquid private invalid timestamp field {field}: {value}"
        ))
    })
}

fn missing_field(field: &str) -> ExchangeError {
    ExchangeError::Parse(format!(
        "hyperliquid private missing required field {field}"
    ))
}

#[cfg(test)]
#[path = "hyperliquid_private_data_tests.rs"]
mod tests;
