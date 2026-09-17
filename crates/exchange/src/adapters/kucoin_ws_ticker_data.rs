//! KuCoin Futures ticker WebSocket payload parsing.

use common::time::now_ms;
use serde::Deserialize;

pub(super) const SNAPSHOT_TOPIC_PREFIX: &str = "/contractMarket/snapshot";
pub(super) const TICKER_TOPIC_PREFIX: &str = "/contractMarket/tickerV2";

#[derive(Debug, Clone)]
pub(super) struct CachedSnapshot {
    pub(super) last_price: f64,
    pub(super) volume_24h_quote: f64,
    pub(super) cached_at_ms: i64,
    pub(super) data_timestamp_ms: i64,
}

#[derive(Debug, Clone)]
pub(super) struct CachedBookTicker {
    pub(super) bid: f64,
    pub(super) ask: f64,
    pub(super) cached_at_ms: i64,
    pub(super) data_timestamp_ms: i64,
}

#[derive(Debug)]
pub(super) struct ParsedSnapshot {
    pub(super) kucoin_symbol: String,
    pub(super) cached: CachedSnapshot,
}

pub(super) fn parse_snapshot(text: &str) -> Option<ParsedSnapshot> {
    let envelope: SnapshotEnvelope = serde_json::from_str(text).ok()?;
    if !envelope.topic.starts_with(SNAPSHOT_TOPIC_PREFIX) {
        return None;
    }
    let kucoin_symbol = envelope.topic.split_once(':')?.1.to_owned();
    let data = envelope.data?;
    let last_price = finite(data.last_price?)?;
    let volume_24h_quote = finite(data.turnover?)?;
    let now = now_ms();
    let data_timestamp_ms = ns_to_ms(data.ts)?;
    Some(ParsedSnapshot {
        kucoin_symbol,
        cached: CachedSnapshot {
            last_price,
            volume_24h_quote,
            cached_at_ms: now,
            data_timestamp_ms,
        },
    })
}

#[derive(Debug)]
pub(super) struct ParsedTickerV2 {
    pub(super) kucoin_symbol: String,
    pub(super) cached: CachedBookTicker,
}

pub(super) fn parse_ticker_v2(text: &str) -> Option<ParsedTickerV2> {
    let envelope: TickerV2Envelope = serde_json::from_str(text).ok()?;
    if !envelope.topic.starts_with(TICKER_TOPIC_PREFIX) {
        return None;
    }
    let kucoin_symbol = envelope.topic.split_once(':')?.1.to_owned();
    let data = envelope.data?;
    let bid = parse_f64(data.best_bid_price.as_deref()?)?;
    let ask = parse_f64(data.best_ask_price.as_deref()?)?;
    let now = now_ms();
    let data_timestamp_ms = ns_to_ms(data.ts)?;
    Some(ParsedTickerV2 {
        kucoin_symbol,
        cached: CachedBookTicker {
            bid,
            ask,
            cached_at_ms: now,
            data_timestamp_ms,
        },
    })
}

fn parse_f64(value: &str) -> Option<f64> {
    value.parse().ok().filter(|v: &f64| v.is_finite())
}

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

/// KuCoin futures public ws timestamps are emitted in **nanoseconds**.
/// Convert to ms; return `None` when the field is missing/zero.
pub(super) fn ns_to_ms(ns: Option<i64>) -> Option<i64> {
    let ns = ns?;
    if ns <= 0 {
        return None;
    }
    Some(ns / 1_000_000)
}

#[derive(Debug, Deserialize)]
struct SnapshotEnvelope {
    #[serde(default)]
    topic: String,
    #[serde(default)]
    data: Option<SnapshotData>,
}

#[derive(Debug, Deserialize)]
struct SnapshotData {
    #[serde(default, rename = "lastPrice")]
    last_price: Option<f64>,
    /// 24h notional turnover (USDT for `*USDTM`).
    #[serde(default)]
    turnover: Option<f64>,
    #[serde(default)]
    ts: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct TickerV2Envelope {
    #[serde(default)]
    topic: String,
    #[serde(default)]
    data: Option<TickerV2Data>,
}

#[derive(Debug, Deserialize)]
struct TickerV2Data {
    #[serde(default, rename = "bestBidPrice")]
    best_bid_price: Option<String>,
    #[serde(default, rename = "bestAskPrice")]
    best_ask_price: Option<String>,
    #[serde(default)]
    ts: Option<i64>,
}
