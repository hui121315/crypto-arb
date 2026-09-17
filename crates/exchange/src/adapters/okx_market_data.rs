//! OKX public market response parsing.
//!
//! Official OKX V5 docs checked before moving these DTOs:
//! - GET /api/v5/public/instruments
//! - GET /api/v5/market/tickers
//! - GET /api/v5/market/books

use crate::adapter::strip_common_suffixes;
use common::time::now_ms;
use serde::Deserialize;
use shared_types::{
    IndexComponent, IndexCompositionQuality, IndexCompositionSnapshot, OrderBookInfo, SpotTick,
    TickerInfo,
};
use std::collections::HashMap;

const NAME: &str = "okx";

#[derive(Debug, Deserialize)]
pub(super) struct InstrumentItem {
    #[serde(rename = "instId")]
    inst_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct TickerItem {
    #[serde(default, rename = "instType")]
    inst_type: String,
    #[serde(rename = "instId")]
    inst_id: String,
    #[serde(default)]
    last: String,
    #[serde(default, rename = "bidPx")]
    bid_px: String,
    #[serde(default, rename = "askPx")]
    ask_px: String,
    #[serde(default, rename = "bidSz")]
    bid_sz: String,
    #[serde(default, rename = "askSz")]
    ask_sz: String,
    #[serde(default, rename = "volCcy24h")]
    vol_ccy_24h: String,
    #[serde(default)]
    ts: String,
}

impl TickerItem {
    pub(super) fn inst_type(&self) -> Option<&str> {
        (!self.inst_type.is_empty()).then_some(self.inst_type.as_str())
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct OrderBookItem {
    #[serde(default)]
    bids: Vec<Vec<String>>,
    #[serde(default)]
    asks: Vec<Vec<String>>,
    #[serde(default)]
    ts: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct IndexComponentsItem {
    #[serde(default)]
    index: String,
    #[serde(default)]
    components: Vec<IndexComponentItem>,
    #[serde(default)]
    ts: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct IndexComponentItem {
    #[serde(default)]
    symbol: String,
    #[serde(default, rename = "symPx")]
    symbol_price: String,
    #[serde(default, rename = "wgt")]
    weight: String,
    #[serde(default, rename = "exch")]
    exchange: String,
}

pub(super) fn swap_inst_ids(items: Vec<InstrumentItem>) -> Vec<String> {
    items
        .into_iter()
        .filter(|item| item.inst_id.ends_with("-USDT-SWAP"))
        .map(|item| item.inst_id)
        .collect()
}

pub(super) fn funding_volume_map(tickers: Vec<TickerItem>) -> HashMap<String, f64> {
    tickers
        .into_iter()
        .filter_map(|ticker| {
            ticker
                .vol_ccy_24h
                .parse::<f64>()
                .ok()
                .map(|volume| (ticker.inst_id, volume))
        })
        .collect()
}

/// Parse an OKX ticker, failing closed when a required price cannot be read.
///
/// `bidPx`/`askPx`/`last` drive execution and risk decisions, so an empty or
/// unparseable value yields `None` (the tick is dropped) instead of a fabricated
/// `0.0` price that downstream scanners would treat as a real quote.
/// `volCcy24h` stays best-effort because it does not gate execution.
pub(super) fn parse_ticker(ticker: &TickerItem) -> Option<TickerInfo> {
    let bid = ticker.bid_px.trim().parse().ok()?;
    let ask = ticker.ask_px.trim().parse().ok()?;
    let last = ticker.last.trim().parse().ok()?;
    Some(TickerInfo {
        symbol: strip_common_suffixes(&ticker.inst_id),
        exchange: NAME.into(),
        bid,
        ask,
        last,
        volume_24h: ticker.vol_ccy_24h.parse().unwrap_or(0.0),
        timestamp: ticker.ts.parse().unwrap_or_else(|_| now_ms()),
    })
}

pub(super) fn parse_spot_tick(ticker: &TickerItem) -> Option<SpotTick> {
    let (base, quote) = crate::spot::delimited_pair(&ticker.inst_id, '-')?;
    crate::spot::spot_tick(crate::spot::SpotFields {
        venue: NAME,
        base: &base,
        quote: &quote,
        bid: &ticker.bid_px,
        ask: &ticker.ask_px,
        last: &ticker.last,
        bid_size: Some(&ticker.bid_sz),
        ask_size: Some(&ticker.ask_sz),
        volume_24h: &ticker.vol_ccy_24h,
        exchange_ts_ms: crate::spot::parse_millis_opt(&ticker.ts),
    })
}

pub(super) fn usdt_swap_ticker(ticker: &TickerItem) -> bool {
    ticker.inst_id.ends_with("-USDT-SWAP")
}

pub(super) fn spot_ticker_matches(ticker: &TickerItem, symbols: Option<&[String]>) -> bool {
    spot_symbol_matches(&ticker.inst_id, symbols)
}

pub(super) fn spot_symbol_matches(inst_id: &str, symbols: Option<&[String]>) -> bool {
    let Some((base, quote)) = crate::spot::delimited_pair(inst_id, '-') else {
        return false;
    };
    crate::spot::symbol_matches(inst_id, &base, &quote, symbols)
}

pub(super) fn orderbook_info(symbol: String, item: OrderBookItem) -> OrderBookInfo {
    OrderBookInfo {
        symbol,
        exchange: NAME.into(),
        bids: parse_levels(item.bids),
        asks: parse_levels(item.asks),
        timestamp: item.ts.parse().unwrap_or_else(|_| now_ms()),
    }
}

pub(super) fn parse_index_components(item: IndexComponentsItem) -> IndexCompositionSnapshot {
    let received_at_ms = item.ts.parse().unwrap_or_else(|_| now_ms());
    IndexCompositionSnapshot {
        venue: NAME.to_owned(),
        symbol: strip_common_suffixes(&item.index),
        index_id: item.index,
        components: item.components.into_iter().map(index_component).collect(),
        quality: IndexCompositionQuality::Verified,
        source: "GET /api/v5/market/index-components".to_owned(),
        received_at_ms,
        freshness_ms: Some(0),
        error: None,
        retry_after_ms: None,
        source_url: None,
        payload_sha256: None,
        schema_version: Some("okx-v5-index-components/1".to_owned()),
    }
}

fn index_component(item: IndexComponentItem) -> IndexComponent {
    IndexComponent {
        symbol: item.symbol,
        name: item.exchange,
        weight: parse_f64(&item.weight),
        price: parse_positive(&item.symbol_price),
    }
}

fn parse_levels(levels: Vec<Vec<String>>) -> Vec<[f64; 2]> {
    // OKX orderbook rows are [price, size, liquidated_orders, num_orders].
    levels
        .into_iter()
        .filter_map(|row| {
            let price = row.first()?.parse::<f64>().ok()?;
            let size = row.get(1)?.parse::<f64>().ok()?;
            Some([price, size])
        })
        .collect()
}

fn parse_f64(value: &str) -> f64 {
    value.parse().unwrap_or(0.0)
}

fn parse_positive(value: &str) -> Option<f64> {
    let value = parse_f64(value);
    (value.is_finite() && value > f64::EPSILON).then_some(value)
}

#[cfg(test)]
#[path = "okx_market_data_tests.rs"]
mod tests;
