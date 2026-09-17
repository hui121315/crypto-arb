//! Bitget V3 / UTA public market response parsing.
//!
//! V3 differs from V2 in four structural ways the DTOs below pin (sample
//! payloads pulled live from `api.bitget.com` at PR-DP-13 · B-2 time):
//! - `ticker` payload exposes `lastPrice / bid1Price / ask1Price / bid1Size / ask1Size`
//!   instead of V2 `lastPr / bidPr / askPr / bidSz / askSz`.
//! - 24h volume uses `volume24h` (base) + `turnover24h` (quote) instead of
//!   V2 `baseVolume` / `quoteVolume`. The quote-volume field name is
//!   `turnover24h`, **not** `quoteVolume24h` (the V2 "hint" did not survive).
//! - `orderbook` payload uses short keys `a` / `b` with numeric levels
//!   (`[price, size]` as floats) instead of V2 `asks` / `bids` with
//!   string-encoded levels.
//! - `current-fund-rate` exposes `nextUpdate` + `fundingRateInterval` on every
//!   row; the REST `tickers` payload itself already carries
//!   `fundingRate / markPrice / indexPrice / openInterest`, which means a
//!   future B-3 WS consumer can stitch funding directly from `topic: ticker`
//!   and only fall back to `current-fund-rate` for the interval.
//!
//! Sources:
//! - Instruments: <https://www.bitget.com/api-doc/uta/public/Instruments>
//! - Tickers: <https://www.bitget.com/api-doc/uta/public/Tickers>
//! - Current funding rate: <https://www.bitget.com/api-doc/uta/public/Get-Current-Funding-Rate>
//! - `OrderBook`: <https://www.bitget.com/api-doc/uta/public/OrderBook>
//! - Server time: <https://www.bitget.com/api-doc/common/public/Get-Server-Time>

use super::bitget_market_data::{parse_levels, valid_bitget_funding_interval};
use crate::adapter::strip_common_suffixes;
use common::time::now_ms;
use serde::{Deserialize, Deserializer};
use shared_types::{
    FundingRateData, IndexComponent, IndexCompositionQuality, IndexCompositionSnapshot,
    MarkIndexInfo, SpotTick, TickerInfo,
};

const NAME: &str = "bitget";

// -- DTOs -------------------------------------------------------------------

/// V3 `/api/v3/market/current-fund-rate` row.
#[derive(Debug, Deserialize)]
pub(super) struct UtaFundingRateItem {
    pub(super) symbol: String,
    #[serde(default, rename = "fundingRate")]
    pub(super) funding_rate: String,
    #[serde(default, rename = "nextUpdate")]
    pub(super) next_update: String,
    #[serde(default, rename = "fundingRateInterval")]
    pub(super) funding_rate_interval: String,
}

/// V3 `/api/v3/market/instruments` row.
#[derive(Debug, Deserialize)]
pub(super) struct UtaInstrumentItem {
    pub(super) symbol: String,
    #[serde(default, rename = "fundInterval")]
    pub(super) fund_interval: String,
}

/// V3 `/api/v3/market/tickers` row.
///
/// Field naming follows the UTA public ticker response (see module docs).
/// All numeric fields are returned as strings by Bitget and parsed lazily by
/// the helpers below.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct UtaTickerItem {
    pub(super) symbol: String,
    #[serde(
        default,
        rename = "lastPrice",
        deserialize_with = "deserialize_string_or_default"
    )]
    pub(super) last_price: String,
    #[serde(
        default,
        rename = "bid1Price",
        deserialize_with = "deserialize_string_or_default"
    )]
    pub(super) bid1_price: String,
    #[serde(
        default,
        rename = "ask1Price",
        deserialize_with = "deserialize_string_or_default"
    )]
    pub(super) ask1_price: String,
    #[serde(
        default,
        rename = "bid1Size",
        deserialize_with = "deserialize_string_or_default"
    )]
    pub(super) bid1_size: String,
    #[serde(
        default,
        rename = "ask1Size",
        deserialize_with = "deserialize_string_or_default"
    )]
    pub(super) ask1_size: String,
    /// 24h traded base volume in contract / base coin units.
    #[serde(
        default,
        rename = "volume24h",
        deserialize_with = "deserialize_string_or_default"
    )]
    pub(super) volume_24h: String,
    /// 24h traded quote volume (USDT for `USDT-FUTURES`). V3 calls this
    /// `turnover24h`; we keep the Rust field aligned with the semantic name
    /// so downstream callers stay readable.
    #[serde(
        default,
        rename = "turnover24h",
        deserialize_with = "deserialize_string_or_default"
    )]
    pub(super) quote_volume_24h: String,
    #[serde(
        default,
        rename = "markPrice",
        deserialize_with = "deserialize_string_or_default"
    )]
    pub(super) mark_price: String,
    #[serde(
        default,
        rename = "indexPrice",
        deserialize_with = "deserialize_string_or_default"
    )]
    pub(super) index_price: String,
    #[serde(
        default,
        rename = "openInterest",
        deserialize_with = "deserialize_string_or_default"
    )]
    pub(super) open_interest: String,
    #[serde(
        default,
        rename = "fundingRate",
        deserialize_with = "deserialize_string_or_default"
    )]
    pub(super) funding_rate: String,
    #[serde(
        default,
        rename = "nextFundingTime",
        deserialize_with = "deserialize_string_or_default"
    )]
    pub(super) next_funding_time: String,
    /// Server timestamp in millis (string-encoded).
    #[serde(default, deserialize_with = "deserialize_string_or_default")]
    pub(super) ts: String,
}

fn deserialize_string_or_default<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

/// V3 `/api/v3/market/orderbook` payload.
///
/// V3 uses the short `a` / `b` keys and emits numeric levels rather than the
/// V2 string-encoded `asks` / `bids`. `deserialize_depth_levels` already
/// handles both numeric and string-encoded inner values, so the same parser
/// is reused; only the field tags need to match the on-the-wire names.
#[derive(Debug, Deserialize)]
pub(super) struct UtaDepthItem {
    #[serde(default, rename = "b", deserialize_with = "deserialize_depth_levels")]
    pub(super) bids: Vec<[f64; 2]>,
    #[serde(default, rename = "a", deserialize_with = "deserialize_depth_levels")]
    pub(super) asks: Vec<[f64; 2]>,
    #[serde(default)]
    pub(super) ts: String,
}

/// V3 `/api/v3/market/index-components` payload.
#[derive(Debug, Deserialize)]
pub(super) struct UtaIndexComponentsData {
    pub(super) symbol: String,
    #[serde(default, rename = "componentList")]
    pub(super) component_list: Vec<UtaIndexComponentItem>,
}

/// V3 `/api/v3/market/index-components` component row.
#[derive(Debug, Deserialize)]
pub(super) struct UtaIndexComponentItem {
    #[serde(default)]
    pub(super) exchange: String,
    #[serde(default, rename = "spotPair")]
    pub(super) spot_pair: String,
    #[serde(default, rename = "equivalentPrice")]
    pub(super) equivalent_price: String,
    #[serde(default)]
    pub(super) weight: String,
}

fn deserialize_depth_levels<'de, D>(deserializer: D) -> Result<Vec<[f64; 2]>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Vec::<Vec<serde_json::Value>>::deserialize(deserializer)?;
    Ok(raw
        .iter()
        .map(Vec::as_slice)
        .filter_map(parse_depth_level)
        .collect())
}

fn parse_depth_level(level: &[serde_json::Value]) -> Option<[f64; 2]> {
    Some([
        parse_depth_value(level.first()?)?,
        parse_depth_value(level.get(1)?)?,
    ])
}

fn parse_depth_value(value: &serde_json::Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

// -- Parse helpers ----------------------------------------------------------

/// Parse a Bitget UTA ticker, failing closed when a required price is absent.
///
/// `bid1Price`/`ask1Price`/`lastPrice` drive execution and risk decisions, so a
/// missing/zero/unparseable value yields `None` (the tick is dropped) instead of
/// a fabricated `0.0` price that downstream scanners would treat as a real quote.
/// `turnover24h`/`volume24h` stay best-effort.
pub(super) fn parse_ticker(item: &UtaTickerItem) -> Option<TickerInfo> {
    let bid = parse_positive(&item.bid1_price)?;
    let ask = parse_positive(&item.ask1_price)?;
    let last = parse_positive(&item.last_price)?;
    Some(TickerInfo {
        symbol: strip_common_suffixes(&item.symbol),
        exchange: NAME.into(),
        bid,
        ask,
        last,
        // `quoteVolume24h` mirrors V2 `quoteVolume`; if the venue stops
        // populating it we fall back to base-volume so downstream code still
        // sees a non-zero turnover. `parse_f64` returns 0.0 on empty.
        volume_24h: parse_quote_volume(item),
        timestamp: item.ts.parse().unwrap_or_else(|_| now_ms()),
    })
}

pub(super) fn parse_mark_index(item: &UtaTickerItem) -> Option<MarkIndexInfo> {
    Some(MarkIndexInfo {
        symbol: strip_common_suffixes(&item.symbol),
        exchange: NAME.into(),
        mark_price: parse_positive(&item.mark_price)?,
        index_price: parse_optional_f64(&item.index_price),
        open_interest: parse_optional_f64(&item.open_interest),
        open_interest_value: None,
        timestamp: item.ts.parse().unwrap_or_else(|_| now_ms()),
    })
}

pub(super) fn parse_funding(item: &UtaFundingRateItem, volume_24h: f64) -> Option<FundingRateData> {
    let rate = parse_optional_f64(&item.funding_rate)?;
    let next_time = parse_positive_i64(&item.next_update)?;
    let interval_hours = valid_bitget_funding_interval(item.funding_rate_interval.parse().ok())?;
    let volume_24h = finite_nonnegative(volume_24h)?;
    Some(FundingRateData {
        symbol: strip_common_suffixes(&item.symbol),
        exchange: NAME.into(),
        rate,
        rate_8h: rate * (8.0 / f64::from(interval_hours)),
        predicted_rate: None,
        next_funding_time: next_time,
        funding_interval: interval_hours,
        volume_24h,
        timestamp: now_ms(),
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    })
}

pub(super) fn parse_funding_from_ticker(
    item: &UtaTickerItem,
    interval_hours: u32,
) -> Option<FundingRateData> {
    let rate = parse_optional_f64(&item.funding_rate)?;
    let interval_hours = valid_bitget_funding_interval(Some(interval_hours))?;
    let timestamp = parse_positive_i64(&item.ts)?;
    let next_funding_time = parse_positive_i64(&item.next_funding_time)
        .or_else(|| next_interval_boundary(timestamp, interval_hours))?;
    Some(FundingRateData {
        symbol: strip_common_suffixes(&item.symbol),
        exchange: NAME.into(),
        rate,
        rate_8h: rate * (8.0 / f64::from(interval_hours)),
        predicted_rate: None,
        next_funding_time,
        funding_interval: interval_hours,
        volume_24h: parse_quote_volume(item),
        timestamp,
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    })
}

fn next_interval_boundary(timestamp_ms: i64, interval_hours: u32) -> Option<i64> {
    let interval_ms = i64::from(interval_hours).checked_mul(3_600_000)?;
    timestamp_ms
        .checked_div(interval_ms)?
        .checked_add(1)?
        .checked_mul(interval_ms)
}

pub(super) fn instrument_funding_interval(item: &UtaInstrumentItem) -> (String, Option<u32>) {
    let interval = item.fund_interval.parse().ok();
    (item.symbol.clone(), valid_bitget_funding_interval(interval))
}

pub(super) fn parse_spot_tick(item: &UtaTickerItem) -> Option<SpotTick> {
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
        volume_24h: &item.quote_volume_24h,
        exchange_ts_ms: crate::spot::parse_millis_opt(&item.ts),
    })
}

/// `(bids, asks, timestamp_ms)` produced by [`finalize_orderbook_levels`].
pub(super) type FinalizedOrderbook = (Vec<[f64; 2]>, Vec<[f64; 2]>, i64);

pub(super) fn finalize_orderbook_levels(item: &UtaDepthItem) -> FinalizedOrderbook {
    let timestamp = item.ts.parse().unwrap_or_else(|_| now_ms());
    (
        parse_levels(&item.bids),
        parse_levels(&item.asks),
        timestamp,
    )
}

pub(super) fn parse_index_components(item: UtaIndexComponentsData) -> IndexCompositionSnapshot {
    IndexCompositionSnapshot {
        venue: NAME.to_owned(),
        symbol: strip_common_suffixes(&item.symbol),
        index_id: item.symbol,
        components: item
            .component_list
            .into_iter()
            .map(index_component)
            .collect(),
        quality: IndexCompositionQuality::Verified,
        source: "GET /api/v3/market/index-components".to_owned(),
        received_at_ms: now_ms(),
        freshness_ms: Some(0),
        error: None,
        retry_after_ms: None,
        source_url: None,
        payload_sha256: None,
        schema_version: Some("bitget-v3-index-components/1".to_owned()),
    }
}

fn index_component(item: UtaIndexComponentItem) -> IndexComponent {
    IndexComponent {
        symbol: item.spot_pair,
        name: item.exchange,
        weight: parse_f64(&item.weight),
        price: parse_positive(&item.equivalent_price),
    }
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

fn parse_positive_i64(value: &str) -> Option<i64> {
    value.parse().ok().filter(|value| *value > 0)
}

fn finite_nonnegative(value: f64) -> Option<f64> {
    (value.is_finite() && value >= 0.0).then_some(value)
}

fn parse_quote_volume(item: &UtaTickerItem) -> f64 {
    let quote = parse_f64(&item.quote_volume_24h);
    if quote > 0.0 {
        return quote;
    }
    parse_f64(&item.volume_24h)
}

#[cfg(test)]
#[path = "bitget_uta_market_data_tests.rs"]
mod tests;
