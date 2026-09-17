use super::bybit_market_data::{
    linear_stream_symbol, parse_ticker, parse_ws_funding, MarketTickerItem,
};
use common::time::now_ms;
use serde::Deserialize;
use serde_json::json;
use shared_types::{FundingRateData, TickerInfo};

#[derive(Debug, Clone)]
pub(super) struct CachedTicker {
    pub(super) item: MarketTickerItem,
    pub(super) cached_at_ms: i64,
    pub(super) data_timestamp_ms: i64,
}

#[derive(Debug)]
pub(super) struct ParsedTickerUpdate {
    pub(super) stream_symbol: String,
    pub(super) item: MarketTickerItem,
    pub(super) timestamp_ms: i64,
    pub(super) is_delta: bool,
}

pub(super) fn stream_symbol(symbol: &str) -> String {
    linear_stream_symbol(symbol)
}

pub(super) fn channel_payload(op: &str, symbols: &[String]) -> String {
    let args: Vec<_> = symbols
        .iter()
        .map(|symbol| format!("tickers.{}", stream_symbol(symbol)))
        .collect();
    json!({ "op": op, "args": args }).to_string()
}

pub(super) fn is_complete(item: &MarketTickerItem) -> bool {
    !item.symbol.is_empty()
        && !item.last_price.is_empty()
        && !item.bid1_price.is_empty()
        && !item.ask1_price.is_empty()
}

pub(super) fn to_ticker(cached: &CachedTicker) -> Option<TickerInfo> {
    let mut ticker = parse_ticker(&cached.item)?;
    ticker.timestamp = cached.data_timestamp_ms;
    Some(ticker)
}

pub(super) fn to_funding(cached: &CachedTicker) -> Option<FundingRateData> {
    let volume_24h = cached.item.turnover24h.parse().unwrap_or(0.0);
    parse_ws_funding(&cached.item, volume_24h, cached.data_timestamp_ms)
}

pub(super) fn merge_item(target: &mut MarketTickerItem, delta: MarketTickerItem) {
    merge_text(&mut target.last_price, delta.last_price);
    merge_text(&mut target.bid1_price, delta.bid1_price);
    merge_text(&mut target.bid1_size, delta.bid1_size);
    merge_text(&mut target.ask1_price, delta.ask1_price);
    merge_text(&mut target.ask1_size, delta.ask1_size);
    merge_text(&mut target.turnover24h, delta.turnover24h);
    merge_text(&mut target.funding_rate, delta.funding_rate);
    merge_text(&mut target.next_funding_time, delta.next_funding_time);
    merge_text(
        &mut target.funding_interval_hour,
        delta.funding_interval_hour,
    );
    merge_text(&mut target.mark_price, delta.mark_price);
    merge_text(&mut target.index_price, delta.index_price);
    merge_text(&mut target.open_interest, delta.open_interest);
    merge_text(&mut target.open_interest_value, delta.open_interest_value);
}

pub(super) fn parse_ticker_update(text: &str) -> Option<ParsedTickerUpdate> {
    let envelope: TickerEnvelope = serde_json::from_str(text).ok()?;
    if !envelope.topic.starts_with("tickers.") {
        return None;
    }
    let item = envelope.data.into_first()?;
    Some(ParsedTickerUpdate {
        stream_symbol: stream_symbol(&item.symbol),
        item,
        timestamp_ms: envelope.ts.unwrap_or_else(now_ms),
        is_delta: envelope.kind == "delta",
    })
}

fn merge_text(target: &mut String, update: String) {
    if !update.is_empty() {
        *target = update;
    }
}

#[derive(Debug, Deserialize)]
struct TickerEnvelope {
    topic: String,
    #[serde(rename = "type")]
    kind: String,
    data: TickerPayload,
    ts: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TickerPayload {
    One(Box<MarketTickerItem>),
    Many(Vec<MarketTickerItem>),
}

impl TickerPayload {
    fn into_first(self) -> Option<MarketTickerItem> {
        match self {
            Self::One(item) => Some(*item),
            Self::Many(items) => items.into_iter().next(),
        }
    }
}
