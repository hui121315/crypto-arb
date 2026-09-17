//! KuCoin private read response parsing.
//!
//! Official KuCoin docs checked before moving these DTOs:
//! - GET /api/v1/account-overview
//!   <https://www.kucoin.com/docs-new/rest/account-info/account-funding/get-account-futures>
//! - GET /api/ua/v1/unified/account/overview
//! - GET /api/ua/v1/unified/account/balance
//! - GET /api/v1/positions
//! - GET /api/v1/orders
//! - GET /api/v1/fills
//!   <https://www.kucoin.com/docs-new/rest/futures-trading/orders/get-trade-history>
//! - GET /api/v1/trade-fees
//!   <https://www.kucoin.com/docs-new/rest/account-info/trade-fee/get-actual-fee-futures>

use crate::adapter::client_order_id_from_str;
use crate::adapters::kucoin_market_data::kucoin_to_normalized;
use crate::error::{ExchangeError, ExchangeResult};
use crate::live::{venue_balance_rows, VenueAccountRead};
use serde::Deserialize;
use serde_json::Value;
use shared_types::{
    AccountEquityScope, BalanceInfo, OrderInfo, OrderIntent, OrderSide, OrderStatus, OrderType,
    PositionInfo, VenueAccountSummary,
};
use std::collections::HashMap;

const NAME: &str = "kucoin";
pub(super) const KUCOIN_FILLS_DOC_URL: &str =
    "https://www.kucoin.com/docs-new/rest/futures-trading/orders/get-trade-history";
pub(super) const KUCOIN_FEE_RATE_DOC_URL: &str =
    "https://www.kucoin.com/docs-new/rest/account-info/trade-fee/get-actual-fee-futures";
type RawNumber = Option<Value>;

#[derive(Debug, Deserialize)]
pub(super) struct AccountOverview {
    #[serde(default, rename = "accountEquity")]
    account_equity: RawNumber,
    #[serde(default, rename = "unrealisedPNL")]
    unrealised_pnl: RawNumber,
    #[serde(default, rename = "availableBalance")]
    available_balance: RawNumber,
    #[serde(default, rename = "marginBalance")]
    margin_balance: RawNumber,
    #[serde(default, rename = "availableMargin")]
    available_margin: RawNumber,
    #[serde(default, rename = "riskRatio")]
    risk_ratio: RawNumber,
    #[serde(default, rename = "positionMargin")]
    position_margin: RawNumber,
    #[serde(default, rename = "orderMargin")]
    order_margin: RawNumber,
    #[serde(default, rename = "frozenFunds")]
    frozen_funds: RawNumber,
    #[serde(default, rename = "maxWithdrawAmount")]
    max_withdraw_amount: RawNumber,
    #[serde(default)]
    currency: String,
}

#[cfg(test)]
#[derive(Debug, Deserialize)]
pub(super) struct UtaAccountOverview {
    #[serde(default, rename = "accountType")]
    account_type: String,
    #[serde(default, rename = "riskRatio")]
    risk_ratio: RawNumber,
    #[serde(default)]
    equity: RawNumber,
    #[serde(default)]
    liability: RawNumber,
    #[serde(default, rename = "availableMargin")]
    available_margin: RawNumber,
    #[serde(default, rename = "adjustedEquity")]
    adjusted_equity: RawNumber,
    #[serde(default, rename = "im")]
    initial_margin: RawNumber,
    #[serde(default, rename = "mm")]
    maintenance_margin: RawNumber,
}

#[cfg(test)]
#[derive(Debug, PartialEq)]
pub(super) struct UtaAccountOverviewInfo {
    account_type: String,
    risk_ratio: f64,
    equity: f64,
    liability: f64,
    available_margin: f64,
    adjusted_equity: f64,
    initial_margin: f64,
    maintenance_margin: f64,
}

#[cfg(test)]
#[derive(Debug, Deserialize)]
pub(super) struct UtaCurrencyAssets {
    #[serde(default, rename = "accountType")]
    account_type: String,
    #[serde(default)]
    accounts: Vec<UtaCurrencyAccount>,
}

#[cfg(test)]
#[derive(Debug, Deserialize)]
pub(super) struct UtaCurrencyAccount {
    #[serde(default)]
    currencies: Vec<UtaCurrencyAsset>,
}

#[cfg(test)]
#[derive(Debug, Deserialize)]
pub(super) struct UtaCurrencyAsset {
    #[serde(default)]
    currency: String,
    #[serde(default)]
    equity: RawNumber,
    #[serde(default)]
    hold: RawNumber,
    #[serde(default)]
    balance: RawNumber,
    #[serde(default)]
    available: RawNumber,
    #[serde(default)]
    liability: RawNumber,
}

#[cfg(test)]
#[derive(Debug, PartialEq)]
pub(super) struct UtaCurrencyAssetInfo {
    currency: String,
    equity: f64,
    hold: f64,
    balance: f64,
    available_wallet_balance: f64,
    liability: f64,
}

#[derive(Debug, Deserialize)]
pub(super) struct PositionRow {
    symbol: String,
    #[serde(default, rename = "currentQty")]
    current_qty: RawNumber,
    #[serde(default, rename = "avgEntryPrice")]
    avg_entry_price: RawNumber,
    #[serde(default, rename = "markPrice")]
    mark_price: RawNumber,
    #[serde(default, rename = "unrealisedPnl")]
    unrealised_pnl: RawNumber,
    #[serde(default)]
    leverage: RawNumber,
    #[serde(default, rename = "marginMode")]
    margin_mode: Option<String>,
    #[serde(default, rename = "liquidationPrice")]
    liquidation_price: RawNumber,
    #[serde(default, rename = "posMargin")]
    pos_margin: RawNumber,
    #[serde(default, rename = "maintMarginReq")]
    maint_margin_req: RawNumber,
    #[serde(default, rename = "isOpen")]
    is_open: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct NativePositionInfo {
    pub(super) native_symbol: String,
    pub(super) position: PositionInfo,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenOrderItem {
    pub(super) id: String,
    symbol: String,
    side: String,
    #[serde(rename = "type")]
    order_type: String,
    status: String,
    price: String,
    size: f64,
    #[serde(rename = "filledSize")]
    pub(super) filled_size: f64,
    #[serde(rename = "filledValue")]
    filled_value: String,
    #[serde(rename = "cancelExist")]
    cancel_exist: bool,
    #[serde(rename = "createdAt")]
    created_at: i64,
    #[serde(rename = "postOnly")]
    post_only: bool,
    #[serde(default, rename = "timeInForce")]
    time_in_force: Option<String>,
    #[serde(rename = "clientOid")]
    pub(super) client_oid: String,
    #[serde(rename = "reduceOnly")]
    reduce_only: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct PaginatedOrders {
    items: Vec<OpenOrderItem>,
}

#[derive(Debug, Deserialize)]
pub(super) struct FillPage {
    items: Vec<FillRow>,
}

#[derive(Debug, Deserialize)]
pub(super) struct FillRow {
    symbol: String,
    #[serde(rename = "tradeId")]
    trade_id: String,
    #[serde(rename = "orderId")]
    order_id: String,
    side: String,
    liquidity: String,
    price: RawNumber,
    size: RawNumber,
    value: RawNumber,
    fee: RawNumber,
    #[serde(rename = "feeRate")]
    fee_rate: RawNumber,
    #[serde(rename = "feeCurrency")]
    fee_currency: String,
    #[serde(rename = "settleCurrency")]
    settle_currency: String,
    #[serde(rename = "createdAt")]
    created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KucoinLiquidity {
    Maker,
    Taker,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct FillEvidence {
    pub(super) symbol: String,
    pub(super) trade_id: String,
    pub(super) order_id: String,
    pub(super) side: OrderSide,
    pub(super) liquidity: KucoinLiquidity,
    pub(super) price: f64,
    pub(super) size: f64,
    pub(super) value: f64,
    pub(super) fee: f64,
    pub(super) fee_rate: f64,
    pub(super) fee_currency: String,
    pub(super) settle_currency: String,
    pub(super) occurred_at_ms: i64,
    pub(super) source_url: &'static str,
}

#[derive(Debug, Deserialize)]
pub(super) struct FeeRateRow {
    symbol: String,
    #[serde(rename = "makerFeeRate")]
    maker_fee_rate: RawNumber,
    #[serde(rename = "takerFeeRate")]
    taker_fee_rate: RawNumber,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct FeeRateEvidence {
    pub(super) symbol: String,
    pub(super) maker_fee_rate: f64,
    pub(super) taker_fee_rate: f64,
    pub(super) fetched_at_ms: i64,
    pub(super) source_url: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub(super) enum KucoinPositionMode {
    OneWay,
    Hedge,
}

#[derive(Debug, Deserialize)]
pub(super) struct PositionModeRow {
    #[serde(rename = "positionMode")]
    position_mode: i64,
}

impl KucoinPositionMode {
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

    pub(super) fn position_side_for_intent(
        self,
        intent: &OrderIntent,
    ) -> ExchangeResult<&'static str> {
        match self {
            Self::OneWay => Ok("BOTH"),
            Self::Hedge if intent.reduce_only => Err(kucoin_validation(
                "kucoin hedge-mode reduceOnly order requires explicit positionSide evidence; OrderIntent currently only carries side".to_owned(),
            )),
            Self::Hedge => Ok(match intent.side {
                OrderSide::Buy => "LONG",
                OrderSide::Sell => "SHORT",
            }),
        }
    }
}

pub(super) fn parse_position_mode(row: &PositionModeRow) -> ExchangeResult<KucoinPositionMode> {
    KucoinPositionMode::from_code(row.position_mode).ok_or_else(|| {
        kucoin_validation(format!(
            "kucoin unknown positionMode from /api/v2/position/getPositionMode: {}",
            row.position_mode
        ))
    })
}

pub(super) fn parse_balance_response(
    account: &AccountOverview,
    requested_currency: &str,
) -> ExchangeResult<HashMap<String, BalanceInfo>> {
    let key = kucoin_required_text("account-overview", "currency", &account.currency)?;
    let requested =
        kucoin_required_text("account-overview request", "currency", requested_currency)?;
    if !key.eq_ignore_ascii_case(requested) {
        return Err(kucoin_parse(format!(
            "kucoin account-overview currency mismatch: requested={requested} response={key}"
        )));
    }
    let key = key.to_ascii_uppercase();
    let scope = format!("account-overview {key}");
    let total = kucoin_required_number(&scope, "accountEquity", &account.account_equity)?;
    let available =
        kucoin_required_non_negative_number(&scope, "availableMargin", &account.available_margin)?;
    kucoin_required_non_negative_number(&scope, "availableBalance", &account.available_balance)?;
    kucoin_required_non_negative_number(&scope, "marginBalance", &account.margin_balance)?;
    kucoin_required_non_negative_number(&scope, "riskRatio", &account.risk_ratio)?;
    let unrealized_pnl = kucoin_required_number(&scope, "unrealisedPNL", &account.unrealised_pnl)?;
    let position_margin =
        kucoin_required_non_negative_number(&scope, "positionMargin", &account.position_margin)?;
    let order_margin =
        kucoin_required_non_negative_number(&scope, "orderMargin", &account.order_margin)?;
    let frozen_funds =
        kucoin_required_non_negative_number(&scope, "frozenFunds", &account.frozen_funds)?;
    let mut out = HashMap::new();
    out.insert(
        key.clone(),
        BalanceInfo {
            currency: key,
            total,
            available,
            frozen: position_margin + order_margin + frozen_funds,
            unrealized_pnl,
        },
    );
    Ok(out)
}

pub(super) fn parse_account_read(
    account: &AccountOverview,
    requested_currency: &str,
    observed_at_ms: i64,
) -> ExchangeResult<VenueAccountRead> {
    let balances = parse_balance_response(account, requested_currency)?;
    let summary = parse_account_summary(account, requested_currency, observed_at_ms)?;
    Ok(VenueAccountRead {
        balances: venue_balance_rows(NAME, balances),
        summaries: vec![summary],
        asset_valuations: Vec::new(),
        issues: Vec::new(),
    })
}

fn parse_account_summary(
    account: &AccountOverview,
    requested_currency: &str,
    observed_at_ms: i64,
) -> ExchangeResult<VenueAccountSummary> {
    let currency = kucoin_required_text("account-overview", "currency", &account.currency)?;
    if !currency.eq_ignore_ascii_case(requested_currency) {
        return Err(kucoin_parse(format!(
            "kucoin account-overview currency mismatch: requested={requested_currency} response={currency}"
        )));
    }
    let scope = format!("account-overview {}", currency.to_ascii_uppercase());
    let total_equity_usd =
        kucoin_required_non_negative_number(&scope, "accountEquity", &account.account_equity)?;
    let total_available_balance_usd =
        kucoin_required_non_negative_number(&scope, "availableMargin", &account.available_margin)?;
    let position_margin =
        kucoin_required_non_negative_number(&scope, "positionMargin", &account.position_margin)?;
    let order_margin =
        kucoin_required_non_negative_number(&scope, "orderMargin", &account.order_margin)?;
    let total_initial_margin_usd = position_margin + order_margin;
    let account_mm_rate =
        kucoin_required_non_negative_number(&scope, "riskRatio", &account.risk_ratio)?;
    let total_maintenance_margin_usd = total_equity_usd * account_mm_rate;
    Ok(VenueAccountSummary {
        venue: NAME.to_owned(),
        account_type: "classic_futures".to_owned(),
        equity_scope: AccountEquityScope::Perpetuals,
        total_equity_usd,
        total_available_balance_usd,
        withdrawable_balance_usd: kucoin_optional_non_negative_number(
            &scope,
            "maxWithdrawAmount",
            &account.max_withdraw_amount,
        )?,
        total_initial_margin_usd,
        total_maintenance_margin_usd,
        account_im_rate: account_ratio(total_initial_margin_usd, total_equity_usd),
        account_mm_rate,
        source: "kucoin.GET /api/v1/account-overview".to_owned(),
        observed_at_ms,
        freshness_ms: Some(0),
        problem: None,
    })
}

fn account_ratio(value: f64, equity: f64) -> f64 {
    if equity > 0.0 {
        value / equity
    } else {
        0.0
    }
}

#[cfg(test)]
pub(super) fn parse_uta_account_overview(
    account: &UtaAccountOverview,
) -> ExchangeResult<UtaAccountOverviewInfo> {
    let scope = "uta account-overview";
    let account_type = kucoin_required_text(scope, "accountType", &account.account_type)?;
    if !account_type.eq_ignore_ascii_case("UNIFIED") {
        return Err(kucoin_parse(format!(
            "kucoin {scope} expected UNIFIED accountType, got {account_type:?}"
        )));
    }
    Ok(UtaAccountOverviewInfo {
        account_type: account_type.to_owned(),
        risk_ratio: kucoin_required_non_negative_number(scope, "riskRatio", &account.risk_ratio)?,
        equity: kucoin_required_non_negative_number(scope, "equity", &account.equity)?,
        liability: kucoin_required_non_negative_number(scope, "liability", &account.liability)?,
        available_margin: kucoin_required_non_negative_number(
            scope,
            "availableMargin",
            &account.available_margin,
        )?,
        adjusted_equity: kucoin_required_non_negative_number(
            scope,
            "adjustedEquity",
            &account.adjusted_equity,
        )?,
        initial_margin: kucoin_required_non_negative_number(scope, "im", &account.initial_margin)?,
        maintenance_margin: kucoin_required_non_negative_number(
            scope,
            "mm",
            &account.maintenance_margin,
        )?,
    })
}

#[cfg(test)]
pub(super) fn parse_uta_currency_assets(
    payload: &UtaCurrencyAssets,
) -> ExchangeResult<Vec<UtaCurrencyAssetInfo>> {
    let scope = "uta currency-assets";
    let account_type = kucoin_required_text(scope, "accountType", &payload.account_type)?;
    if !account_type.eq_ignore_ascii_case("UNIFIED") {
        return Err(kucoin_parse(format!(
            "kucoin {scope} expected UNIFIED accountType, got {account_type:?}"
        )));
    }

    let mut out = Vec::new();
    for account in &payload.accounts {
        for asset in &account.currencies {
            out.push(parse_uta_currency_asset(asset)?);
        }
    }
    if out.is_empty() {
        return Err(kucoin_parse(format!("kucoin {scope} missing currencies")));
    }
    Ok(out)
}

#[cfg(test)]
fn parse_uta_currency_asset(asset: &UtaCurrencyAsset) -> ExchangeResult<UtaCurrencyAssetInfo> {
    let currency = kucoin_required_text("uta currency-assets", "currency", &asset.currency)?;
    let scope = format!("uta currency-assets {currency}");
    Ok(UtaCurrencyAssetInfo {
        currency: currency.to_owned(),
        equity: kucoin_required_non_negative_number(&scope, "equity", &asset.equity)?,
        hold: kucoin_required_non_negative_number(&scope, "hold", &asset.hold)?,
        balance: kucoin_required_non_negative_number(&scope, "balance", &asset.balance)?,
        available_wallet_balance: kucoin_required_non_negative_number(
            &scope,
            "available",
            &asset.available,
        )?,
        liability: kucoin_required_non_negative_number(&scope, "liability", &asset.liability)?,
    })
}

#[cfg(test)]
pub(super) fn parse_positions(
    rows: &[PositionRow],
    target: Option<&str>,
) -> ExchangeResult<Vec<PositionInfo>> {
    Ok(parse_positions_with_native(rows, target)?
        .into_iter()
        .map(|row| row.position)
        .collect())
}

pub(super) fn parse_positions_with_native(
    rows: &[PositionRow],
    target: Option<&str>,
) -> ExchangeResult<Vec<NativePositionInfo>> {
    let mut out = Vec::new();
    for row in rows.iter().filter(|row| match target {
        Some(want) => row.symbol == want,
        None => true,
    }) {
        if let Some(position) = parse_position(row)? {
            out.push(NativePositionInfo {
                native_symbol: kucoin_required_text("position", "symbol", &row.symbol)?
                    .to_ascii_uppercase(),
                position,
            });
        }
    }
    Ok(out)
}

pub(super) fn parse_open_orders(page: &PaginatedOrders) -> ExchangeResult<Vec<OrderInfo>> {
    page.items.iter().map(parse_open_order).collect()
}

pub(super) fn parse_open_order(order: &OpenOrderItem) -> ExchangeResult<OrderInfo> {
    parse_order_with_fills(order, &[])
}

pub(super) fn parse_order_with_fills(
    order: &OpenOrderItem,
    fills: &[FillEvidence],
) -> ExchangeResult<OrderInfo> {
    let side = kucoin_order_side(order)?;
    let order_type = kucoin_order_type(order)?;
    let status = kucoin_order_status(order)?;
    let quantity = kucoin_positive_value("size", order.size, &order.id)?;
    let filled_quantity = kucoin_non_negative_value("filledSize", order.filled_size, &order.id)?;
    if filled_quantity > quantity {
        return Err(kucoin_parse(format!(
            "kucoin order {} has filledSize {filled_quantity} above size {quantity}",
            order.id
        )));
    }
    let price = kucoin_order_price(order)?;
    let (filled_price, fees) = kucoin_order_execution(order, filled_quantity, fills)?;
    let created_at = kucoin_order_created_at(order)?;
    let venue_time_in_force = kucoin_time_in_force(order.time_in_force.as_deref(), &order.id)?;
    Ok(OrderInfo {
        execution_style: None,
        venue_time_in_force,
        client_order_id: client_order_id_from_str(&order.client_oid),
        reduce_only: Some(order.reduce_only),
        order_id: order.id.clone(),
        symbol: kucoin_to_normalized(&order.symbol),
        exchange: NAME.into(),
        side,
        order_type,
        status,
        quantity,
        price,
        filled_quantity,
        filled_price,
        fees,
        created_at,
    })
}

fn kucoin_time_in_force(raw: Option<&str>, order_id: &str) -> ExchangeResult<Option<String>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    match raw {
        "GTC" | "IOC" | "FOK" => Ok(Some(raw.to_owned())),
        _ => Err(kucoin_parse(format!(
            "kucoin order {order_id} has unsupported timeInForce {raw}"
        ))),
    }
}

pub(super) fn parse_fill_page(
    page: &FillPage,
    requested_order_id: &str,
) -> ExchangeResult<Vec<FillEvidence>> {
    let requested_order_id = kucoin_required_text("fills request", "orderId", requested_order_id)?;
    page.items
        .iter()
        .map(|row| parse_fill(row, requested_order_id))
        .collect()
}

pub(super) fn parse_fee_rate(
    row: &FeeRateRow,
    requested_symbol: &str,
    fetched_at_ms: i64,
) -> ExchangeResult<FeeRateEvidence> {
    let symbol = kucoin_required_text("trade-fees", "symbol", &row.symbol)?.to_ascii_uppercase();
    let requested = kucoin_required_text("trade-fees request", "symbol", requested_symbol)?
        .to_ascii_uppercase();
    if symbol != requested {
        return Err(kucoin_parse(format!(
            "kucoin trade-fees symbol mismatch: requested={requested} response={symbol}"
        )));
    }
    if fetched_at_ms <= 0 {
        return Err(kucoin_parse(
            "kucoin trade-fees requires positive fetched_at_ms".to_owned(),
        ));
    }
    Ok(FeeRateEvidence {
        symbol,
        maker_fee_rate: kucoin_rate("trade-fees", "makerFeeRate", &row.maker_fee_rate)?,
        taker_fee_rate: kucoin_rate("trade-fees", "takerFeeRate", &row.taker_fee_rate)?,
        fetched_at_ms,
        source_url: KUCOIN_FEE_RATE_DOC_URL,
    })
}

fn kucoin_order_side(order: &OpenOrderItem) -> ExchangeResult<OrderSide> {
    match order.side.to_ascii_lowercase().as_str() {
        "buy" => Ok(OrderSide::Buy),
        "sell" => Ok(OrderSide::Sell),
        side => Err(kucoin_parse(format!(
            "kucoin order {} has unknown side {side}",
            order.id
        ))),
    }
}

fn kucoin_order_type(order: &OpenOrderItem) -> ExchangeResult<OrderType> {
    let order_type = match order.order_type.to_ascii_lowercase().as_str() {
        "limit" => Ok(OrderType::Limit),
        "market" => Ok(OrderType::Market),
        kind => Err(kucoin_parse(format!(
            "kucoin order {} has unknown type {kind}",
            order.id
        ))),
    }?;
    if order.post_only {
        if matches!(order_type, OrderType::Limit) {
            Ok(OrderType::PostOnly)
        } else {
            Err(kucoin_parse(format!(
                "kucoin order {} has postOnly market type",
                order.id
            )))
        }
    } else {
        Ok(order_type)
    }
}

fn kucoin_order_status(order: &OpenOrderItem) -> ExchangeResult<OrderStatus> {
    match order.status.to_ascii_lowercase().as_str() {
        "open" if order.filled_size > 0.0 => Ok(OrderStatus::PartiallyFilled),
        "open" => Ok(OrderStatus::Open),
        "match" => Ok(OrderStatus::PartiallyFilled),
        "done" if order.cancel_exist => Ok(OrderStatus::Canceled),
        "done" | "filled" => Ok(OrderStatus::Filled),
        "canceled" => Ok(OrderStatus::Canceled),
        "rejected" => Ok(OrderStatus::Rejected),
        status => Err(kucoin_parse(format!(
            "kucoin order {} has unknown status {status}",
            order.id
        ))),
    }
}

fn kucoin_order_price(order: &OpenOrderItem) -> ExchangeResult<f64> {
    kucoin_non_negative_decimal("price", &order.price, &order.id)
}

fn kucoin_order_execution(
    order: &OpenOrderItem,
    filled_size: f64,
    fills: &[FillEvidence],
) -> ExchangeResult<(f64, f64)> {
    let filled_value = kucoin_non_negative_decimal("filledValue", &order.filled_value, &order.id)?;
    if filled_size == 0.0 {
        if filled_value != 0.0 {
            return Err(kucoin_parse(format!(
                "kucoin order {} has zero filledSize with filledValue {filled_value}",
                order.id
            )));
        }
        if fills.is_empty() {
            return Ok((0.0, 0.0));
        }
        return Err(kucoin_parse(format!(
            "kucoin order {} has fill evidence but filledSize is zero",
            order.id
        )));
    }
    if fills.is_empty() {
        return Err(kucoin_parse(format!(
            "kucoin order {} has filledSize {filled_size} but no /api/v1/fills fee evidence",
            order.id
        )));
    }
    let mut fill_size = 0.0;
    let mut weighted_price = 0.0;
    let mut fees = 0.0;
    for fill in fills {
        if fill.order_id != order.id {
            return Err(kucoin_parse(format!(
                "kucoin order {} received fill for order {}",
                order.id, fill.order_id
            )));
        }
        fill_size += fill.size;
        weighted_price += fill.price * fill.size;
        fees += fill.fee;
    }
    if (fill_size - filled_size).abs() > f64::EPSILON * filled_size.abs().max(1.0) * 8.0 {
        return Err(kucoin_parse(format!(
            "kucoin order {} fill size mismatch: order={filled_size} fills={fill_size}",
            order.id
        )));
    }
    Ok((weighted_price / fill_size, fees))
}

fn parse_fill(row: &FillRow, requested_order_id: &str) -> ExchangeResult<FillEvidence> {
    let trade_id = kucoin_required_text("fill", "tradeId", &row.trade_id)?;
    let scope = format!("fill {trade_id}");
    let order_id = kucoin_required_text(&scope, "orderId", &row.order_id)?;
    if order_id != requested_order_id {
        return Err(kucoin_parse(format!(
            "kucoin {scope} order mismatch: requested={requested_order_id} response={order_id}"
        )));
    }
    let fee_currency =
        kucoin_required_text(&scope, "feeCurrency", &row.fee_currency)?.to_ascii_uppercase();
    let settle_currency =
        kucoin_required_text(&scope, "settleCurrency", &row.settle_currency)?.to_ascii_uppercase();
    if fee_currency != settle_currency {
        return Err(kucoin_parse(format!(
            "kucoin {scope} fee currency mismatch: fee={fee_currency} settle={settle_currency}"
        )));
    }
    let occurred_at_ms = row.created_at;
    if occurred_at_ms <= 0
        || chrono::DateTime::<chrono::Utc>::from_timestamp_millis(occurred_at_ms).is_none()
    {
        return Err(kucoin_parse(format!(
            "kucoin {scope} invalid createdAt {occurred_at_ms}"
        )));
    }
    Ok(FillEvidence {
        symbol: kucoin_required_text(&scope, "symbol", &row.symbol)?.to_ascii_uppercase(),
        trade_id: trade_id.to_owned(),
        order_id: order_id.to_owned(),
        side: kucoin_side_value(&scope, &row.side)?,
        liquidity: kucoin_liquidity(&scope, &row.liquidity)?,
        price: kucoin_required_positive_number(&scope, "price", &row.price)?,
        size: kucoin_required_positive_number(&scope, "size", &row.size)?,
        value: kucoin_required_positive_number(&scope, "value", &row.value)?,
        fee: kucoin_required_number(&scope, "fee", &row.fee)?,
        fee_rate: kucoin_rate(&scope, "feeRate", &row.fee_rate)?,
        fee_currency,
        settle_currency,
        occurred_at_ms,
        source_url: KUCOIN_FILLS_DOC_URL,
    })
}

fn kucoin_side_value(scope: &str, value: &str) -> ExchangeResult<OrderSide> {
    match value.trim().to_ascii_lowercase().as_str() {
        "buy" => Ok(OrderSide::Buy),
        "sell" => Ok(OrderSide::Sell),
        other => Err(kucoin_parse(format!(
            "kucoin {scope} has unknown side {other}"
        ))),
    }
}

fn kucoin_liquidity(scope: &str, value: &str) -> ExchangeResult<KucoinLiquidity> {
    match value.trim().to_ascii_lowercase().as_str() {
        "maker" => Ok(KucoinLiquidity::Maker),
        "taker" => Ok(KucoinLiquidity::Taker),
        other => Err(kucoin_parse(format!(
            "kucoin {scope} has unknown liquidity {other}"
        ))),
    }
}

fn kucoin_rate(scope: &str, field: &str, value: &RawNumber) -> ExchangeResult<f64> {
    let rate = kucoin_required_number(scope, field, value)?;
    if rate.abs() <= 1.0 {
        Ok(rate)
    } else {
        Err(kucoin_parse(format!(
            "kucoin {scope} has out-of-range {field} {rate}"
        )))
    }
}

fn kucoin_order_created_at(order: &OpenOrderItem) -> ExchangeResult<chrono::DateTime<chrono::Utc>> {
    if order.created_at <= 0 {
        return Err(kucoin_parse(format!(
            "kucoin order {} has invalid createdAt {}",
            order.id, order.created_at
        )));
    }
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(order.created_at).ok_or_else(|| {
        kucoin_parse(format!(
            "kucoin order {} has invalid createdAt {}",
            order.id, order.created_at
        ))
    })
}

fn kucoin_positive_value(field: &str, value: f64, order_id: &str) -> ExchangeResult<f64> {
    if value > 0.0 {
        Ok(value)
    } else {
        Err(kucoin_parse(format!(
            "kucoin order {order_id} has non-positive {field} {value}"
        )))
    }
}

fn kucoin_non_negative_value(field: &str, value: f64, order_id: &str) -> ExchangeResult<f64> {
    if value >= 0.0 {
        Ok(value)
    } else {
        Err(kucoin_parse(format!(
            "kucoin order {order_id} has negative {field} {value}"
        )))
    }
}

fn kucoin_parse_decimal(field: &str, value: &str, order_id: &str) -> ExchangeResult<f64> {
    let parsed = value.parse::<f64>().map_err(|source| {
        kucoin_parse(format!(
            "kucoin order {order_id} has invalid {field} {value:?}: {source}"
        ))
    })?;
    kucoin_finite_number("order", field, parsed)
}

fn kucoin_non_negative_decimal(field: &str, value: &str, order_id: &str) -> ExchangeResult<f64> {
    let parsed = kucoin_parse_decimal(field, value, order_id)?;
    if parsed >= 0.0 {
        Ok(parsed)
    } else {
        Err(kucoin_parse(format!(
            "kucoin order {order_id} has negative {field} {parsed}"
        )))
    }
}

fn kucoin_required_non_negative_number(
    scope: &str,
    field: &str,
    value: &RawNumber,
) -> ExchangeResult<f64> {
    let parsed = kucoin_required_number(scope, field, value)?;
    if parsed >= 0.0 {
        Ok(parsed)
    } else {
        Err(kucoin_parse(format!(
            "kucoin {scope} has negative {field} {parsed}"
        )))
    }
}

fn kucoin_required_positive_number(
    scope: &str,
    field: &str,
    value: &RawNumber,
) -> ExchangeResult<f64> {
    let parsed = kucoin_required_number(scope, field, value)?;
    if parsed > 0.0 {
        Ok(parsed)
    } else {
        Err(kucoin_parse(format!(
            "kucoin {scope} has non-positive {field} {parsed}"
        )))
    }
}

fn kucoin_required_number(scope: &str, field: &str, value: &RawNumber) -> ExchangeResult<f64> {
    let Some(value) = value.as_ref() else {
        return Err(kucoin_parse(format!("kucoin {scope} missing {field}")));
    };
    kucoin_number_value(scope, field, value)
}

fn kucoin_required_text<'a>(scope: &str, field: &str, value: &'a str) -> ExchangeResult<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        Err(kucoin_parse(format!("kucoin {scope} missing {field}")))
    } else {
        Ok(value)
    }
}

fn kucoin_optional_non_negative_number(
    scope: &str,
    field: &str,
    value: &RawNumber,
) -> ExchangeResult<Option<f64>> {
    match value.as_ref() {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) if raw.trim().is_empty() => Ok(None),
        Some(value) => {
            let parsed = kucoin_number_value(scope, field, value)?;
            if parsed < 0.0 {
                Err(kucoin_parse(format!(
                    "kucoin {scope} has negative {field} {parsed}"
                )))
            } else {
                Ok(Some(parsed))
            }
        }
    }
}

fn kucoin_number_value(scope: &str, field: &str, value: &Value) -> ExchangeResult<f64> {
    let parsed = match value {
        Value::Number(number) => number
            .as_f64()
            .ok_or_else(|| kucoin_parse(format!("kucoin {scope} has invalid {field} {value}")))?,
        Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(kucoin_parse(format!("kucoin {scope} has empty {field}")));
            }
            trimmed.parse::<f64>().map_err(|source| {
                kucoin_parse(format!(
                    "kucoin {scope} has invalid {field} {raw:?}: {source}"
                ))
            })?
        }
        _ => {
            return Err(kucoin_parse(format!(
                "kucoin {scope} has invalid {field} {value}"
            )));
        }
    };
    kucoin_finite_number(scope, field, parsed)
}

fn kucoin_finite_number(scope: &str, field: &str, value: f64) -> ExchangeResult<f64> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(kucoin_parse(format!(
            "kucoin {scope} has non-finite {field} {value}"
        )))
    }
}

fn kucoin_parse(message: String) -> ExchangeError {
    ExchangeError::Parse(message)
}

fn kucoin_validation(message: String) -> ExchangeError {
    ExchangeError::Api {
        exchange: NAME.into(),
        code: "validation".into(),
        message,
    }
}

fn parse_position(row: &PositionRow) -> ExchangeResult<Option<PositionInfo>> {
    let scope = format!("position {}", row.symbol);
    let is_open = row
        .is_open
        .ok_or_else(|| kucoin_parse(format!("kucoin {scope} missing isOpen")))?;
    if !is_open {
        return Ok(None);
    }
    let current_qty = kucoin_required_number(&scope, "currentQty", &row.current_qty)?;
    if current_qty == 0.0 {
        return Ok(None);
    }
    let entry_price =
        kucoin_required_positive_number(&scope, "avgEntryPrice", &row.avg_entry_price)?;
    let mark_price = kucoin_required_positive_number(&scope, "markPrice", &row.mark_price)?;
    let unrealized_pnl = kucoin_required_number(&scope, "unrealisedPnl", &row.unrealised_pnl)?;
    let leverage = kucoin_required_positive_number(&scope, "leverage", &row.leverage)?;
    let margin_mode = kucoin_position_margin_mode(row)?;
    let margin = kucoin_required_non_negative_number(&scope, "posMargin", &row.pos_margin)?;
    let maintenance_margin_ratio =
        kucoin_required_non_negative_number(&scope, "maintMarginReq", &row.maint_margin_req)?;
    let liquidation_price =
        kucoin_optional_non_negative_number(&scope, "liquidationPrice", &row.liquidation_price)?
            .filter(|value| *value > 0.0);
    Ok(Some(PositionInfo {
        symbol: kucoin_to_normalized(&row.symbol),
        exchange: NAME.into(),
        side: if current_qty > 0.0 { "long" } else { "short" }.to_owned(),
        quantity: current_qty.abs(),
        entry_price,
        mark_price,
        unrealized_pnl,
        leverage,
        liquidation_price,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin,
        maintenance_margin_ratio,
        position_mode: None,
        margin_mode,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }))
}

fn kucoin_position_margin_mode(row: &PositionRow) -> ExchangeResult<Option<String>> {
    let reported = row
        .margin_mode
        .as_deref()
        .map(str::trim)
        .filter(|mode| !mode.is_empty())
        .map(str::to_ascii_lowercase);
    match reported.as_deref() {
        Some("cross") => Ok(Some("cross".to_owned())),
        Some("isolated") => Ok(Some("isolated".to_owned())),
        Some(other) => Err(kucoin_parse(format!(
            "kucoin position {} has unknown marginMode {other}",
            row.symbol
        ))),
        None => Ok(None),
    }
}

#[cfg(test)]
#[path = "kucoin_private_data_tests.rs"]
mod tests;
