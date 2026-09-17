//! KuCoin Pro mark-price and funding-fee payload parsing.

use common::time::now_ms;
use serde::Deserialize;

const CHANNEL_MARK_PRICE: &str = "mark-price";
const CHANNEL_FUNDING_FEE: &str = "funding-fee";
const HOUR_MS: i64 = 3_600_000;

#[derive(Debug, Clone)]
pub(super) struct CachedMarkIndex {
    pub(super) mark_price: f64,
    pub(super) index_price: Option<f64>,
    pub(super) open_interest: Option<f64>,
    pub(super) cached_at_ms: i64,
    pub(super) data_timestamp_ms: i64,
}

#[derive(Debug, Clone)]
pub(super) struct CachedFunding {
    pub(super) rate: f64,
    pub(super) interval_hours: u32,
    pub(super) next_funding_time_ms: i64,
    pub(super) cached_at_ms: i64,
    pub(super) data_timestamp_ms: i64,
}

#[derive(Debug)]
pub(super) struct ParsedMarkIndex {
    pub(super) kucoin_symbol: String,
    pub(super) cached: CachedMarkIndex,
}

#[derive(Debug)]
pub(super) struct ParsedFunding {
    pub(super) kucoin_symbol: String,
    pub(super) cached: CachedFunding,
}

pub(super) fn parse_mark_index_update(text: &str) -> Option<ParsedMarkIndex> {
    let envelope: ProEnvelope = serde_json::from_str(text).ok()?;
    if envelope.topic != CHANNEL_MARK_PRICE {
        return None;
    }
    let data = envelope.data?;
    let symbol = nonempty(data.symbol)?;
    let mark_price = positive(data.mark_price.as_deref()?)?;
    let index_price = data.index_price.as_deref().and_then(positive);
    let open_interest = data.open_interest.as_deref().and_then(nonnegative);
    let data_timestamp_ms = positive_timestamp_ms(data.timestamp_ms?)?;
    Some(ParsedMarkIndex {
        kucoin_symbol: symbol,
        cached: CachedMarkIndex {
            mark_price,
            index_price,
            open_interest,
            cached_at_ms: now_ms(),
            data_timestamp_ms,
        },
    })
}

pub(super) fn parse_funding_update(text: &str) -> Option<ParsedFunding> {
    let envelope: ProEnvelope = serde_json::from_str(text).ok()?;
    if envelope.topic != CHANNEL_FUNDING_FEE {
        return None;
    }
    let publish_timestamp = positive_timestamp_ms(envelope.publish_timestamp?)?;
    let data = envelope.data?;
    let symbol = nonempty(data.symbol)?;
    let rate = finite(data.funding_rate.as_deref()?)?;
    let interval_ms = data.granularity_ms.filter(|value| *value > 0)?;
    if interval_ms % HOUR_MS != 0 {
        return None;
    }
    let interval_hours = u32::try_from(interval_ms / HOUR_MS).ok()?;
    if interval_hours == 0 {
        return None;
    }
    let next_funding_time_ms = positive_timestamp_ms(data.next_funding_time_ms?)?;
    Some(ParsedFunding {
        kucoin_symbol: symbol,
        cached: CachedFunding {
            rate,
            interval_hours,
            next_funding_time_ms,
            cached_at_ms: now_ms(),
            data_timestamp_ms: publish_timestamp,
        },
    })
}

fn positive_timestamp_ms(value: i64) -> Option<i64> {
    if value <= 0 {
        return None;
    }
    Some(match value {
        value if value >= 100_000_000_000_000_000 => value / 1_000_000,
        value if value >= 100_000_000_000_000 => value / 1_000,
        value => value,
    })
}

fn nonempty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn finite(value: &str) -> Option<f64> {
    value.parse().ok().filter(|value: &f64| value.is_finite())
}

fn positive(value: &str) -> Option<f64> {
    finite(value).filter(|value| *value > f64::EPSILON)
}

fn nonnegative(value: &str) -> Option<f64> {
    finite(value).filter(|value| *value >= 0.0)
}

#[derive(Debug, Deserialize)]
struct ProEnvelope {
    #[serde(default, rename = "T")]
    topic: String,
    #[serde(default, rename = "P")]
    publish_timestamp: Option<i64>,
    #[serde(default, rename = "d")]
    data: Option<ProData>,
}

#[derive(Debug, Deserialize)]
struct ProData {
    #[serde(default, rename = "s")]
    symbol: String,
    #[serde(default, rename = "mp")]
    mark_price: Option<String>,
    #[serde(default, rename = "ip")]
    index_price: Option<String>,
    #[serde(default, rename = "oi")]
    open_interest: Option<String>,
    #[serde(default, rename = "ts")]
    timestamp_ms: Option<i64>,
    #[serde(default, rename = "fr")]
    funding_rate: Option<String>,
    #[serde(default, rename = "nt")]
    next_funding_time_ms: Option<i64>,
    #[serde(default, rename = "gl")]
    granularity_ms: Option<i64>,
}
