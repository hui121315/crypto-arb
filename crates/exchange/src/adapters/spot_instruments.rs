//! Official CEX spot instrument specifications.
//!
//! Exchanges expose these slowly-changing precision/listing snapshots over
//! public HTTP, not account WebSockets. Order mutation and finality stay on the
//! private WS paths; this module is the fail-closed source for native symbol,
//! tick, lot and minimum-order constraints.

use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use reqwest::Method;
use serde::Deserialize;
use shared_types::instrument_registry::{InstrumentAssetClass, VenueInstrument};
use shared_types::instruments::{InstrumentListingStatus, InstrumentMetadataSource};

const BINANCE_PATH: &str = "/api/v3/exchangeInfo";
const BINANCE_SCHEMA: &str = "binance-spot-exchange-info-2026-08-04";
const OKX_PATH: &str = "/api/v5/public/instruments?instType=SPOT";
const OKX_SCHEMA: &str = "okx-v5-spot-instruments-2026-08-04";
const BYBIT_PATH: &str = "/v5/market/instruments-info?category=spot";
const BYBIT_SCHEMA: &str = "bybit-v5-spot-instruments-2026-08-04";
const GATE_PATH: &str = "/api/v4/spot/currency_pairs";
const GATE_SCHEMA: &str = "gate-v4-spot-currency-pairs-2026-08-04";
const KUCOIN_PATH: &str = "/api/v2/symbols";
const KUCOIN_SCHEMA: &str = "kucoin-spot-symbols-v2-2026-08-04";

pub(crate) fn evidence_matches(instrument: &VenueInstrument) -> bool {
    let venue = instrument
        .venue
        .split_once(':')
        .map_or(instrument.venue.as_str(), |(family, _)| family);
    let expected = match venue.to_ascii_lowercase().as_str() {
        "binance" => Some((BINANCE_PATH, BINANCE_SCHEMA)),
        "okx" => Some((OKX_PATH, OKX_SCHEMA)),
        "bybit" => Some((BYBIT_PATH, BYBIT_SCHEMA)),
        "gate" => Some((GATE_PATH, GATE_SCHEMA)),
        "kucoin" => Some((KUCOIN_PATH, KUCOIN_SCHEMA)),
        "bitget" => Some((
            "/api/v3/market/instruments?category=SPOT",
            super::bitget_instruments::SCHEMA_VERSION,
        )),
        "hyperliquid" => Some((
            "/info {\"type\":\"spotMeta\"}",
            super::hyperliquid_instruments::SPOT_SCHEMA_VERSION,
        )),
        _ => None,
    };
    let Some((path, schema)) = expected else {
        return false;
    };
    instrument.product_type.as_deref() == Some("spot")
        && instrument.source == InstrumentMetadataSource::OfficialEndpoint
        && instrument.source_url.as_deref() == Some(path)
        && instrument.schema_version.as_deref() == Some(schema)
}

pub(super) async fn binance(
    http: &HttpClient,
    base_url: &str,
    checked_at_ms: i64,
) -> ExchangeResult<Vec<VenueInstrument>> {
    let response: BinanceResponse = get_json(http, base_url, BINANCE_PATH, "binance").await?;
    Ok(response
        .symbols
        .iter()
        .filter_map(|row| binance_instrument(row, checked_at_ms))
        .collect())
}

pub(super) async fn okx(
    http: &HttpClient,
    base_url: &str,
    checked_at_ms: i64,
) -> ExchangeResult<Vec<VenueInstrument>> {
    let response: OkxResponse<OkxSpotRow> = get_json(http, base_url, OKX_PATH, "okx").await?;
    response.ensure_success("okx")?;
    Ok(response
        .data
        .iter()
        .filter_map(|row| okx_instrument(row, checked_at_ms))
        .collect())
}

pub(super) async fn bybit(
    http: &HttpClient,
    base_url: &str,
    checked_at_ms: i64,
) -> ExchangeResult<Vec<VenueInstrument>> {
    let response: BybitResponse = get_json(http, base_url, BYBIT_PATH, "bybit").await?;
    if response.ret_code != 0 {
        return Err(api_error(
            "bybit",
            response.ret_code.to_string(),
            response.ret_msg,
        ));
    }
    Ok(response
        .result
        .list
        .iter()
        .filter_map(|row| bybit_instrument(row, checked_at_ms))
        .collect())
}

pub(super) async fn gate(
    http: &HttpClient,
    base_url: &str,
    checked_at_ms: i64,
) -> ExchangeResult<Vec<VenueInstrument>> {
    let rows: Vec<GateSpotRow> = get_json(http, base_url, GATE_PATH, "gate").await?;
    Ok(rows
        .iter()
        .filter_map(|row| gate_instrument(row, checked_at_ms))
        .collect())
}

pub(super) async fn kucoin(
    http: &HttpClient,
    base_url: &str,
    checked_at_ms: i64,
) -> ExchangeResult<Vec<VenueInstrument>> {
    let response: KucoinResponse<Vec<KucoinSpotRow>> =
        get_json(http, base_url, KUCOIN_PATH, "kucoin").await?;
    if response.code != "200000" {
        return Err(api_error("kucoin", response.code, response.msg));
    }
    Ok(response
        .data
        .iter()
        .flatten()
        .filter_map(|row| kucoin_instrument(row, checked_at_ms))
        .collect())
}

async fn get_json<T: serde::de::DeserializeOwned>(
    http: &HttpClient,
    base_url: &str,
    path: &str,
    venue: &str,
) -> ExchangeResult<T> {
    let url = format!("{base_url}{path}");
    let response = http
        .execute_with_retry(|| http.request(Method::GET, &url))
        .await?;
    response
        .json()
        .await
        .map_err(|error| ExchangeError::Parse(format!("{venue} spot instrument response: {error}")))
}

fn spot_instrument(input: &SpotInstrumentInput<'_>) -> Option<VenueInstrument> {
    let base = non_empty(input.base)?.to_ascii_uppercase();
    let quote = non_empty(input.quote)?.to_ascii_uppercase();
    let native_symbol = non_empty(input.native_symbol)?.to_ascii_uppercase();
    Some(VenueInstrument {
        venue: input.venue.to_owned(),
        native_symbol,
        canonical_symbol: base.clone(),
        display_symbol: format!("{base}-{quote} Spot"),
        asset_class: InstrumentAssetClass::Crypto,
        product_type: Some("spot".to_owned()),
        quote_asset: Some(quote),
        settle_asset: None,
        margin_asset: None,
        contract_size: Some(1.0),
        execution_supported: true,
        price_tick: positive(input.price_tick),
        qty_step: positive(input.qty_step),
        min_qty: positive(input.min_qty),
        min_notional: positive(input.min_notional),
        listing_status: input.listing_status,
        funding_interval_ms: None,
        builder_dex: None,
        source: InstrumentMetadataSource::OfficialEndpoint,
        source_url: Some(input.source_url.to_owned()),
        checked_at_ms: input.checked_at_ms,
        schema_version: Some(input.schema_version.to_owned()),
    })
}

struct SpotInstrumentInput<'a> {
    venue: &'a str,
    native_symbol: &'a str,
    base: &'a str,
    quote: &'a str,
    price_tick: f64,
    qty_step: f64,
    min_qty: f64,
    min_notional: f64,
    listing_status: InstrumentListingStatus,
    source_url: &'a str,
    schema_version: &'a str,
    checked_at_ms: i64,
}

#[derive(Deserialize)]
struct BinanceResponse {
    #[serde(default)]
    symbols: Vec<BinanceSpotRow>,
}

#[derive(Deserialize)]
struct BinanceSpotRow {
    #[serde(default)]
    symbol: String,
    #[serde(default)]
    status: String,
    #[serde(default, rename = "baseAsset")]
    base_asset: String,
    #[serde(default, rename = "quoteAsset")]
    quote_asset: String,
    #[serde(default)]
    filters: Vec<BinanceFilter>,
}

#[derive(Deserialize)]
struct BinanceFilter {
    #[serde(default, rename = "filterType")]
    filter_type: String,
    #[serde(default, rename = "tickSize")]
    tick_size: String,
    #[serde(default, rename = "stepSize")]
    step_size: String,
    #[serde(default, rename = "minQty")]
    min_qty: String,
    #[serde(default, rename = "minNotional", alias = "notional")]
    min_notional: String,
}

fn binance_instrument(row: &BinanceSpotRow, checked_at_ms: i64) -> Option<VenueInstrument> {
    let price = row
        .filters
        .iter()
        .find(|row| row.filter_type == "PRICE_FILTER")?;
    let lot = row
        .filters
        .iter()
        .find(|row| row.filter_type == "LOT_SIZE")?;
    let notional = row
        .filters
        .iter()
        .find(|row| matches!(row.filter_type.as_str(), "NOTIONAL" | "MIN_NOTIONAL"));
    spot_instrument(&SpotInstrumentInput {
        venue: "binance",
        native_symbol: &row.symbol,
        base: &row.base_asset,
        quote: &row.quote_asset,
        price_tick: parse_number(&price.tick_size),
        qty_step: parse_number(&lot.step_size),
        min_qty: parse_number(&lot.min_qty),
        min_notional: notional.map_or(0.0, |row| parse_number(&row.min_notional)),
        listing_status: listing(row.status.eq_ignore_ascii_case("TRADING"), &row.status),
        source_url: BINANCE_PATH,
        schema_version: BINANCE_SCHEMA,
        checked_at_ms,
    })
}

#[derive(Deserialize)]
struct OkxResponse<T> {
    #[serde(default)]
    code: String,
    #[serde(default)]
    msg: String,
    #[serde(default)]
    data: Vec<T>,
}

impl<T> OkxResponse<T> {
    fn ensure_success(&self, venue: &str) -> ExchangeResult<()> {
        if self.code == "0" {
            Ok(())
        } else {
            Err(api_error(venue, self.code.clone(), self.msg.clone()))
        }
    }
}

#[derive(Default, Deserialize)]
struct OkxSpotRow {
    #[serde(default, rename = "instId")]
    inst_id: String,
    #[serde(default, rename = "baseCcy")]
    base: String,
    #[serde(default, rename = "quoteCcy")]
    quote: String,
    #[serde(default, rename = "tickSz")]
    tick: String,
    #[serde(default, rename = "lotSz")]
    lot: String,
    #[serde(default, rename = "minSz")]
    min_size: String,
    #[serde(default)]
    state: String,
}

fn okx_instrument(row: &OkxSpotRow, checked_at_ms: i64) -> Option<VenueInstrument> {
    spot_instrument(&SpotInstrumentInput {
        venue: "okx",
        native_symbol: &row.inst_id,
        base: &row.base,
        quote: &row.quote,
        price_tick: parse_number(&row.tick),
        qty_step: parse_number(&row.lot),
        min_qty: parse_number(&row.min_size),
        min_notional: 0.0,
        listing_status: listing(row.state == "live", &row.state),
        source_url: OKX_PATH,
        schema_version: OKX_SCHEMA,
        checked_at_ms,
    })
}

#[derive(Deserialize)]
struct BybitResponse {
    #[serde(default, rename = "retCode")]
    ret_code: i64,
    #[serde(default, rename = "retMsg")]
    ret_msg: String,
    #[serde(default)]
    result: BybitResult,
}

#[derive(Default, Deserialize)]
struct BybitResult {
    #[serde(default)]
    list: Vec<BybitSpotRow>,
}

#[derive(Deserialize)]
struct BybitSpotRow {
    #[serde(default)]
    symbol: String,
    #[serde(default, rename = "baseCoin")]
    base: String,
    #[serde(default, rename = "quoteCoin")]
    quote: String,
    #[serde(default)]
    status: String,
    #[serde(default, rename = "priceFilter")]
    price: BybitPriceFilter,
    #[serde(default, rename = "lotSizeFilter")]
    lot: BybitLotFilter,
}

#[derive(Default, Deserialize)]
struct BybitPriceFilter {
    #[serde(default, rename = "tickSize")]
    tick_size: String,
}

#[derive(Default, Deserialize)]
struct BybitLotFilter {
    #[serde(default, rename = "basePrecision", alias = "qtyStep")]
    base_precision: String,
    #[serde(default, rename = "minOrderQty")]
    min_qty: String,
    #[serde(default, rename = "minOrderAmt", alias = "minNotionalValue")]
    min_notional: String,
}

fn bybit_instrument(row: &BybitSpotRow, checked_at_ms: i64) -> Option<VenueInstrument> {
    spot_instrument(&SpotInstrumentInput {
        venue: "bybit",
        native_symbol: &row.symbol,
        base: &row.base,
        quote: &row.quote,
        price_tick: parse_number(&row.price.tick_size),
        qty_step: parse_number(&row.lot.base_precision),
        min_qty: parse_number(&row.lot.min_qty),
        min_notional: parse_number(&row.lot.min_notional),
        listing_status: listing(row.status == "Trading", &row.status),
        source_url: BYBIT_PATH,
        schema_version: BYBIT_SCHEMA,
        checked_at_ms,
    })
}

#[derive(Deserialize)]
struct GateSpotRow {
    #[serde(default)]
    id: String,
    #[serde(default)]
    base: String,
    #[serde(default)]
    quote: String,
    #[serde(default)]
    precision: u32,
    #[serde(default, rename = "amount_precision")]
    amount_precision: u32,
    #[serde(default, rename = "min_base_amount")]
    min_base_amount: String,
    #[serde(default, rename = "min_quote_amount")]
    min_quote_amount: String,
    #[serde(default, rename = "trade_status")]
    trade_status: String,
}

fn gate_instrument(row: &GateSpotRow, checked_at_ms: i64) -> Option<VenueInstrument> {
    spot_instrument(&SpotInstrumentInput {
        venue: "gate",
        native_symbol: &row.id,
        base: &row.base,
        quote: &row.quote,
        price_tick: decimal_step(row.precision),
        qty_step: decimal_step(row.amount_precision),
        min_qty: parse_number(&row.min_base_amount),
        min_notional: parse_number(&row.min_quote_amount),
        listing_status: listing(row.trade_status == "tradable", &row.trade_status),
        source_url: GATE_PATH,
        schema_version: GATE_SCHEMA,
        checked_at_ms,
    })
}

#[derive(Deserialize)]
struct KucoinResponse<T> {
    #[serde(default)]
    code: String,
    #[serde(default)]
    msg: String,
    data: Option<T>,
}

#[derive(Deserialize)]
struct KucoinSpotRow {
    #[serde(default)]
    symbol: String,
    #[serde(default, rename = "baseCurrency")]
    base: String,
    #[serde(default, rename = "quoteCurrency")]
    quote: String,
    #[serde(default, rename = "priceIncrement")]
    price_increment: String,
    #[serde(default, rename = "baseIncrement")]
    base_increment: String,
    #[serde(default, rename = "baseMinSize")]
    base_min_size: String,
    #[serde(default, rename = "quoteMinSize")]
    quote_min_size: String,
    #[serde(default, rename = "enableTrading")]
    enable_trading: bool,
}

fn kucoin_instrument(row: &KucoinSpotRow, checked_at_ms: i64) -> Option<VenueInstrument> {
    spot_instrument(&SpotInstrumentInput {
        venue: "kucoin",
        native_symbol: &row.symbol,
        base: &row.base,
        quote: &row.quote,
        price_tick: parse_number(&row.price_increment),
        qty_step: parse_number(&row.base_increment),
        min_qty: parse_number(&row.base_min_size),
        min_notional: parse_number(&row.quote_min_size),
        listing_status: if row.enable_trading {
            InstrumentListingStatus::Trading
        } else {
            InstrumentListingStatus::Suspended
        },
        source_url: KUCOIN_PATH,
        schema_version: KUCOIN_SCHEMA,
        checked_at_ms,
    })
}

fn non_empty(value: &str) -> Option<&str> {
    (!value.trim().is_empty()).then(|| value.trim())
}

fn parse_number(value: &str) -> f64 {
    value.trim().parse().unwrap_or_default()
}

fn positive(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

fn decimal_step(precision: u32) -> f64 {
    10_f64.powi(-(i32::try_from(precision).unwrap_or(i32::MAX)))
}

fn listing(is_trading: bool, raw: &str) -> InstrumentListingStatus {
    if is_trading {
        InstrumentListingStatus::Trading
    } else if raw.trim().is_empty() {
        InstrumentListingStatus::Unknown
    } else {
        InstrumentListingStatus::Suspended
    }
}

fn api_error(venue: &str, code: String, message: String) -> ExchangeError {
    ExchangeError::Api {
        exchange: venue.to_owned(),
        code,
        message,
    }
}

#[cfg(test)]
#[path = "spot_instruments_tests.rs"]
mod tests;
