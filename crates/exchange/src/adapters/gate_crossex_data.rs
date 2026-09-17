//! Gate `CrossEx` public WebSocket and instrument parsers.

use super::gate_crossex_symbols::{CrossExBusiness, CrossExRoute};
use crate::error::{ExchangeError, ExchangeResult};
use common::time::now_ms;
use rust_decimal::Decimal;
use serde::Deserialize;
use shared_types::{
    FundingRateData, InstrumentAssetClass, InstrumentListingStatus, InstrumentMetadataSource,
    MarkIndexInfo, OrderBookInfo, SpotTick, TickerInfo, VenueInstrument,
};

pub(super) const SYMBOLS_SOURCE_URL: &str = "https://api.gateio.ws/api/v4/crossex/rule/symbols";

#[derive(Debug, Clone)]
pub(super) struct CrossExTickerUpdate {
    pub(super) route: CrossExRoute,
    pub(super) ticker: TickerInfo,
    pub(super) spot: Option<SpotTick>,
}

#[derive(Debug, Clone)]
pub(super) struct CrossExFundingUpdate {
    pub(super) route: CrossExRoute,
    pub(super) rate: f64,
    pub(super) next_funding_time: i64,
    pub(super) timestamp: i64,
}

impl CrossExFundingUpdate {
    pub(super) fn into_row(self, funding_interval: u32, volume_24h: f64) -> FundingRateData {
        let exchange = self.route.venue();
        FundingRateData {
            symbol: self.route.base,
            exchange,
            rate: self.rate,
            // Compatibility field only: CrossEx publishes the native event rate.
            // Multiplying to an 8h value would alter the settlement economics.
            rate_8h: self.rate,
            predicted_rate: None,
            next_funding_time: self.next_funding_time,
            funding_interval,
            volume_24h,
            timestamp: self.timestamp,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) enum CrossExReferenceUpdate {
    Mark {
        route: CrossExRoute,
        value: f64,
        timestamp: i64,
    },
    Index {
        route: CrossExRoute,
        value: f64,
        timestamp: i64,
    },
    OpenInterest {
        route: CrossExRoute,
        quantity: f64,
        value: Option<f64>,
        timestamp: i64,
    },
}

#[derive(Debug, Clone)]
pub(super) struct CrossExBookSnapshot {
    pub(super) route: CrossExRoute,
    pub(super) bids: Vec<[Decimal; 2]>,
    pub(super) asks: Vec<[Decimal; 2]>,
    pub(super) timestamp: i64,
}

impl CrossExBookSnapshot {
    pub(super) fn to_orderbook(&self, depth: usize, quantity_multiplier: Decimal) -> OrderBookInfo {
        let convert = |levels: &[[Decimal; 2]]| {
            levels
                .iter()
                .take(depth.max(1))
                .filter_map(|[price, quantity]| {
                    Some([
                        price.to_string().parse().ok()?,
                        (*quantity * quantity_multiplier).to_string().parse().ok()?,
                    ])
                })
                .collect()
        };
        OrderBookInfo {
            symbol: self.route.base.clone(),
            exchange: self.route.venue(),
            bids: convert(&self.bids),
            asks: convert(&self.asks),
            timestamp: self.timestamp,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct CrossExBookDelta {
    pub(super) route: CrossExRoute,
    pub(super) snapshot: bool,
    pub(super) sequence_start: u64,
    pub(super) sequence_end: u64,
    pub(super) bids: Vec<[Decimal; 2]>,
    pub(super) asks: Vec<[Decimal; 2]>,
    pub(super) timestamp: i64,
}

pub(super) fn parse_ticker_frame(text: &str) -> ExchangeResult<Option<CrossExTickerUpdate>> {
    let frame: Envelope<TickerPayload> = parse_update(text, "ticker")?;
    let Some(row) = frame.result else {
        return Ok(None);
    };
    let route = CrossExRoute::parse(&row.symbol)?;
    let bid = decimal(&row.bid, "ticker.bp")?;
    let ask = decimal(&row.ask, "ticker.ap")?;
    let last = decimal(&row.last, "ticker.lp")?;
    let volume = decimal(&row.volume, "ticker.v")?;
    let timestamp = row.timestamp;
    let exchange = route.venue();
    let ticker = TickerInfo {
        symbol: route.base.clone(),
        exchange: exchange.clone(),
        bid: decimal_f64(bid, "ticker.bp")?,
        ask: decimal_f64(ask, "ticker.ap")?,
        last: decimal_f64(last, "ticker.lp")?,
        volume_24h: decimal_f64(volume, "ticker.v")?,
        timestamp,
    };
    let spot = (route.business == CrossExBusiness::Spot).then(|| SpotTick {
        venue: exchange,
        symbol: route.base.clone(),
        bid,
        ask,
        last,
        bid_size: optional_decimal(row.bid_size.as_deref()),
        ask_size: optional_decimal(row.ask_size.as_deref()),
        volume_24h: volume,
        exchange_ts_ms: Some(timestamp),
        received_at_ms: now_ms(),
    });
    Ok(Some(CrossExTickerUpdate {
        route,
        ticker,
        spot,
    }))
}

pub(super) fn parse_funding_frame(text: &str) -> ExchangeResult<Option<CrossExFundingUpdate>> {
    let frame: Envelope<FundingPayload> = parse_update(text, "funding_rate")?;
    let Some(row) = frame.result else {
        return Ok(None);
    };
    let route = CrossExRoute::parse(&row.symbol)?;
    if route.business != CrossExBusiness::Future {
        return Err(ExchangeError::Parse(format!(
            "CrossEx funding frame used non-future route {}",
            route.native_symbol
        )));
    }
    Ok(Some(CrossExFundingUpdate {
        route,
        rate: parse_f64(&row.rate, "funding_rate.r")?,
        next_funding_time: row.next_funding_time,
        timestamp: frame.time_ms.unwrap_or_else(now_ms),
    }))
}

pub(super) fn parse_reference_frame(text: &str) -> ExchangeResult<Option<CrossExReferenceUpdate>> {
    let root: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("gate crossex json: {error}")))?;
    if root.get("event").and_then(serde_json::Value::as_str) != Some("update") {
        return Ok(None);
    }
    let channel = root.get("channel").and_then(serde_json::Value::as_str);
    let timestamp = root
        .get("time_ms")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or_else(now_ms);
    let result = root.get("result").cloned().unwrap_or_default();
    match channel {
        Some("mark_price") => {
            let row: PricePayload = serde_json::from_value(result)
                .map_err(|error| ExchangeError::Parse(format!("gate crossex mark: {error}")))?;
            Ok(Some(CrossExReferenceUpdate::Mark {
                route: CrossExRoute::parse(&row.symbol)?,
                value: parse_f64(&row.mark_price, "mark_price.mp")?,
                timestamp,
            }))
        }
        Some("index_price") => {
            let row: IndexPayload = serde_json::from_value(result)
                .map_err(|error| ExchangeError::Parse(format!("gate crossex index: {error}")))?;
            Ok(Some(CrossExReferenceUpdate::Index {
                route: CrossExRoute::parse(&row.symbol)?,
                value: parse_f64(&row.index_price, "index_price.ip")?,
                timestamp,
            }))
        }
        Some("open_interest") => {
            let row: OpenInterestPayload = serde_json::from_value(result)
                .map_err(|error| ExchangeError::Parse(format!("gate crossex oi: {error}")))?;
            Ok(Some(CrossExReferenceUpdate::OpenInterest {
                route: CrossExRoute::parse(&row.symbol)?,
                quantity: parse_f64(&row.open_interest, "open_interest.oi")?,
                value: row
                    .open_interest_value
                    .as_deref()
                    .and_then(|value| value.parse().ok()),
                timestamp,
            }))
        }
        _ => Ok(None),
    }
}

pub(super) fn parse_book_snapshot(text: &str) -> ExchangeResult<Option<CrossExBookSnapshot>> {
    let root: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("gate crossex book json: {error}")))?;
    let channel = root
        .get("channel")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if !channel.starts_with("order_book_")
        || channel == "order_book_update"
        || root.get("event").and_then(serde_json::Value::as_str) != Some("update")
    {
        return Ok(None);
    }
    let row: BookPayload = serde_json::from_value(root.get("result").cloned().unwrap_or_default())
        .map_err(|error| ExchangeError::Parse(format!("gate crossex book: {error}")))?;
    Ok(Some(CrossExBookSnapshot {
        route: CrossExRoute::parse(&row.symbol)?,
        bids: levels(row.bids, "book.b")?,
        asks: levels(row.asks, "book.a")?,
        timestamp: row.timestamp,
    }))
}

pub(super) fn parse_book_delta(text: &str) -> ExchangeResult<Option<CrossExBookDelta>> {
    let frame: Envelope<BookDeltaPayload> = parse_update(text, "order_book_update")?;
    let Some(row) = frame.result else {
        return Ok(None);
    };
    // Gate-originated frames currently expose U/u in the opposite numeric
    // direction from Binance/OKX frames. Their interval is still contiguous,
    // so normalize at the protocol boundary instead of venue-guessing later.
    let sequence_start = row.first_sequence.min(row.last_sequence);
    let sequence_end = row.first_sequence.max(row.last_sequence);
    Ok(Some(CrossExBookDelta {
        route: CrossExRoute::parse(&row.symbol)?,
        snapshot: row.snapshot,
        sequence_start,
        sequence_end,
        bids: levels(row.bids, "book_update.b")?,
        asks: levels(row.asks, "book_update.a")?,
        timestamp: row.timestamp,
    }))
}

pub(super) fn parse_instruments(text: &str) -> ExchangeResult<Vec<VenueInstrument>> {
    let rows: Vec<InstrumentPayload> = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("gate crossex instruments: {error}")))?;
    let checked_at_ms = now_ms();
    rows.iter()
        .map(|row| instrument(row, checked_at_ms))
        .collect()
}

pub(super) fn reference_row(
    route: &CrossExRoute,
    mark_price: f64,
    index_price: Option<f64>,
    open_interest: Option<f64>,
    open_interest_value: Option<f64>,
    timestamp: i64,
) -> MarkIndexInfo {
    MarkIndexInfo {
        symbol: route.base.clone(),
        exchange: route.venue(),
        mark_price,
        index_price,
        open_interest,
        open_interest_value,
        timestamp,
    }
}

fn instrument(row: &InstrumentPayload, checked_at_ms: i64) -> ExchangeResult<VenueInstrument> {
    let route = CrossExRoute::parse(&row.symbol)?;
    let listing_status = match row.state.to_ascii_lowercase().as_str() {
        "live" => InstrumentListingStatus::Trading,
        "suspend" => InstrumentListingStatus::Suspended,
        _ if row.delist_time.as_deref().is_some_and(|value| value != "0") => {
            InstrumentListingStatus::Delisted
        }
        _ => InstrumentListingStatus::Unknown,
    };
    let execution_supported = listing_status == InstrumentListingStatus::Trading
        && matches!(
            route.business,
            CrossExBusiness::Spot | CrossExBusiness::Future
        );
    Ok(VenueInstrument {
        venue: route.venue(),
        native_symbol: route.native_symbol.clone(),
        canonical_symbol: route.base.clone(),
        display_symbol: route.display_symbol(),
        asset_class: InstrumentAssetClass::Crypto,
        product_type: Some(route.business.product_type().to_owned()),
        quote_asset: Some(route.quote.clone()),
        settle_asset: (route.business == CrossExBusiness::Future).then(|| route.quote.clone()),
        margin_asset: (route.business != CrossExBusiness::Spot).then(|| route.quote.clone()),
        // CrossEx order quantities are documented in base-currency units.
        contract_size: Some(1.0),
        execution_supported,
        price_tick: positive_f64(row.tick_size.as_deref()),
        qty_step: positive_f64(row.lot_size.as_deref()),
        min_qty: positive_f64(row.min_size.as_deref()),
        min_notional: positive_f64(row.min_notional.as_deref()),
        listing_status,
        funding_interval_ms: None,
        builder_dex: None,
        source: InstrumentMetadataSource::OfficialEndpoint,
        source_url: Some(SYMBOLS_SOURCE_URL.to_owned()),
        checked_at_ms,
        schema_version: Some("crossex-rest-v1.0.2".to_owned()),
    })
}

fn parse_update<T: for<'de> Deserialize<'de>>(
    text: &str,
    expected_channel: &str,
) -> ExchangeResult<Envelope<T>> {
    let frame: Envelope<serde_json::Value> = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("gate crossex json: {error}")))?;
    if frame.channel != expected_channel || frame.event != "update" {
        return Ok(Envelope {
            channel: frame.channel,
            event: frame.event,
            result: None,
            time_ms: frame.time_ms,
        });
    }
    let result = frame
        .result
        .map(|value| {
            serde_json::from_value(value)
                .map_err(|error| ExchangeError::Parse(format!("gate crossex json: {error}")))
        })
        .transpose()?;
    Ok(Envelope {
        channel: frame.channel,
        event: frame.event,
        result,
        time_ms: frame.time_ms,
    })
}

fn levels(rows: Vec<[String; 2]>, field: &str) -> ExchangeResult<Vec<[Decimal; 2]>> {
    rows.into_iter()
        .map(|[price, quantity]| Ok([decimal(&price, field)?, decimal(&quantity, field)?]))
        .collect()
}

fn decimal(value: &str, field: &str) -> ExchangeResult<Decimal> {
    value.parse().map_err(|error| {
        ExchangeError::Parse(format!("gate crossex {field} decimal {value:?}: {error}"))
    })
}

fn decimal_f64(value: Decimal, field: &str) -> ExchangeResult<f64> {
    value.to_string().parse().map_err(|error| {
        ExchangeError::Parse(format!("gate crossex {field} f64 conversion: {error}"))
    })
}

fn parse_f64(value: &str, field: &str) -> ExchangeResult<f64> {
    value.parse().map_err(|error| {
        ExchangeError::Parse(format!("gate crossex {field} f64 {value:?}: {error}"))
    })
}

fn positive_f64(value: Option<&str>) -> Option<f64> {
    value
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn optional_decimal(value: Option<&str>) -> Option<Decimal> {
    value.and_then(|value| value.parse().ok())
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    #[serde(default)]
    channel: String,
    #[serde(default)]
    event: String,
    result: Option<T>,
    #[serde(default)]
    time_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct TickerPayload {
    #[serde(rename = "s")]
    symbol: String,
    #[serde(rename = "lp")]
    last: String,
    #[serde(rename = "bp")]
    bid: String,
    #[serde(rename = "bs")]
    bid_size: Option<String>,
    #[serde(rename = "ap")]
    ask: String,
    #[serde(rename = "as")]
    ask_size: Option<String>,
    #[serde(rename = "v")]
    volume: String,
    #[serde(rename = "ts")]
    timestamp: i64,
}

#[derive(Debug, Deserialize)]
struct FundingPayload {
    #[serde(rename = "s")]
    symbol: String,
    #[serde(rename = "r")]
    rate: String,
    #[serde(rename = "T")]
    next_funding_time: i64,
}

#[derive(Debug, Deserialize)]
struct PricePayload {
    #[serde(rename = "s")]
    symbol: String,
    #[serde(rename = "mp")]
    mark_price: String,
}

#[derive(Debug, Deserialize)]
struct IndexPayload {
    #[serde(rename = "s")]
    symbol: String,
    #[serde(rename = "ip")]
    index_price: String,
}

#[derive(Debug, Deserialize)]
struct OpenInterestPayload {
    #[serde(rename = "s")]
    symbol: String,
    #[serde(rename = "oi")]
    open_interest: String,
    #[serde(rename = "oiV")]
    open_interest_value: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BookPayload {
    #[serde(rename = "s")]
    symbol: String,
    #[serde(rename = "ts")]
    timestamp: i64,
    #[serde(rename = "a")]
    asks: Vec<[String; 2]>,
    #[serde(rename = "b")]
    bids: Vec<[String; 2]>,
}

#[derive(Debug, Deserialize)]
struct BookDeltaPayload {
    #[serde(rename = "s")]
    symbol: String,
    snapshot: bool,
    #[serde(rename = "ts")]
    timestamp: i64,
    #[serde(rename = "U")]
    first_sequence: u64,
    #[serde(rename = "u")]
    last_sequence: u64,
    #[serde(rename = "a")]
    asks: Vec<[String; 2]>,
    #[serde(rename = "b")]
    bids: Vec<[String; 2]>,
}

#[derive(Debug, Deserialize)]
struct InstrumentPayload {
    symbol: String,
    state: String,
    min_size: Option<String>,
    min_notional: Option<String>,
    lot_size: Option<String>,
    tick_size: Option<String>,
    delist_time: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_public_frames_and_preserves_route() {
        let ticker = parse_ticker_frame(include_str!(
            "../../fixtures/gate_crossex/ticker_gate_future_btc_usdt.json"
        ))
        .unwrap()
        .unwrap();
        assert_eq!(ticker.route.venue(), "gate_crossex:gate");
        assert_eq!(ticker.ticker.symbol, "BTC");
        assert_eq!(ticker.ticker.bid, 64_455.0);

        let funding = parse_funding_frame(include_str!(
            "../../fixtures/gate_crossex/funding_gate_future_btc_usdt.json"
        ))
        .unwrap()
        .unwrap();
        let row = funding.into_row(8, ticker.ticker.volume_24h);
        assert_eq!(row.rate, 0.000_071);
        assert_eq!(row.rate_8h, row.rate);
        assert_eq!(row.funding_interval, 8);
    }

    #[test]
    fn subscription_ack_is_ignored_before_typed_result_decode() {
        let ack = r#"{"channel":"ticker","event":"subscribe","result":{"status":"success"}}"#;
        assert!(parse_ticker_frame(ack).unwrap().is_none());
    }

    #[test]
    fn normalizes_gate_reversed_sequence_bounds() {
        let delta = parse_book_delta(include_str!(
            "../../fixtures/gate_crossex/book_update_gate_future_btc_usdt.json"
        ))
        .unwrap()
        .unwrap();
        assert_eq!(delta.sequence_start, 121_192_177_094);
        assert_eq!(delta.sequence_end, 121_192_177_131);
    }

    #[test]
    fn official_symbol_specs_are_execution_constructible() {
        let rows = parse_instruments(include_str!(
            "../../fixtures/gate_crossex/symbols_routes.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(VenueInstrument::is_hedge_constructible));
        assert!(rows.iter().any(|row| row.venue == "gate_crossex:kraken"
            && row.native_symbol == "KRAKEN_FUTURE_BTC_USD"));
    }

    #[test]
    fn full_book_applies_explicit_quantity_multiplier() {
        let book = parse_book_snapshot(include_str!(
            "../../fixtures/gate_crossex/book_5_gate_future_btc_usdt.json"
        ))
        .unwrap()
        .unwrap();
        let projected = book.to_orderbook(1, Decimal::new(1, 4));
        assert_eq!(projected.asks[0], [64_559.3, 4.5182]);
        assert_eq!(projected.exchange, "gate_crossex:gate");
    }
}
