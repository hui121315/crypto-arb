//! Bybit public market response parsing.

use crate::adapter::strip_common_suffixes;
use common::time::now_ms;
use serde::Deserialize;
use shared_types::{
    FundingRateData, IndexComponent, IndexCompositionQuality, IndexCompositionSnapshot,
    MarkIndexInfo, SpotTick, TickerInfo,
};

const NAME: &str = "bybit";

/// Bybit V5 `/v5/market/tickers` row.
#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct MarketTickerItem {
    pub(super) symbol: String,
    #[serde(default, rename = "lastPrice")]
    pub(super) last_price: String,
    #[serde(default, rename = "bid1Price")]
    pub(super) bid1_price: String,
    #[serde(default, rename = "bid1Size")]
    pub(super) bid1_size: String,
    #[serde(default, rename = "ask1Price")]
    pub(super) ask1_price: String,
    #[serde(default, rename = "ask1Size")]
    pub(super) ask1_size: String,
    #[serde(default)]
    pub(super) turnover24h: String,
    #[serde(default, rename = "fundingRate")]
    pub(super) funding_rate: String,
    #[serde(default, rename = "nextFundingTime")]
    pub(super) next_funding_time: String,
    #[serde(default, rename = "fundingIntervalHour")]
    pub(super) funding_interval_hour: String,
    #[serde(default, rename = "markPrice")]
    pub(super) mark_price: String,
    #[serde(default, rename = "indexPrice")]
    pub(super) index_price: String,
    #[serde(default, rename = "openInterest")]
    pub(super) open_interest: String,
    #[serde(default, rename = "openInterestValue")]
    pub(super) open_interest_value: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct InstrumentInfoItem {
    pub(super) symbol: String,
    #[serde(
        default,
        rename = "fundingInterval",
        deserialize_with = "de_flex_minutes"
    )]
    pub(super) funding_interval: i64,
    #[serde(default, rename = "settleCoin")]
    pub(super) settle_coin: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct IndexComponentsItem {
    #[serde(default, rename = "indexName")]
    pub(super) index_name: String,
    #[serde(default, rename = "updateTime")]
    pub(super) update_time: String,
    #[serde(default)]
    pub(super) components: Vec<IndexComponentItem>,
}

#[derive(Debug, Deserialize)]
pub(super) struct IndexComponentItem {
    #[serde(default)]
    pub(super) exchange: String,
    #[serde(default, rename = "spotPair")]
    pub(super) spot_pair: String,
    #[serde(default, rename = "equivalentPrice")]
    pub(super) equivalent_price: String,
    #[serde(default)]
    pub(super) price: String,
    #[serde(default)]
    pub(super) weight: String,
}

fn de_flex_minutes<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Deserialize};
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(number) => number
            .as_i64()
            .ok_or_else(|| de::Error::custom("invalid integer")),
        serde_json::Value::String(text) => text.parse::<i64>().map_err(de::Error::custom),
        serde_json::Value::Null => Ok(0),
        _ => Err(de::Error::custom("expected number or string")),
    }
}

pub(super) fn funding_interval_minutes_to_hours(minutes: i64) -> u32 {
    let minutes = u32::try_from(minutes)
        .ok()
        .filter(|&value| value > 0)
        .unwrap_or(480);
    ((f64::from(minutes) / 60.0).round() as u32).clamp(1, 24)
}

pub(super) fn is_usdm_perp(symbol: &str) -> bool {
    symbol.ends_with("USDT") || symbol.ends_with("USDC") || symbol.ends_with("PERP")
}

pub(super) fn is_usdt_perp(symbol: &str) -> bool {
    symbol.ends_with("USDT")
}

/// Normalize a Bybit V5 linear symbol without erasing an explicit USDC market.
/// Bybit uses both `BTCUSDC`-style and `BTCPERP` native symbols for USDC products.
pub(super) fn linear_stream_symbol(symbol: &str) -> String {
    let compact = symbol
        .trim()
        .chars()
        .filter(|character| !matches!(character, '/' | '-' | '_'))
        .collect::<String>()
        .to_ascii_uppercase();
    for (suffix, quote) in [
        ("USDTSWAP", "USDT"),
        ("USDTPERP", "USDT"),
        ("USDCSWAP", "USDC"),
        ("USDCPERP", "USDC"),
    ] {
        if let Some(base) = compact.strip_suffix(suffix) {
            return format!("{base}{quote}");
        }
    }
    if is_usdm_perp(&compact) {
        return compact;
    }
    format!("{}USDT", strip_common_suffixes(symbol))
}

pub(super) fn clamp_orderbook_limit(category: &str, depth: u32) -> u32 {
    let max = match category {
        "spot" | "linear" | "inverse" => 1_000,
        "option" => 25,
        _ => 1_000,
    };
    depth.clamp(1, max)
}

pub(super) fn parse_funding(
    item: &MarketTickerItem,
    interval_hours: u32,
    volume_24h: f64,
    server_time_ms: i64,
) -> Option<FundingRateData> {
    let rate = parse_optional_f64(&item.funding_rate)?;
    let next_time = item
        .next_funding_time
        .parse::<i64>()
        .ok()
        .filter(|value| *value > 0)?;
    let interval = interval_hours.clamp(1, 24);
    Some(FundingRateData {
        symbol: strip_common_suffixes(&item.symbol),
        exchange: NAME.into(),
        rate,
        rate_8h: rate * (8.0 / interval as f64),
        predicted_rate: None,
        next_funding_time: next_time,
        funding_interval: interval,
        volume_24h,
        timestamp: if server_time_ms > 0 {
            server_time_ms
        } else {
            now_ms()
        },
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    })
}

pub(super) fn parse_ws_funding(
    item: &MarketTickerItem,
    volume_24h: f64,
    timestamp_ms: i64,
) -> Option<FundingRateData> {
    let interval = item
        .funding_interval_hour
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0)?;
    parse_funding(item, interval, volume_24h, timestamp_ms)
}

/// Parse a Bybit ticker, failing closed when a required price cannot be read.
///
/// `bid1Price`/`ask1Price`/`lastPrice` drive execution and risk decisions, so an
/// empty or unparseable value yields `None` (the tick is dropped) instead of a
/// fabricated `0.0` price that downstream scanners would treat as a real quote.
/// `turnover24h` stays best-effort because it does not gate execution.
pub(super) fn parse_ticker(item: &MarketTickerItem) -> Option<TickerInfo> {
    let bid = parse_optional_f64(&item.bid1_price)?;
    let ask = parse_optional_f64(&item.ask1_price)?;
    let last = parse_optional_f64(&item.last_price)?;
    Some(TickerInfo {
        symbol: strip_common_suffixes(&item.symbol),
        exchange: NAME.into(),
        bid,
        ask,
        last,
        volume_24h: parse_f64(&item.turnover24h),
        timestamp: now_ms(),
    })
}

pub(super) fn parse_mark_index(item: &MarketTickerItem, timestamp: i64) -> Option<MarkIndexInfo> {
    Some(MarkIndexInfo {
        symbol: strip_common_suffixes(&item.symbol),
        exchange: NAME.into(),
        mark_price: parse_positive(&item.mark_price)?,
        index_price: parse_optional_f64(&item.index_price),
        open_interest: parse_optional_f64(&item.open_interest),
        open_interest_value: parse_optional_f64(&item.open_interest_value),
        timestamp: if timestamp > 0 { timestamp } else { now_ms() },
    })
}

pub(super) fn parse_spot_tick(item: &MarketTickerItem, timestamp_ms: i64) -> Option<SpotTick> {
    let (base, quote) = crate::spot::suffix_pair(&item.symbol)?;
    crate::spot::spot_tick(crate::spot::SpotFields {
        venue: NAME,
        base: &base,
        quote: &quote,
        bid: &item.bid1_price,
        ask: &item.ask1_price,
        last: &item.last_price,
        bid_size: Some(&item.bid1_size),
        ask_size: Some(&item.ask1_size),
        volume_24h: &item.turnover24h,
        exchange_ts_ms: Some(timestamp_ms),
    })
}

pub(super) fn parse_index_components(item: IndexComponentsItem) -> IndexCompositionSnapshot {
    let received_at_ms = item.update_time.parse().unwrap_or_else(|_| now_ms());
    IndexCompositionSnapshot {
        venue: NAME.to_owned(),
        symbol: strip_common_suffixes(&item.index_name),
        index_id: item.index_name,
        components: item.components.into_iter().map(index_component).collect(),
        quality: IndexCompositionQuality::Verified,
        source: "GET /v5/market/index-price-components".to_owned(),
        received_at_ms,
        freshness_ms: Some(0),
        error: None,
        retry_after_ms: None,
        source_url: None,
        payload_sha256: None,
        schema_version: Some("bybit-v5-index-price-components/1".to_owned()),
    }
}

fn index_component(item: IndexComponentItem) -> IndexComponent {
    IndexComponent {
        symbol: item.spot_pair,
        name: item.exchange,
        weight: parse_f64(&item.weight),
        price: parse_positive(&item.price).or_else(|| parse_positive(&item.equivalent_price)),
    }
}

pub(super) fn spot_symbol_matches(symbol: &str, symbols: Option<&[String]>) -> bool {
    let Some((base, quote)) = crate::spot::suffix_pair(symbol) else {
        return false;
    };
    crate::spot::symbol_matches(symbol, &base, &quote, symbols)
}

fn parse_f64(value: &str) -> f64 {
    value.parse().unwrap_or(0.0)
}

fn parse_positive(value: &str) -> Option<f64> {
    parse_optional_f64(value).filter(|value| *value > 0.0)
}

fn parse_optional_f64(value: &str) -> Option<f64> {
    value.parse().ok().filter(|value: &f64| value.is_finite())
}

#[cfg(test)]
#[path = "bybit_market_data_tests.rs"]
mod tests;
