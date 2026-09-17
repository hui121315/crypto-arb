//! Kraken Derivatives public REST metadata and WebSocket market parsers.
//!
//! Official schemas:
//! - <https://docs.kraken.com/exchange/api-reference/futures-websocket/ticker>
//! - <https://docs.kraken.com/exchange/api-reference/futures-websocket/book>
//! - <https://futures.kraken.com/derivatives/api/v3/instruments>

use super::kraken_symbols::{canonical_asset, canonical_symbol};
use crate::error::{ExchangeError, ExchangeResult};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde_json::Value;
use shared_types::instrument_registry::{InstrumentAssetClass, VenueInstrument};
use shared_types::instruments::{InstrumentListingStatus, InstrumentMetadataSource};
use shared_types::{FundingRateData, MarkIndexInfo, OrderBookInfo, TickerInfo};
use std::str::FromStr;

const VENUE: &str = "kraken";
const FUNDING_INTERVAL_HOURS: u32 = 1;
const INSTRUMENT_SOURCE_URL: &str = "https://futures.kraken.com/derivatives/api/v3/instruments";

#[derive(Debug, Clone)]
pub(super) struct FuturesTickerUpdate {
    pub ticker: TickerInfo,
    pub funding: Option<FundingRateData>,
    pub mark_index: MarkIndexInfo,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct FuturesBookSnapshot {
    pub product_id: String,
    pub sequence: u64,
    pub timestamp_ms: i64,
    pub bids: Vec<(Decimal, Decimal)>,
    pub asks: Vec<(Decimal, Decimal)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BookSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct FuturesBookDelta {
    pub product_id: String,
    pub sequence: u64,
    pub timestamp_ms: i64,
    pub side: BookSide,
    pub price: Decimal,
    pub quantity: Decimal,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum FuturesBookFrame {
    Snapshot(FuturesBookSnapshot),
    Delta(FuturesBookDelta),
    Ignore,
}

pub(super) fn parse_ticker_frame(text: &str) -> ExchangeResult<Option<FuturesTickerUpdate>> {
    let row: Value = parse_json(text, "ticker")?;
    if row.get("feed").and_then(Value::as_str) != Some("ticker") || row.get("event").is_some() {
        return Ok(None);
    }

    let product_id = required_str(&row, "product_id")?;
    let symbol = canonical_symbol(product_id);
    let timestamp = required_i64(&row, "time")?;
    let volume_24h = optional_f64(&row, "volumeQuote")?
        .or(optional_f64(&row, "volume")?)
        .ok_or_else(|| ExchangeError::Parse("kraken ticker volume is missing".to_owned()))?;
    let mark_price = required_f64(&row, "markPrice")?;
    let open_interest = optional_f64(&row, "openInterest")?;

    let ticker = TickerInfo {
        symbol: symbol.clone(),
        exchange: VENUE.to_owned(),
        bid: required_f64(&row, "bid")?,
        ask: required_f64(&row, "ask")?,
        last: required_f64(&row, "last")?,
        volume_24h,
        timestamp,
    };
    let funding = parse_funding(&row, &symbol, volume_24h, timestamp)?;
    let mark_index = MarkIndexInfo {
        symbol,
        exchange: VENUE.to_owned(),
        mark_price,
        index_price: optional_f64(&row, "index")?,
        open_interest,
        open_interest_value: open_interest.map(|value| value * mark_price),
        timestamp,
    };
    Ok(Some(FuturesTickerUpdate {
        ticker,
        funding,
        mark_index,
    }))
}

pub(super) fn parse_book_frame(text: &str) -> ExchangeResult<FuturesBookFrame> {
    let row: Value = parse_json(text, "book")?;
    match row.get("feed").and_then(Value::as_str) {
        Some("book_snapshot") => Ok(FuturesBookFrame::Snapshot(FuturesBookSnapshot {
            product_id: required_str(&row, "product_id")?.to_owned(),
            sequence: required_u64(&row, "seq")?,
            timestamp_ms: required_i64(&row, "timestamp")?,
            bids: parse_levels(row.get("bids"))?,
            asks: parse_levels(row.get("asks"))?,
        })),
        Some("book") if row.get("event").is_none() => {
            let side = match required_str(&row, "side")? {
                "buy" => BookSide::Buy,
                "sell" => BookSide::Sell,
                value => {
                    return Err(ExchangeError::Parse(format!(
                        "kraken book side {value} is unsupported"
                    )))
                }
            };
            Ok(FuturesBookFrame::Delta(FuturesBookDelta {
                product_id: required_str(&row, "product_id")?.to_owned(),
                sequence: required_u64(&row, "seq")?,
                timestamp_ms: required_i64(&row, "timestamp")?,
                side,
                price: required_decimal(&row, "price")?,
                quantity: required_decimal(&row, "qty")?,
            }))
        }
        _ => Ok(FuturesBookFrame::Ignore),
    }
}

pub(super) fn order_book(
    native_symbol: &str,
    bids: impl Iterator<Item = (Decimal, Decimal)>,
    asks: impl Iterator<Item = (Decimal, Decimal)>,
    timestamp_ms: i64,
    depth: usize,
    contract_multiplier: Decimal,
) -> OrderBookInfo {
    let levels = |side: &mut dyn Iterator<Item = (Decimal, Decimal)>| {
        side.take(depth)
            .map(|(price, quantity)| {
                [
                    decimal_to_f64(price),
                    decimal_to_f64(quantity * contract_multiplier),
                ]
            })
            .collect()
    };
    let mut bids = bids;
    let mut asks = asks;
    OrderBookInfo {
        symbol: canonical_symbol(native_symbol),
        exchange: VENUE.to_owned(),
        bids: levels(&mut bids),
        asks: levels(&mut asks),
        timestamp: timestamp_ms,
    }
}

pub(super) fn parse_instruments(text: &str) -> ExchangeResult<Vec<VenueInstrument>> {
    let root: Value = parse_json(text, "instruments")?;
    if root.get("result").and_then(Value::as_str) != Some("success") {
        return Err(ExchangeError::Parse(
            "kraken futures instruments result is not success".to_owned(),
        ));
    }
    let rows = root
        .get("instruments")
        .and_then(Value::as_array)
        .ok_or_else(|| ExchangeError::Parse("kraken instruments are missing".to_owned()))?;
    let checked_at_ms = common::time::now_ms();
    rows.iter()
        .map(|row| parse_instrument(row, checked_at_ms))
        .collect()
}

fn parse_funding(
    row: &Value,
    symbol: &str,
    volume_24h: f64,
    timestamp: i64,
) -> ExchangeResult<Option<FundingRateData>> {
    if row.get("tag").and_then(Value::as_str) != Some("perpetual") {
        return Ok(None);
    }
    let Some(next_funding_time) = optional_i64(row, "next_funding_rate_time")? else {
        return Ok(None);
    };
    // Kraken documents zero rates as omitted. `relative_*` is the fractional
    // settlement rate; `funding_rate` is not a notional-relative percentage.
    let rate = optional_f64(row, "relative_funding_rate")?.unwrap_or(0.0);
    let predicted_rate = optional_f64(row, "relative_funding_rate_prediction")?.or(Some(0.0));
    Ok(Some(FundingRateData {
        symbol: symbol.to_owned(),
        exchange: VENUE.to_owned(),
        rate,
        // Compatibility field: preserve the native one-hour event rate. Do not
        // multiply it into a synthetic eight-hour event.
        rate_8h: rate,
        predicted_rate,
        next_funding_time,
        funding_interval: FUNDING_INTERVAL_HOURS,
        volume_24h,
        timestamp,
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    }))
}

fn parse_instrument(row: &Value, checked_at_ms: i64) -> ExchangeResult<VenueInstrument> {
    let native_symbol = required_str(row, "symbol")?;
    let product_type = required_str(row, "type")?;
    let base = canonical_asset(required_str(row, "base")?);
    let quote = canonical_asset(required_str(row, "quote")?);
    let tradeable = required_bool(row, "tradeable")?;
    let expired = row
        .get("isExpired")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let precision = i32::try_from(required_i64(row, "contractValueTradePrecision")?)
        .map_err(|_| ExchangeError::Parse("kraken contract precision exceeds i32".to_owned()))?;
    let exponent = precision.checked_neg().ok_or_else(|| {
        ExchangeError::Parse("kraken contract precision cannot be negated".to_owned())
    })?;
    let quantity_step = 10_f64.powi(exponent);
    let listing_status = if expired {
        InstrumentListingStatus::Delisted
    } else if tradeable {
        InstrumentListingStatus::Trading
    } else {
        InstrumentListingStatus::Suspended
    };
    let execution_supported = product_type == "flexible_futures";

    Ok(VenueInstrument {
        venue: VENUE.to_owned(),
        native_symbol: native_symbol.to_owned(),
        canonical_symbol: base.clone(),
        display_symbol: format!("{base}/{quote} Perp"),
        asset_class: InstrumentAssetClass::Crypto,
        product_type: Some(
            if execution_supported {
                "perp"
            } else {
                "inverse_perp"
            }
            .to_owned(),
        ),
        quote_asset: Some(quote.clone()),
        settle_asset: Some(quote),
        margin_asset: None,
        contract_size: optional_f64(row, "contractSize")?,
        execution_supported,
        price_tick: optional_f64(row, "tickSize")?,
        qty_step: Some(quantity_step),
        min_qty: Some(quantity_step),
        min_notional: None,
        listing_status,
        funding_interval_ms: Some(i64::from(FUNDING_INTERVAL_HOURS) * 3_600_000),
        builder_dex: None,
        source: InstrumentMetadataSource::OfficialEndpoint,
        source_url: Some(INSTRUMENT_SOURCE_URL.to_owned()),
        checked_at_ms,
        schema_version: Some("kraken-futures-instruments-v3-2026-08-06".to_owned()),
    })
}

fn parse_levels(value: Option<&Value>) -> ExchangeResult<Vec<(Decimal, Decimal)>> {
    value
        .and_then(Value::as_array)
        .ok_or_else(|| ExchangeError::Parse("kraken book levels are missing".to_owned()))?
        .iter()
        .map(|row| {
            Ok((
                required_decimal(row, "price")?,
                required_decimal(row, "qty")?,
            ))
        })
        .collect()
}

fn parse_json(text: &str, operation: &str) -> ExchangeResult<Value> {
    serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("kraken futures {operation} json: {error}")))
}

fn required_str<'a>(value: &'a Value, field: &str) -> ExchangeResult<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| ExchangeError::Parse(format!("kraken field {field} is missing")))
}

fn required_bool(value: &Value, field: &str) -> ExchangeResult<bool> {
    value
        .get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| ExchangeError::Parse(format!("kraken field {field} is missing")))
}

fn required_u64(value: &Value, field: &str) -> ExchangeResult<u64> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| ExchangeError::Parse(format!("kraken field {field} is missing")))
}

fn required_i64(value: &Value, field: &str) -> ExchangeResult<i64> {
    optional_i64(value, field)?
        .ok_or_else(|| ExchangeError::Parse(format!("kraken field {field} is missing")))
}

fn optional_i64(value: &Value, field: &str) -> ExchangeResult<Option<i64>> {
    value
        .get(field)
        .map(|value| {
            value
                .as_i64()
                .ok_or_else(|| ExchangeError::Parse(format!("kraken field {field} is not i64")))
        })
        .transpose()
}

fn required_f64(value: &Value, field: &str) -> ExchangeResult<f64> {
    optional_f64(value, field)?
        .ok_or_else(|| ExchangeError::Parse(format!("kraken field {field} is missing")))
}

fn optional_f64(value: &Value, field: &str) -> ExchangeResult<Option<f64>> {
    value
        .get(field)
        .map(decimal_value)
        .transpose()
        .map(|value| value.map(decimal_to_f64))
}

fn required_decimal(value: &Value, field: &str) -> ExchangeResult<Decimal> {
    value
        .get(field)
        .ok_or_else(|| ExchangeError::Parse(format!("kraken field {field} is missing")))
        .and_then(decimal_value)
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

#[cfg(test)]
#[path = "kraken_futures_data_tests.rs"]
mod tests;
