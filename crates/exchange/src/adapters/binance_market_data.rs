//! Binance public market response parsing.

use crate::adapter::strip_common_suffixes;
use common::time::now_ms;
use serde::Deserialize;
use shared_types::{
    FundingRateData, IndexComponent, IndexCompositionQuality, IndexCompositionSnapshot,
    MarkIndexInfo, SpotTick, TickerInfo,
};
use std::collections::HashMap;

const NAME: &str = "binance";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PremiumIndexItem {
    pub(super) symbol: String,
    #[serde(default)]
    pub(super) mark_price: String,
    #[serde(default)]
    pub(super) index_price: String,
    #[serde(default)]
    pub(super) last_funding_rate: String,
    #[serde(default)]
    pub(super) next_funding_time: i64,
    #[serde(default)]
    pub(super) time: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Ticker24hItem {
    pub(super) symbol: String,
    #[serde(default)]
    pub(super) last_price: String,
    #[serde(default)]
    pub(super) bid_price: String,
    #[serde(default)]
    pub(super) ask_price: String,
    #[serde(default)]
    pub(super) quote_volume: String,
    #[serde(default)]
    pub(super) close_time: i64,
}

/// Binance USDⓈ-M `GET /fapi/v1/ticker/bookTicker`.
/// Official docs: <https://developers.binance.com/docs/derivatives/usds-margined-futures/market-data/rest-api/Symbol-Order-Book-Ticker>
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BookTickerItem {
    pub(super) symbol: String,
    #[serde(default)]
    pub(super) bid_price: String,
    #[serde(default)]
    pub(super) ask_price: String,
    #[serde(default)]
    pub(super) time: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OpenInterestItem {
    pub(super) symbol: String,
    #[serde(default)]
    pub(super) open_interest: String,
    #[serde(default)]
    pub(super) time: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IndexConstituentsResponse {
    pub(super) symbol: String,
    #[serde(default)]
    pub(super) time: i64,
    #[serde(default)]
    pub(super) constituents: Vec<IndexConstituentItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IndexConstituentItem {
    pub(super) exchange: String,
    pub(super) symbol: String,
    #[serde(default)]
    pub(super) price: String,
    #[serde(default)]
    pub(super) weight: String,
}

/// Parse Binance USDM funding data.
///
/// `rate` keeps the raw single-period funding rate. `rate_8h` is normalized for
/// cross-venue comparison when Binance reports non-8h funding intervals.
pub(super) fn parse_funding(
    item: &PremiumIndexItem,
    volume_24h: f64,
    interval_hours: u32,
) -> Option<FundingRateData> {
    let rate = item
        .last_funding_rate
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())?;
    if item.next_funding_time <= 0 {
        return None;
    }
    let interval = interval_hours.max(1);
    let rate_8h = rate * (8.0 / interval as f64);
    Some(FundingRateData {
        symbol: strip_common_suffixes(&item.symbol),
        exchange: NAME.into(),
        rate,
        rate_8h,
        predicted_rate: None,
        next_funding_time: item.next_funding_time,
        funding_interval: interval,
        volume_24h,
        timestamp: if item.time == 0 { now_ms() } else { item.time },
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    })
}

pub(super) fn parse_mark_index(item: &PremiumIndexItem) -> Option<MarkIndexInfo> {
    let mark_price = parse_positive(&item.mark_price)?;
    Some(MarkIndexInfo {
        symbol: strip_common_suffixes(&item.symbol),
        exchange: NAME.into(),
        mark_price,
        index_price: parse_positive(&item.index_price),
        open_interest: None,
        open_interest_value: None,
        timestamp: if item.time == 0 { now_ms() } else { item.time },
    })
}

pub(super) fn parse_open_interest(item: &OpenInterestItem) -> Option<(String, f64, i64)> {
    Some((
        item.symbol.clone(),
        parse_positive(&item.open_interest)?,
        item.time,
    ))
}

pub(super) fn parse_index_constituents(
    item: IndexConstituentsResponse,
) -> IndexCompositionSnapshot {
    let received_at_ms = if item.time == 0 { now_ms() } else { item.time };
    IndexCompositionSnapshot {
        venue: NAME.to_owned(),
        symbol: strip_common_suffixes(&item.symbol),
        index_id: item.symbol,
        components: item.constituents.into_iter().map(index_component).collect(),
        quality: IndexCompositionQuality::Verified,
        source: "GET /fapi/v1/constituents".to_owned(),
        received_at_ms,
        freshness_ms: Some(0),
        error: None,
        retry_after_ms: None,
        source_url: None,
        payload_sha256: None,
        schema_version: Some("binance-fapi-constituents/1".to_owned()),
    }
}

fn index_component(item: IndexConstituentItem) -> IndexComponent {
    IndexComponent {
        symbol: item.symbol,
        name: item.exchange,
        weight: parse_f64(&item.weight),
        price: parse_positive(&item.price),
    }
}

pub(super) fn book_ticker_index(rows: Vec<BookTickerItem>) -> HashMap<String, BookTickerItem> {
    rows.into_iter()
        .map(|row| (row.symbol.clone(), row))
        .collect()
}

/// Parse a Binance USDⓈ-M ticker, failing closed when a required price is absent.
///
/// Best bid/ask (sourced from the dedicated `bookTicker` endpoint, since the 24h
/// ticker does not carry them) and `lastPrice` drive execution and risk
/// decisions, so a missing/zero/unparseable required price yields `None` (the
/// tick is dropped) instead of a fabricated `0.0` price that downstream scanners
/// would treat as a real quote. `quoteVolume` stays best-effort.
pub(super) fn parse_ticker(
    t: &Ticker24hItem,
    normalized: &str,
    book: Option<&BookTickerItem>,
) -> Option<TickerInfo> {
    let timestamp = book
        .and_then(|row| (row.time > 0).then_some(row.time))
        .unwrap_or(t.close_time);
    let bid = book
        .and_then(|row| parse_positive(&row.bid_price))
        .or_else(|| parse_positive(&t.bid_price))?;
    let ask = book
        .and_then(|row| parse_positive(&row.ask_price))
        .or_else(|| parse_positive(&t.ask_price))?;
    let last = parse_positive(&t.last_price)?;
    Some(TickerInfo {
        symbol: normalized.to_owned(),
        exchange: NAME.into(),
        bid,
        ask,
        last,
        volume_24h: parse_f64(&t.quote_volume),
        timestamp: if timestamp == 0 { now_ms() } else { timestamp },
    })
}

pub(super) fn parse_spot_tick(t: &Ticker24hItem) -> Option<SpotTick> {
    let (base, quote) = crate::spot::suffix_pair(&t.symbol)?;
    crate::spot::spot_tick(crate::spot::SpotFields {
        venue: NAME,
        base: &base,
        quote: &quote,
        bid: &t.bid_price,
        ask: &t.ask_price,
        last: &t.last_price,
        bid_size: None,
        ask_size: None,
        volume_24h: &t.quote_volume,
        exchange_ts_ms: Some(t.close_time),
    })
}

pub(super) fn spot_symbol_matches(symbol: &str, symbols: Option<&[String]>) -> bool {
    let Some((base, quote)) = crate::spot::suffix_pair(symbol) else {
        return false;
    };
    crate::spot::symbol_matches(symbol, &base, &quote, symbols)
}

/// Snap arbitrary depth to Binance `/fapi/v1/depth` accepted limits.
const BINANCE_DEPTH_LEVELS: [u32; 7] = [5, 10, 20, 50, 100, 500, 1000];

pub(super) fn snap_binance_depth(depth: u32) -> u32 {
    BINANCE_DEPTH_LEVELS
        .iter()
        .copied()
        .min_by_key(|v| v.abs_diff(depth))
        .unwrap_or(5)
}

pub(super) fn parse_depth_levels(levels: Vec<[String; 2]>) -> Vec<[f64; 2]> {
    levels
        .into_iter()
        .filter_map(|[price, quantity]| Some([price.parse().ok()?, quantity.parse().ok()?]))
        .collect()
}

fn parse_positive(value: &str) -> Option<f64> {
    let value = parse_f64(value);
    (value.is_finite() && value > f64::EPSILON).then_some(value)
}

fn parse_f64(value: &str) -> f64 {
    value.parse().unwrap_or(0.0)
}

#[cfg(test)]
#[path = "binance_market_data_tests.rs"]
mod tests;
