//! KuCoin public market response parsing and symbol conversion.

use common::time::now_ms;
use serde::{Deserialize, Deserializer};
use shared_types::{FundingRateData, MarkIndexInfo, SpotTick, TickerInfo};

const NAME: &str = "kucoin";

#[derive(Debug, Deserialize)]
pub(super) struct ContractActive {
    pub(super) symbol: String,
    #[serde(default, rename = "baseCurrency")]
    pub(super) base_currency: String,
    #[serde(default, rename = "quoteCurrency")]
    pub(super) quote_currency: String,
    #[serde(default, rename = "settleCurrency")]
    pub(super) settle_currency: String,
    #[serde(default)]
    pub(super) multiplier: serde_json::Value,
    #[serde(default, rename = "tickSize")]
    pub(super) tick_size: serde_json::Value,
    #[serde(default, rename = "lotSize")]
    pub(super) lot_size: serde_json::Value,
    #[serde(default, rename = "maxOrderQty")]
    pub(super) max_order_qty: serde_json::Value,
    #[serde(default, rename = "marketMaxOrderQty")]
    pub(super) market_max_order_qty: serde_json::Value,
    #[serde(default, rename = "isInverse")]
    pub(super) is_inverse: bool,
    #[serde(default, rename = "marketType")]
    pub(super) market_type: String,
    #[serde(default)]
    pub(super) status: String,
    #[serde(default, rename = "fundingFeeRate")]
    funding_fee_rate: serde_json::Value,
    #[serde(default, rename = "predictedFundingFeeRate")]
    predicted_funding_fee_rate: serde_json::Value,
    #[serde(default, rename = "fundingRateGranularity")]
    funding_rate_granularity: serde_json::Value,
    #[serde(default, rename = "nextFundingRateDateTime")]
    next_funding_rate_date_time: serde_json::Value,
    #[serde(default, rename = "lastTradePrice")]
    last_trade_price: serde_json::Value,
    #[serde(default, rename = "turnoverOf24h")]
    turnover_of_24h: serde_json::Value,
    #[serde(default, rename = "markPrice")]
    mark_price: serde_json::Value,
    #[serde(default, rename = "indexPrice")]
    index_price: serde_json::Value,
    #[serde(default, rename = "openInterest")]
    open_interest: serde_json::Value,
}

#[derive(Clone, Debug)]
pub(super) struct KucoinContractSpec {
    pub(super) native_symbol: String,
    pub(super) order_unit: f64,
    pub(super) price_tick: f64,
    pub(super) lot_size: f64,
    pub(super) funding_interval_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct KucoinContractIdentity {
    pub(super) native_symbol: String,
    pub(super) normalized_symbol: String,
    pub(super) quote_currency: String,
    pub(super) settle_currency: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct FuturesTickerItem {
    pub(super) symbol: String,
    #[serde(default)]
    price: serde_json::Value,
    #[serde(default, rename = "bestBidPrice")]
    best_bid_price: serde_json::Value,
    #[serde(default, rename = "bestAskPrice")]
    best_ask_price: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct SpotTickerData {
    /// KuCoin `market/allTickers` 包络层服务器时间（毫秒），作为整快照的交易所时间戳。
    #[serde(default)]
    pub(super) time: i64,
    #[serde(default = "Vec::new")]
    pub(super) ticker: Vec<SpotTickerItem>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SpotTickerItem {
    pub(super) symbol: String,
    #[serde(default, deserialize_with = "nullable_string")]
    buy: String,
    #[serde(default, rename = "bestBidSize", deserialize_with = "nullable_string")]
    best_bid_size: String,
    #[serde(default, deserialize_with = "nullable_string")]
    sell: String,
    #[serde(default, rename = "bestAskSize", deserialize_with = "nullable_string")]
    best_ask_size: String,
    #[serde(default, deserialize_with = "nullable_string")]
    last: String,
    #[serde(default, rename = "volValue", deserialize_with = "nullable_string")]
    pub(super) vol_value: String,
}

fn nullable_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Deserialize)]
pub(super) struct DepthData {
    #[serde(default)]
    pub(super) bids: Vec<[f64; 2]>,
    #[serde(default)]
    pub(super) asks: Vec<[f64; 2]>,
    #[serde(default)]
    pub(super) ts: i64,
}

#[derive(Debug, Deserialize)]
pub(super) struct SpotDepthData {
    #[serde(default)]
    pub(super) bids: Vec<[serde_json::Value; 2]>,
    #[serde(default)]
    pub(super) asks: Vec<[serde_json::Value; 2]>,
    #[serde(default)]
    pub(super) time: i64,
}

pub(super) fn contract_spec(contract: &ContractActive) -> Option<KucoinContractSpec> {
    if !contract_is_open(contract) {
        return None;
    }
    let order_unit = flex_f64_positive(&contract.multiplier)?;
    let price_tick = flex_f64_positive(&contract.tick_size)?;
    let lot_size = flex_f64_positive(&contract.lot_size)?;
    let max_order_qty = flex_f64_positive(&contract.max_order_qty)?;
    let market_max_order_qty = flex_f64_positive(&contract.market_max_order_qty)?;
    if contract.is_inverse
        || lot_size.fract() != 0.0
        || lot_size > max_order_qty
        || market_max_order_qty > max_order_qty
    {
        return None;
    }
    let quote_currency = nonempty_upper(&contract.quote_currency)?;
    let settle_currency = nonempty_upper(&contract.settle_currency)?;
    if quote_currency != settle_currency {
        return None;
    }
    Some(KucoinContractSpec {
        native_symbol: nonempty_upper(&contract.symbol)?,
        order_unit,
        price_tick,
        lot_size,
        funding_interval_ms: positive_i64(&contract.funding_rate_granularity),
    })
}

pub(super) fn contract_identity(contract: &ContractActive) -> Option<KucoinContractIdentity> {
    if !contract_is_open(contract) {
        return None;
    }
    let base = nonempty_upper(&contract.base_currency)?;
    Some(KucoinContractIdentity {
        native_symbol: nonempty_upper(&contract.symbol)?,
        normalized_symbol: from_kucoin_base(&base),
        quote_currency: nonempty_upper(&contract.quote_currency)?,
        settle_currency: nonempty_upper(&contract.settle_currency)?,
    })
}

pub(super) fn contract_is_open(contract: &ContractActive) -> bool {
    contract.status.is_empty() || contract.status.eq_ignore_ascii_case("open")
}

pub(super) fn parse_spot_depth_levels(levels: Vec<[serde_json::Value; 2]>) -> Vec<[f64; 2]> {
    levels
        .into_iter()
        .filter_map(|[price, size]| {
            let price = flex_f64(&price);
            let size = flex_f64(&size);
            (price > 0.0 && size > 0.0).then_some([price, size])
        })
        .collect()
}

pub(super) fn kucoin_to_normalized(symbol: &str) -> String {
    let upper = symbol.to_ascii_uppercase();
    let stripped = upper
        .strip_suffix("USDTM")
        .or_else(|| upper.strip_suffix("USDCM"))
        .or_else(|| upper.strip_suffix("USDM"))
        .unwrap_or(&upper);
    from_kucoin_base(stripped)
}

pub(super) fn normalized_to_kucoin(symbol: &str) -> String {
    let upper = symbol.to_ascii_uppercase();
    if upper.ends_with('M') && !upper.contains('-') && !upper.contains('_') && !upper.contains('/')
    {
        return upper;
    }
    upper
}

fn nonempty_upper(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_ascii_uppercase())
}

#[cfg(test)]
fn to_kucoin_base(coin: &str) -> Option<&'static str> {
    match coin.to_ascii_uppercase().as_str() {
        "BTC" => Some("XBT"),
        _ => None,
    }
}

fn from_kucoin_base(coin: &str) -> String {
    match coin.to_ascii_uppercase().as_str() {
        "XBT" => "BTC".to_owned(),
        other => other.to_owned(),
    }
}

pub(super) fn snap_kucoin_depth_endpoint(depth: u32) -> &'static str {
    if depth == 0 || depth > 20 {
        "depth100"
    } else {
        "depth20"
    }
}

pub(super) fn parse_funding(contract: &ContractActive) -> Option<FundingRateData> {
    let interval_hours = funding_interval_hours(&contract.funding_rate_granularity)?;
    let rate = finite_f64(&contract.funding_fee_rate)?;
    let next_funding_time = positive_i64(&contract.next_funding_rate_date_time)?;
    let volume_24h = nonnegative_f64(&contract.turnover_of_24h)?;
    Some(FundingRateData {
        symbol: kucoin_to_normalized(&contract.symbol),
        exchange: NAME.into(),
        rate,
        rate_8h: rate * (8.0 / interval_hours as f64),
        predicted_rate: finite_f64_opt(&contract.predicted_funding_fee_rate),
        next_funding_time,
        funding_interval: interval_hours,
        volume_24h,
        timestamp: now_ms(),
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    })
}

/// Parse a KuCoin futures ticker, failing closed on required prices.
///
/// `bid`/`ask`/`last` must resolve to a positive finite value or the tick is
/// dropped (returns `None`) instead of fabricating a 0.0 quote that the
/// opportunity scanner / risk would treat as a real price. `last` is taken from
/// the freshest positive of the quote price and the contract last-trade price;
/// `bid`/`ask` prefer the quote BBO and otherwise fall back to `last`.
/// `volume_24h` stays best-effort (non-execution).
pub(super) fn parse_ticker(
    contract: &ContractActive,
    quote: Option<&FuturesTickerItem>,
) -> Option<TickerInfo> {
    let contract_last = flex_f64_positive(&contract.last_trade_price);
    let quote_last = quote.and_then(|item| flex_f64_positive(&item.price));
    let last = match (quote_last, contract_last) {
        (Some(q), Some(c)) => q.max(c),
        (Some(value), None) | (None, Some(value)) => value,
        (None, None) => return None,
    };
    let bid = quote
        .and_then(|item| flex_f64_positive(&item.best_bid_price))
        .unwrap_or(last);
    let ask = quote
        .and_then(|item| flex_f64_positive(&item.best_ask_price))
        .unwrap_or(last);
    Some(TickerInfo {
        symbol: kucoin_to_normalized(&contract.symbol),
        exchange: NAME.into(),
        bid,
        ask,
        last,
        volume_24h: flex_f64(&contract.turnover_of_24h),
        timestamp: now_ms(),
    })
}

pub(super) fn parse_mark_index(contract: &ContractActive) -> Option<MarkIndexInfo> {
    Some(MarkIndexInfo {
        symbol: kucoin_to_normalized(&contract.symbol),
        exchange: NAME.into(),
        mark_price: flex_f64_positive(&contract.mark_price)?,
        index_price: flex_f64_positive(&contract.index_price),
        open_interest: flex_f64_positive(&contract.open_interest),
        open_interest_value: None,
        timestamp: now_ms(),
    })
}

pub(super) fn parse_spot_tick(
    ticker: &SpotTickerItem,
    exchange_ts_ms: Option<i64>,
) -> Option<SpotTick> {
    let (base, quote) = spot_pair(&ticker.symbol)?;
    crate::spot::spot_tick(crate::spot::SpotFields {
        venue: NAME,
        base: &base,
        quote: &quote,
        bid: &ticker.buy,
        ask: &ticker.sell,
        last: &ticker.last,
        bid_size: Some(&ticker.best_bid_size),
        ask_size: Some(&ticker.best_ask_size),
        volume_24h: &ticker.vol_value,
        exchange_ts_ms,
    })
}

pub(super) fn spot_symbol_matches(symbol: &str, symbols: Option<&[String]>) -> bool {
    let Some((base, quote)) = spot_pair(symbol) else {
        return false;
    };
    crate::spot::symbol_matches(symbol, &base, &quote, symbols)
}

fn spot_pair(symbol: &str) -> Option<(String, String)> {
    let (base, quote) = crate::spot::delimited_pair(symbol, '-')?;
    Some((from_kucoin_base(&base), quote))
}

fn flex_f64(value: &serde_json::Value) -> f64 {
    match value {
        serde_json::Value::Number(number) => number.as_f64().unwrap_or(0.0),
        serde_json::Value::String(text) => text.parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn flex_f64_positive(value: &serde_json::Value) -> Option<f64> {
    let value = flex_f64(value);
    (value.is_finite() && value > f64::EPSILON).then_some(value)
}

fn nonnegative_f64(value: &serde_json::Value) -> Option<f64> {
    let value = finite_f64(value)?;
    (value >= 0.0).then_some(value)
}

fn finite_f64(value: &serde_json::Value) -> Option<f64> {
    finite_f64_opt(value)
}

fn flex_i64(value: &serde_json::Value) -> i64 {
    match value {
        serde_json::Value::Number(number) => number
            .as_i64()
            .unwrap_or_else(|| number.as_f64().unwrap_or(0.0) as i64),
        serde_json::Value::String(text) => text.parse().unwrap_or(0),
        _ => 0,
    }
}

fn positive_i64(value: &serde_json::Value) -> Option<i64> {
    let value = flex_i64(value);
    (value > 0).then_some(value)
}

fn funding_interval_hours(value: &serde_json::Value) -> Option<u32> {
    let granularity_ms = positive_i64(value)?;
    let interval_ms = 3_600_000;
    if granularity_ms % interval_ms != 0 {
        return None;
    }
    let hours = granularity_ms / interval_ms;
    (1..=24).contains(&hours).then_some(hours as u32)
}

fn flex_f64_opt(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn finite_f64_opt(value: &serde_json::Value) -> Option<f64> {
    flex_f64_opt(value).filter(|value| value.is_finite())
}

#[cfg(test)]
#[path = "kucoin_market_data_tests.rs"]
mod tests;
