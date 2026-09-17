//! Parsers for Kraken Spot public market data.
//!
//! Official schemas:
//! - <https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/ticker>
//! - <https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/instrument>
//! - <https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/book>
//! - <https://docs.kraken.com/api-reference/market-data/get-tradable-asset-pairs>

use super::kraken_symbols::{canonical_asset, spot_symbol};
use crate::error::{ExchangeError, ExchangeResult};
use chrono::DateTime;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde_json::Value;
use shared_types::instrument_registry::{InstrumentAssetClass, VenueInstrument};
use shared_types::instruments::{InstrumentListingStatus, InstrumentMetadataSource};
#[cfg(test)]
use shared_types::TickerInfo;
use shared_types::{OrderBookInfo, SpotTick};
use std::collections::BTreeMap;
use std::str::FromStr;

const VENUE: &str = "kraken";
const INSTRUMENT_SOURCE_URL: &str =
    "https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/instrument";
const ASSET_PAIRS_SOURCE_URL: &str = "https://api.kraken.com/0/public/AssetPairs?assetVersion=1";

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SpotBookUpdate {
    pub symbol: String,
    pub snapshot: bool,
    pub bids: Vec<(Decimal, Decimal)>,
    pub asks: Vec<(Decimal, Decimal)>,
    pub checksum: u32,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone)]
pub(super) struct SpotTickerUpdate {
    pub native_symbol: String,
    pub tick: SpotTick,
}

#[derive(Debug, Clone)]
pub(super) struct SpotInstrumentFrame {
    pub snapshot: bool,
    pub rows: Vec<VenueInstrument>,
    pub asset_classes: BTreeMap<String, InstrumentAssetClass>,
}

#[cfg(test)]
pub(super) fn parse_ticker_frame(text: &str) -> ExchangeResult<Vec<SpotTick>> {
    parse_ticker_updates(text).map(|rows| rows.into_iter().map(|row| row.tick).collect())
}

pub(super) fn parse_ticker_updates(text: &str) -> ExchangeResult<Vec<SpotTickerUpdate>> {
    let root = parse_channel_frame(text, "ticker")?;
    let Some(rows) = root.get("data").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    rows.iter()
        .map(|row| {
            let native_symbol = required_str(row, "symbol")?.to_owned();
            Ok(SpotTickerUpdate {
                native_symbol,
                tick: parse_ticker_row(row)?,
            })
        })
        .collect()
}

#[cfg(test)]
pub(super) fn spot_tick_to_ticker(row: &SpotTick) -> TickerInfo {
    TickerInfo {
        symbol: row.symbol.clone(),
        exchange: row.venue.clone(),
        bid: decimal_to_f64(row.bid),
        ask: decimal_to_f64(row.ask),
        last: decimal_to_f64(row.last),
        volume_24h: decimal_to_f64(row.volume_24h),
        timestamp: row.best_timestamp_ms(),
    }
}

pub(super) fn parse_instrument_frame(text: &str) -> ExchangeResult<SpotInstrumentFrame> {
    let root = parse_channel_frame(text, "instrument")?;
    let snapshot = match root.get("type").and_then(Value::as_str) {
        Some("snapshot") => true,
        Some("update") => false,
        Some(value) => {
            return Err(ExchangeError::Parse(format!(
                "kraken spot instrument frame type is unsupported: {value}"
            )))
        }
        None => {
            return Err(ExchangeError::Parse(
                "kraken spot instrument frame type is missing".to_owned(),
            ))
        }
    };
    let asset_classes = root
        .get("data")
        .and_then(|d| d.get("assets"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|asset| {
            let id = canonical_asset(asset.get("id")?.as_str()?);
            let class = match asset.get("class")?.as_str()? {
                "tokenized_asset" => InstrumentAssetClass::Equity,
                "currency" => InstrumentAssetClass::Crypto,
                _ => InstrumentAssetClass::Unknown,
            };
            Some((id, class))
        })
        .collect::<BTreeMap<_, _>>();
    let Some(pairs) = root
        .get("data")
        .and_then(|data| data.get("pairs"))
        .and_then(Value::as_array)
    else {
        return Ok(SpotInstrumentFrame {
            snapshot,
            rows: Vec::new(),
            asset_classes,
        });
    };
    let checked_at_ms = common::time::now_ms();
    let rows = pairs
        .iter()
        .map(|pair| {
            let mut row = parse_instrument_pair(pair, checked_at_ms)?;
            if let Some(class) = asset_classes.get(&row.canonical_symbol) {
                row.asset_class = *class;
                // Public xStock data does not prove the corporate-action-aware order compiler.
                row.execution_supported = *class == InstrumentAssetClass::Crypto;
            }
            Ok(row)
        })
        .collect::<ExchangeResult<Vec<_>>>()?;
    Ok(SpotInstrumentFrame {
        snapshot,
        rows,
        asset_classes,
    })
}

pub(super) fn parse_asset_pairs(text: &str) -> ExchangeResult<Vec<VenueInstrument>> {
    let root: Value = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("kraken asset pairs json: {error}")))?;
    if let Some(error) = root
        .get("error")
        .and_then(Value::as_array)
        .and_then(|rows| rows.first())
        .and_then(Value::as_str)
    {
        return Err(ExchangeError::Api {
            exchange: VENUE.to_owned(),
            code: error.split(':').next().unwrap_or("unknown").to_owned(),
            message: error.to_owned(),
        });
    }
    let pairs = root
        .get("result")
        .and_then(Value::as_object)
        .ok_or_else(|| ExchangeError::Parse("kraken asset pairs result missing".to_owned()))?;
    let checked_at_ms = common::time::now_ms();
    pairs
        .values()
        .filter(|pair| pair.get("wsname").and_then(Value::as_str).is_some())
        .map(|pair| parse_asset_pair(pair, checked_at_ms))
        .collect()
}

pub(super) fn parse_book_frame(text: &str) -> ExchangeResult<Vec<SpotBookUpdate>> {
    let root = parse_channel_frame(text, "book")?;
    let snapshot = root.get("type").and_then(Value::as_str) == Some("snapshot");
    let Some(rows) = root.get("data").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    rows.iter()
        .map(|row| {
            Ok(SpotBookUpdate {
                symbol: required_str(row, "symbol")?.to_owned(),
                snapshot,
                bids: parse_book_side(row.get("bids"))?,
                asks: parse_book_side(row.get("asks"))?,
                checksum: required_u64(row, "checksum")?.try_into().map_err(|_| {
                    ExchangeError::Parse("kraken book checksum exceeds u32".to_owned())
                })?,
                timestamp_ms: parse_timestamp(required_str(row, "timestamp")?)?,
            })
        })
        .collect()
}

pub(super) fn order_book(
    native_symbol: &str,
    bids: impl Iterator<Item = (Decimal, Decimal)>,
    asks: impl Iterator<Item = (Decimal, Decimal)>,
    timestamp_ms: i64,
    depth: usize,
) -> OrderBookInfo {
    OrderBookInfo {
        symbol: spot_symbol(native_symbol),
        exchange: VENUE.to_owned(),
        bids: bids
            .take(depth)
            .map(|(price, qty)| [decimal_to_f64(price), decimal_to_f64(qty)])
            .collect(),
        asks: asks
            .take(depth)
            .map(|(price, qty)| [decimal_to_f64(price), decimal_to_f64(qty)])
            .collect(),
        timestamp: timestamp_ms,
    }
}

fn parse_channel_frame(text: &str, expected: &str) -> ExchangeResult<Value> {
    let root: Value = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("kraken spot ws json: {error}")))?;
    if root.get("channel").and_then(Value::as_str) != Some(expected) {
        return Ok(Value::Null);
    }
    Ok(root)
}

fn parse_ticker_row(row: &Value) -> ExchangeResult<SpotTick> {
    let received_at_ms = common::time::now_ms();
    Ok(SpotTick {
        venue: VENUE.to_owned(),
        // Keep the quote identity. Collapsing `BTC/USD` to `BTC` makes the
        // cross-venue spot index reject the row as an ambiguous pair.
        symbol: spot_symbol(required_str(row, "symbol")?),
        bid: required_decimal(row, "bid")?,
        ask: required_decimal(row, "ask")?,
        last: required_decimal(row, "last")?,
        bid_size: optional_decimal(row, "bid_qty")?,
        ask_size: optional_decimal(row, "ask_qty")?,
        volume_24h: required_decimal(row, "volume")?,
        exchange_ts_ms: row
            .get("timestamp")
            .and_then(Value::as_str)
            .map(parse_timestamp)
            .transpose()?,
        received_at_ms,
    })
}

fn parse_instrument_pair(pair: &Value, checked_at_ms: i64) -> ExchangeResult<VenueInstrument> {
    let native_symbol = required_str(pair, "symbol")?;
    let base = canonical_asset(required_str(pair, "base")?);
    let quote = canonical_asset(required_str(pair, "quote")?);
    let listing_status = listing_status(required_str(pair, "status")?);
    Ok(VenueInstrument {
        venue: VENUE.to_owned(),
        native_symbol: native_symbol.to_owned(),
        canonical_symbol: base.clone(),
        display_symbol: format!("{base}/{quote}"),
        asset_class: InstrumentAssetClass::Unknown,
        product_type: Some("spot".to_owned()),
        quote_asset: Some(quote.clone()),
        settle_asset: Some(quote),
        margin_asset: None,
        contract_size: Some(1.0),
        execution_supported: false,
        price_tick: optional_f64(pair, "price_increment")?,
        qty_step: optional_f64(pair, "qty_increment")?,
        min_qty: optional_f64(pair, "qty_min")?,
        min_notional: optional_f64(pair, "cost_min")?,
        listing_status,
        funding_interval_ms: None,
        builder_dex: None,
        source: InstrumentMetadataSource::OfficialEndpoint,
        source_url: Some(INSTRUMENT_SOURCE_URL.to_owned()),
        checked_at_ms,
        schema_version: Some("kraken-spot-ws-v2-instrument-2026-08-06".to_owned()),
    })
}

fn parse_asset_pair(pair: &Value, checked_at_ms: i64) -> ExchangeResult<VenueInstrument> {
    let native_symbol = required_str(pair, "wsname")?;
    let base = canonical_asset(required_str(pair, "base")?);
    let quote = canonical_asset(required_str(pair, "quote")?);
    let price_tick = optional_f64(pair, "tick_size")?.or(decimal_step(pair, "pair_decimals")?);
    let tokenized = pair.get("aclass_base").and_then(Value::as_str) == Some("tokenized_asset");
    Ok(VenueInstrument {
        venue: VENUE.to_owned(),
        // AssetPairs exposes the legacy v1 `wsname`; the Spot v2 wire uses
        // readable symbols such as BTC/USD and DOGE/USD.
        native_symbol: if tokenized {
            native_symbol.to_owned()
        } else {
            spot_symbol(native_symbol)
        },
        canonical_symbol: base.clone(),
        display_symbol: format!("{base}/{quote}"),
        asset_class: if tokenized {
            InstrumentAssetClass::Equity
        } else {
            InstrumentAssetClass::Crypto
        },
        product_type: Some("spot".to_owned()),
        quote_asset: Some(quote.clone()),
        settle_asset: Some(quote),
        margin_asset: None,
        contract_size: Some(1.0),
        execution_supported: !tokenized,
        price_tick,
        qty_step: decimal_step(pair, "lot_decimals")?,
        min_qty: optional_f64(pair, "ordermin")?,
        min_notional: optional_f64(pair, "costmin")?,
        listing_status: listing_status(required_str(pair, "status")?),
        funding_interval_ms: None,
        builder_dex: None,
        source: InstrumentMetadataSource::OfficialEndpoint,
        source_url: Some(if tokenized {
            format!("{ASSET_PAIRS_SOURCE_URL}&aclass_base=tokenized_asset")
        } else {
            ASSET_PAIRS_SOURCE_URL.to_owned()
        }),
        checked_at_ms,
        schema_version: Some("kraken-spot-rest-asset-pairs-v1-2026-08-11".to_owned()),
    })
}

fn listing_status(value: &str) -> InstrumentListingStatus {
    match value {
        "online" | "limit_only" | "post_only" => InstrumentListingStatus::Trading,
        "work_in_progress" => InstrumentListingStatus::PreLaunch,
        "cancel_only" | "maintenance" | "reduce_only" => InstrumentListingStatus::Suspended,
        "delisted" => InstrumentListingStatus::Delisted,
        _ => InstrumentListingStatus::Unknown,
    }
}

fn decimal_step(value: &Value, field: &str) -> ExchangeResult<Option<f64>> {
    let Some(decimals) = value.get(field).and_then(Value::as_u64) else {
        return Ok(None);
    };
    let exponent = i32::try_from(decimals)
        .ok()
        .filter(|value| *value <= 18)
        .ok_or_else(|| ExchangeError::Parse(format!("kraken {field} exceeds 18 decimals")))?;
    Ok(Some(10_f64.powi(-exponent)))
}

fn parse_book_side(value: Option<&Value>) -> ExchangeResult<Vec<(Decimal, Decimal)>> {
    value
        .and_then(Value::as_array)
        .map_or(Ok(Vec::new()), |rows| {
            rows.iter()
                .map(|row| {
                    Ok((
                        required_decimal(row, "price")?,
                        required_decimal(row, "qty")?,
                    ))
                })
                .collect()
        })
}

fn required_str<'a>(value: &'a Value, field: &str) -> ExchangeResult<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| ExchangeError::Parse(format!("kraken field {field} is missing")))
}

fn required_u64(value: &Value, field: &str) -> ExchangeResult<u64> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| ExchangeError::Parse(format!("kraken field {field} is missing")))
}

fn required_decimal(value: &Value, field: &str) -> ExchangeResult<Decimal> {
    value
        .get(field)
        .ok_or_else(|| ExchangeError::Parse(format!("kraken field {field} is missing")))
        .and_then(decimal_value)
}

fn optional_decimal(value: &Value, field: &str) -> ExchangeResult<Option<Decimal>> {
    value.get(field).map(decimal_value).transpose()
}

fn optional_f64(value: &Value, field: &str) -> ExchangeResult<Option<f64>> {
    optional_decimal(value, field).map(|value| value.map(decimal_to_f64))
}

fn decimal_value(value: &Value) -> ExchangeResult<Decimal> {
    let raw = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string());
    Decimal::from_str(&raw)
        .map_err(|error| ExchangeError::Parse(format!("kraken decimal {raw}: {error}")))
}

fn decimal_to_f64(value: Decimal) -> f64 {
    value.to_f64().unwrap_or(f64::NAN)
}

fn parse_timestamp(value: &str) -> ExchangeResult<i64> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.timestamp_millis())
        .map_err(|error| ExchangeError::Parse(format!("kraken timestamp {value}: {error}")))
}

#[cfg(test)]
#[path = "kraken_spot_data_tests.rs"]
mod tests;
