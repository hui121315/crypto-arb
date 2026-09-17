//! Gate public market response parsing.

use crate::adapter::strip_common_suffixes;
use common::time::now_ms;
use serde::Deserialize;
use shared_types::{
    FundingRateData, IndexComponent, IndexCompositionQuality, IndexCompositionSnapshot,
    MarkIndexInfo, SpotTick, TickerInfo,
};

const NAME: &str = "gate";

#[derive(Debug, Deserialize)]
pub(super) struct ContractItem {
    pub(super) name: String,
    #[serde(default, rename = "funding_rate")]
    pub(super) funding_rate: String,
    #[serde(default, rename = "funding_rate_indicative")]
    pub(super) funding_rate_indicative: String,
    #[serde(default, rename = "funding_next_apply")]
    pub(super) funding_next_apply: i64,
    #[serde(default, rename = "funding_interval")]
    pub(super) funding_interval: i64,
    #[allow(dead_code)]
    #[serde(default, rename = "in_delisting")]
    pub(super) in_delisting: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct TickerItem {
    pub(super) contract: String,
    #[serde(default)]
    pub(super) last: String,
    #[serde(default, rename = "highest_bid")]
    pub(super) highest_bid: String,
    #[serde(default, rename = "lowest_ask")]
    pub(super) lowest_ask: String,
    #[serde(default, rename = "volume_24h_quote")]
    pub(super) volume_24h_quote: String,
    #[serde(default, rename = "volume_24h_settle")]
    pub(super) volume_24h_settle: String,
    /// Funding rate for the current epoch (string-encoded; `""` until the
    /// futures contract reports its first funding).
    #[serde(default, rename = "funding_rate")]
    pub(super) funding_rate: String,
    /// Indicative funding rate for the next epoch.
    #[serde(default, rename = "funding_rate_indicative")]
    pub(super) funding_rate_indicative: String,
    /// Unix seconds (Gate emits this as a number; `0` if missing).
    #[serde(default, rename = "funding_next_apply")]
    pub(super) funding_next_apply: i64,
    #[serde(default, rename = "mark_price")]
    pub(super) mark_price: String,
    #[serde(default, rename = "index_price")]
    pub(super) index_price: String,
    #[serde(default, rename = "total_size")]
    pub(super) total_size: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct SpotTickerItem {
    pub(super) currency_pair: String,
    #[serde(default)]
    pub(super) last: String,
    #[serde(default)]
    pub(super) highest_bid: String,
    #[serde(default)]
    pub(super) lowest_ask: String,
    #[serde(default)]
    pub(super) quote_volume: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct OrderBookResp {
    #[serde(default)]
    pub(super) bids: Vec<DepthLevel>,
    #[serde(default)]
    pub(super) asks: Vec<DepthLevel>,
    #[serde(default)]
    pub(super) update: f64,
}

#[derive(Debug, Deserialize)]
pub(super) struct IndexConstituentsResp {
    pub(super) index: String,
    #[serde(default)]
    pub(super) constituents: Vec<IndexConstituentItem>,
}

#[derive(Debug, Deserialize)]
pub(super) struct IndexConstituentItem {
    #[serde(default)]
    pub(super) exchange: String,
    #[serde(default)]
    pub(super) symbols: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum DepthLevel {
    Object { p: String, s: serde_json::Value },
    Array([serde_json::Value; 2]),
}

pub(super) fn parse_levels(levels: Vec<DepthLevel>) -> Vec<[f64; 2]> {
    levels
        .into_iter()
        .filter_map(|level| {
            let (price, size) = match level {
                DepthLevel::Object { p, s } => (serde_json::Value::String(p), s),
                DepthLevel::Array([p, s]) => (p, s),
            };
            Some([value_f64(&price)?, value_f64(&size)?])
        })
        .collect()
}

fn value_f64(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

pub(super) fn parse_funding(contract: &ContractItem, volume_24h: f64) -> Option<FundingRateData> {
    let rate = parse_optional_f64(&contract.funding_rate)?;
    if contract.funding_interval <= 0 {
        return None;
    }
    let interval_hours = funding_interval_hours(contract.funding_interval);
    let next_time_ms = contract.funding_next_apply.checked_mul(1000)?;
    if next_time_ms <= 0 {
        return None;
    }
    let predicted_rate = contract
        .funding_rate_indicative
        .parse()
        .ok()
        .filter(|value: &f64| value.is_finite());
    Some(FundingRateData {
        symbol: strip_common_suffixes(&contract.name),
        exchange: NAME.into(),
        rate,
        rate_8h: rate * (8.0 / interval_hours as f64),
        predicted_rate,
        next_funding_time: next_time_ms,
        funding_interval: interval_hours,
        volume_24h,
        timestamp: now_ms(),
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    })
}

pub(super) fn funding_interval_hours(seconds: i64) -> u32 {
    if seconds > 0 {
        ((seconds / 3600) as u32).clamp(1, 24)
    } else {
        8
    }
}

/// Build [`FundingRateData`] from a WS `futures.tickers` row when the funding
/// fields have been populated; returns `None` if the row carries no funding
/// signal yet (some contracts emit `""`/`0` before their first funding apply).
/// `interval_hours` is sourced from the contract metadata cache because the
/// WS payload does not carry the interval itself.
pub(super) fn parse_funding_from_ws_ticker(
    ticker: &TickerItem,
    interval_hours: u32,
) -> Option<FundingRateData> {
    let next_funding_time = ticker.funding_next_apply.checked_mul(1000)?;
    parse_funding_from_ticker_schedule(ticker, interval_hours, next_funding_time)
}

pub(super) fn parse_funding_from_ticker_schedule(
    ticker: &TickerItem,
    interval_hours: u32,
    next_funding_time: i64,
) -> Option<FundingRateData> {
    let rate = ticker
        .funding_rate
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())?;
    if rate == 0.0 && ticker.funding_rate.trim().is_empty() {
        return None;
    }
    let predicted_rate = ticker
        .funding_rate_indicative
        .parse()
        .ok()
        .filter(|value: &f64| value.is_finite());
    if next_funding_time <= 0 {
        return None;
    }
    let interval = interval_hours.clamp(1, 24);
    Some(FundingRateData {
        symbol: strip_common_suffixes(&ticker.contract),
        exchange: NAME.into(),
        rate,
        rate_8h: rate * (8.0 / f64::from(interval)),
        predicted_rate,
        next_funding_time,
        funding_interval: interval,
        volume_24h: parse_quote_volume(ticker),
        timestamp: now_ms(),
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    })
}

/// Parse a Gate REST ticker, failing closed when a required price is unreadable.
///
/// `highest_bid`/`lowest_ask`/`last` drive execution and risk decisions, so a
/// blank or unparseable value yields `None` (the tick is dropped) instead of a
/// fabricated `0.0` price that downstream scanners would treat as a real quote.
/// The WS path builds its quote from the dedicated book-ticker channel (see
/// `gate_ws_ticker_data::to_ticker`) and does not rely on these REST fields.
pub(super) fn parse_ticker(ticker: &TickerItem) -> Option<TickerInfo> {
    let bid = parse_optional_f64(&ticker.highest_bid)?;
    let ask = parse_optional_f64(&ticker.lowest_ask)?;
    let last = parse_optional_f64(&ticker.last)?;
    Some(TickerInfo {
        symbol: strip_common_suffixes(&ticker.contract),
        exchange: NAME.into(),
        bid,
        ask,
        last,
        volume_24h: parse_quote_volume(ticker),
        timestamp: now_ms(),
    })
}

pub(super) fn parse_mark_index(ticker: &TickerItem, timestamp: i64) -> Option<MarkIndexInfo> {
    Some(MarkIndexInfo {
        symbol: strip_common_suffixes(&ticker.contract),
        exchange: NAME.into(),
        mark_price: parse_positive(&ticker.mark_price)?,
        index_price: parse_optional_f64(&ticker.index_price),
        open_interest: parse_optional_f64(&ticker.total_size),
        open_interest_value: None,
        timestamp,
    })
}

pub(super) fn parse_index_constituents(item: IndexConstituentsResp) -> IndexCompositionSnapshot {
    IndexCompositionSnapshot {
        venue: NAME.to_owned(),
        symbol: strip_common_suffixes(&item.index),
        index_id: item.index,
        components: item
            .constituents
            .into_iter()
            .flat_map(index_components)
            .collect(),
        quality: IndexCompositionQuality::Verified,
        source: "GET /api/v4/futures/usdt/index_constituents/{index}".to_owned(),
        received_at_ms: now_ms(),
        freshness_ms: Some(0),
        error: None,
        retry_after_ms: None,
        source_url: None,
        payload_sha256: None,
        schema_version: Some("gate-v4-index-constituents/1".to_owned()),
    }
}

fn index_components(item: IndexConstituentItem) -> Vec<IndexComponent> {
    item.symbols
        .into_iter()
        .map(|symbol| IndexComponent {
            symbol,
            name: item.exchange.clone(),
            // Gate does not expose component weights or prices on this endpoint.
            weight: 0.0,
            price: None,
        })
        .collect()
}

pub(super) fn parse_quote_volume(ticker: &TickerItem) -> f64 {
    ticker
        .volume_24h_quote
        .parse()
        .ok()
        .or_else(|| ticker.volume_24h_settle.parse().ok())
        .unwrap_or(0.0)
}

pub(super) fn parse_spot_tick(ticker: &SpotTickerItem) -> Option<SpotTick> {
    let (base, quote) = crate::spot::delimited_pair(&ticker.currency_pair, '_')?;
    crate::spot::spot_tick(crate::spot::SpotFields {
        venue: NAME,
        base: &base,
        quote: &quote,
        bid: &ticker.highest_bid,
        ask: &ticker.lowest_ask,
        last: &ticker.last,
        bid_size: None,
        ask_size: None,
        volume_24h: &ticker.quote_volume,
        exchange_ts_ms: None,
    })
}

pub(super) fn spot_symbol_matches(symbol: &str, symbols: Option<&[String]>) -> bool {
    let Some((base, quote)) = crate::spot::delimited_pair(symbol, '_') else {
        return false;
    };
    crate::spot::symbol_matches(symbol, &base, &quote, symbols)
}

const GATE_DEPTH_LEVELS: &[u32] = &[1, 5, 10, 20, 50, 100];

pub(super) fn snap_gate_depth(depth: u32) -> u32 {
    GATE_DEPTH_LEVELS
        .iter()
        .copied()
        .min_by_key(|value| value.abs_diff(depth))
        .unwrap_or(20)
}

pub(super) fn parse_optional_f64(value: &str) -> Option<f64> {
    value.parse().ok().filter(|value: &f64| value.is_finite())
}

fn parse_positive(value: &str) -> Option<f64> {
    parse_optional_f64(value).filter(|value| *value > 0.0)
}

#[cfg(test)]
#[path = "gate_market_data_tests.rs"]
mod tests;
