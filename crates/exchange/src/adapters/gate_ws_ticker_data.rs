use super::gate_market_data::{parse_optional_f64, parse_quote_volume, TickerItem};
use crate::adapter::strip_common_suffixes;
use common::time::{now_ms, now_secs};
use serde::Deserialize;
use serde_json::json;
use shared_types::TickerInfo;

pub(super) const EXCHANGE: &str = "gate";
pub(super) const TICKERS_CHANNEL: &str = "futures.tickers";
pub(super) const BOOK_TICKER_CHANNEL: &str = "futures.book_ticker";
pub(super) const ORDER_BOOK_CHANNEL: &str = "futures.order_book";

#[derive(Debug, Clone)]
pub(super) struct CachedMarket {
    pub(super) item: TickerItem,
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

pub(super) fn stream_symbol(symbol: &str) -> String {
    let upper = strip_common_suffixes(symbol);
    format!("{upper}_USDT")
}

pub(super) fn channel_payload(op: &str, channel: &str, symbols: &[String]) -> String {
    let payload: Vec<_> = symbols.iter().map(|symbol| stream_symbol(symbol)).collect();
    json!({
        "time": now_secs(),
        "channel": channel,
        "event": op,
        "payload": payload
    })
    .to_string()
}

pub(super) fn order_book_payload(op: &str, symbol: &str) -> String {
    json!({
        "time": now_secs(),
        "channel": ORDER_BOOK_CHANNEL,
        "event": op,
        "payload": [stream_symbol(symbol), "1", "0"]
    })
    .to_string()
}

pub(super) fn to_ticker(market: &CachedMarket, book: &CachedBookTicker) -> Option<TickerInfo> {
    // Best bid/ask arrive on the dedicated book-ticker channel; the market
    // channel only carries `last`, so fail closed on an unreadable last price
    // rather than emitting a fabricated `0.0` quote.
    let last = parse_optional_f64(&market.item.last)?;
    Some(TickerInfo {
        symbol: strip_common_suffixes(&market.item.contract),
        exchange: EXCHANGE.into(),
        bid: book.bid,
        ask: book.ask,
        last,
        volume_24h: parse_quote_volume(&market.item),
        timestamp: market.data_timestamp_ms.max(book.data_timestamp_ms),
    })
}

pub(super) fn parse_market_updates(text: &str) -> Vec<CachedMarket> {
    let Ok(envelope) = serde_json::from_str::<GateEnvelope<Vec<TickerItem>>>(text) else {
        return Vec::new();
    };
    if envelope.channel != TICKERS_CHANNEL || envelope.event != "update" {
        return Vec::new();
    }
    let now = now_ms();
    envelope
        .result
        .unwrap_or_default()
        .into_iter()
        .map(|item| CachedMarket {
            data_timestamp_ms: envelope.time.map(|seconds| seconds * 1000).unwrap_or(now),
            cached_at_ms: now,
            item,
        })
        .collect()
}

pub(super) fn parse_book_update(text: &str) -> Option<(String, CachedBookTicker)> {
    let envelope: GateEnvelope<BookTickerItem> = serde_json::from_str(text).ok()?;
    if envelope.channel != BOOK_TICKER_CHANNEL || envelope.event != "update" {
        return None;
    }
    let now = now_ms();
    let item = envelope.result?;
    let row = CachedBookTicker {
        bid: parse_f64(&item.bid)?,
        ask: parse_f64(&item.ask)?,
        cached_at_ms: now,
        data_timestamp_ms: item
            .timestamp_ms
            .unwrap_or_else(|| envelope.time.map(|seconds| seconds * 1000).unwrap_or(now)),
    };
    Some((item.contract, row))
}

fn parse_f64(value: &str) -> Option<f64> {
    value.parse().ok().filter(|value: &f64| value.is_finite())
}

#[derive(Debug, Deserialize)]
struct GateEnvelope<T> {
    channel: String,
    event: String,
    time: Option<i64>,
    result: Option<T>,
}

#[derive(Debug, Deserialize)]
struct BookTickerItem {
    #[serde(rename = "s")]
    contract: String,
    #[serde(default, rename = "b")]
    bid: String,
    #[serde(default, rename = "a")]
    ask: String,
    #[serde(default, rename = "t")]
    timestamp_ms: Option<i64>,
}
