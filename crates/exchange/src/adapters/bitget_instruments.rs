//! Bitget V3 / UTA native instrument identity, sizing metadata and cache.
//!
//! Official source: `GET /api/v3/market/instruments` for Spot plus USDT, USDC
//! and COIN futures. Spot and linear products are executable when every
//! required constraint is present. COIN inverse sizing and Reality order
//! routing are retained in the registry but explicitly observation-only.

use super::bitget_uta_config::BitgetUtaCategory;
use crate::error::{ExchangeError, ExchangeResult};
use dashmap::DashMap;
use serde::Deserialize;
use shared_types::instrument_registry::{InstrumentAssetClass, VenueInstrument};
use shared_types::instruments::{InstrumentListingStatus, InstrumentMetadataSource};
use std::sync::atomic::{AtomicI64, Ordering};

const NAME: &str = "bitget";
const INSTRUMENT_PATH: &str = "/api/v3/market/instruments";
pub(super) const SCHEMA_VERSION: &str = "bitget-uta-get-instruments-2026-06-03";
const HOUR_MS: i64 = 60 * 60 * 1000;
pub(super) const INSTRUMENT_CACHE_TTL_MS: i64 = 6 * HOUR_MS;

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct BitgetInstrumentRow {
    #[serde(default)]
    pub(super) symbol: String,
    #[serde(default)]
    pub(super) category: String,
    #[serde(default, rename = "baseCoin")]
    pub(super) base_coin: String,
    #[serde(default, rename = "quoteCoin")]
    pub(super) quote_coin: String,
    #[serde(default, rename = "settleCoin")]
    pub(super) settle_coin: String,
    #[serde(default, rename = "type")]
    pub(super) contract_type: String,
    #[serde(default)]
    pub(super) status: String,
    #[serde(default, rename = "symbolType")]
    pub(super) symbol_type: String,
    #[serde(default, rename = "isRwa")]
    pub(super) is_rwa: String,
    #[serde(default, rename = "isReality")]
    pub(super) is_reality: String,
    #[serde(default, rename = "priceMultiplier")]
    pub(super) price_multiplier: String,
    #[serde(default, rename = "quantityMultiplier")]
    pub(super) quantity_multiplier: String,
    #[serde(default, rename = "pricePrecision")]
    pub(super) price_precision: String,
    #[serde(default, rename = "quantityPrecision")]
    pub(super) quantity_precision: String,
    #[serde(default, rename = "minOrderQty")]
    pub(super) min_order_qty: String,
    #[serde(default, rename = "minOrderAmount")]
    pub(super) min_order_amount: String,
    #[serde(default, rename = "fundInterval")]
    pub(super) fund_interval: String,
}

#[derive(Debug, Clone)]
pub(super) struct BitgetInstrumentRule {
    pub(super) category: BitgetUtaCategory,
    pub(super) symbol: String,
    pub(super) base: String,
    pub(super) quote: String,
    pub(super) settle: String,
    pub(super) price_tick: f64,
    pub(super) qty_step: f64,
    pub(super) min_qty: f64,
    pub(super) min_notional: Option<f64>,
    pub(super) funding_interval_ms: Option<i64>,
    pub(super) status: String,
    pub(super) asset_class: InstrumentAssetClass,
    pub(super) execution_supported: bool,
}

#[derive(Debug, Clone)]
pub(super) struct BitgetInstrumentSpec {
    pub(super) category: BitgetUtaCategory,
    pub(super) native_symbol: String,
    pub(super) price_tick: f64,
    pub(super) qty_step: f64,
    pub(super) min_qty: f64,
    pub(super) min_notional: Option<f64>,
    pub(super) execution_supported: bool,
    pub(super) listing_status: InstrumentListingStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BitgetNativeIdentity {
    pub(super) category: BitgetUtaCategory,
    pub(super) native_symbol: String,
}

pub(super) fn native_identity(requested: &str) -> ExchangeResult<BitgetNativeIdentity> {
    let normalized = requested
        .trim()
        .replace(['/', '-'], "")
        .to_ascii_uppercase();
    if normalized.is_empty() {
        return Err(ExchangeError::UnsupportedSymbol(requested.to_owned()));
    }
    if normalized.ends_with("_CM") {
        return Ok(BitgetNativeIdentity {
            category: BitgetUtaCategory::CoinFutures,
            native_symbol: normalized,
        });
    }
    if normalized.ends_with("PERP") {
        return Ok(BitgetNativeIdentity {
            category: BitgetUtaCategory::UsdcFutures,
            native_symbol: normalized,
        });
    }
    if let Some(base) = normalized.strip_suffix("USDC") {
        return Ok(BitgetNativeIdentity {
            category: BitgetUtaCategory::UsdcFutures,
            native_symbol: format!("{base}PERP"),
        });
    }
    Ok(BitgetNativeIdentity {
        category: BitgetUtaCategory::UsdtFutures,
        native_symbol: if normalized.ends_with("USDT") {
            normalized
        } else {
            format!("{normalized}USDT")
        },
    })
}

#[derive(Debug, Default)]
pub(super) struct BitgetInstrumentCache {
    specs: DashMap<String, BitgetInstrumentSpec>,
    fetched_at_ms: AtomicI64,
}

impl BitgetInstrumentCache {
    pub(super) fn is_fresh(&self, now_ms: i64) -> bool {
        let fetched_at = self.fetched_at_ms.load(Ordering::Relaxed);
        fetched_at > 0
            && now_ms.saturating_sub(fetched_at) < INSTRUMENT_CACHE_TTL_MS
            && !self.specs.is_empty()
    }

    pub(super) fn replace(&self, specs: Vec<BitgetInstrumentSpec>, fetched_at_ms: i64) {
        self.specs.clear();
        for spec in specs {
            let native = spec.native_symbol.to_ascii_uppercase();
            let base = canonical_base(&native, spec.category);
            self.specs.insert(native, spec.clone());
            match spec.category {
                BitgetUtaCategory::UsdtFutures => {
                    self.specs.insert(base.clone(), spec.clone());
                    self.specs.insert(format!("{base}USDT"), spec);
                }
                BitgetUtaCategory::UsdcFutures => {
                    self.specs.insert(format!("{base}USDC"), spec);
                }
                BitgetUtaCategory::CoinFutures | BitgetUtaCategory::Spot => {}
            }
        }
        self.fetched_at_ms.store(fetched_at_ms, Ordering::Relaxed);
    }

    pub(super) fn resolve(&self, requested: &str) -> ExchangeResult<BitgetInstrumentSpec> {
        let identity = native_identity(requested)?;
        let normalized = requested
            .trim()
            .replace(['/', '-'], "")
            .to_ascii_uppercase();
        self.specs
            .get(&normalized)
            .or_else(|| self.specs.get(&identity.native_symbol))
            .map(|entry| entry.clone())
            .ok_or_else(|| ExchangeError::UnsupportedSymbol(requested.to_owned()))
    }
}

impl BitgetInstrumentRule {
    pub(super) fn from_row(row: BitgetInstrumentRow) -> ExchangeResult<Self> {
        let symbol = non_empty(row.symbol, "symbol")?.to_ascii_uppercase();
        let category = parse_category(&row.category, &symbol)?;
        if category != BitgetUtaCategory::Spot
            && !row.contract_type.eq_ignore_ascii_case("perpetual")
        {
            return Err(validation_error(format!(
                "bitget contract {symbol} type {} is not a perpetual",
                row.contract_type
            )));
        }
        let base = non_empty(row.base_coin, "baseCoin")?.to_ascii_uppercase();
        let quote = expected_quote(category, &row.quote_coin, &symbol)?;
        let settle = if category == BitgetUtaCategory::Spot {
            quote.clone()
        } else if row.settle_coin.trim().is_empty() {
            match category {
                BitgetUtaCategory::CoinFutures => base.clone(),
                _ => quote.clone(),
            }
        } else {
            row.settle_coin.to_ascii_uppercase()
        };
        let is_rwa = yes_no(&row.is_rwa, "isRwa")?;
        let is_reality = yes_no(&row.is_reality, "isReality")?;
        let execution_supported = category != BitgetUtaCategory::CoinFutures && !is_reality;
        let funding_interval_ms = (category != BitgetUtaCategory::Spot)
            .then(|| optional_positive_str(&row.fund_interval))
            .flatten()
            .map(|hours| (hours.round() as i64) * HOUR_MS)
            .filter(|ms| *ms > 0);
        Ok(Self {
            category,
            symbol,
            base,
            quote,
            settle,
            price_tick: multiplier_or_precision(
                &row.price_multiplier,
                &row.price_precision,
                "priceMultiplier/pricePrecision",
            )?,
            qty_step: multiplier_or_precision(
                &row.quantity_multiplier,
                &row.quantity_precision,
                "quantityMultiplier/quantityPrecision",
            )?,
            min_qty: positive_str(&row.min_order_qty, "minOrderQty")?,
            min_notional: optional_positive_str(&row.min_order_amount),
            funding_interval_ms,
            status: row.status,
            asset_class: asset_class(is_rwa, &row.symbol_type),
            execution_supported,
        })
    }

    pub(super) fn to_spec(&self) -> BitgetInstrumentSpec {
        BitgetInstrumentSpec {
            category: self.category,
            native_symbol: self.symbol.clone(),
            price_tick: self.price_tick,
            qty_step: self.qty_step,
            min_qty: self.min_qty,
            min_notional: self.min_notional,
            execution_supported: self.execution_supported,
            listing_status: listing_status(&self.status),
        }
    }

    pub(super) fn into_venue_instrument(self, checked_at_ms: i64) -> VenueInstrument {
        let source_url = format!("{INSTRUMENT_PATH}?category={}", self.category.as_query());
        let spot = self.category == BitgetUtaCategory::Spot;
        VenueInstrument {
            venue: NAME.to_owned(),
            native_symbol: self.symbol,
            canonical_symbol: self.base.clone(),
            display_symbol: format!(
                "{}-{} {}",
                self.base,
                self.quote,
                if spot { "Spot" } else { "Perp" }
            ),
            asset_class: self.asset_class,
            product_type: Some(if spot { "spot" } else { "perp" }.to_owned()),
            quote_asset: Some(self.quote),
            settle_asset: (!spot).then(|| self.settle.clone()),
            margin_asset: (!spot).then_some(self.settle),
            contract_size: Some(1.0),
            execution_supported: self.execution_supported,
            price_tick: Some(self.price_tick),
            qty_step: Some(self.qty_step),
            min_qty: Some(self.min_qty),
            min_notional: self.min_notional,
            listing_status: listing_status(&self.status),
            funding_interval_ms: self.funding_interval_ms,
            builder_dex: None,
            source: InstrumentMetadataSource::OfficialEndpoint,
            source_url: Some(source_url),
            checked_at_ms,
            schema_version: Some(SCHEMA_VERSION.to_owned()),
        }
    }
}

pub(super) fn instruments_and_specs_from_rows(
    rows: Vec<BitgetInstrumentRow>,
    checked_at_ms: i64,
) -> (Vec<VenueInstrument>, Vec<BitgetInstrumentSpec>) {
    let rules = rows
        .into_iter()
        .filter_map(|row| BitgetInstrumentRule::from_row(row).ok())
        .collect::<Vec<_>>();
    let specs = rules.iter().map(BitgetInstrumentRule::to_spec).collect();
    let instruments = rules
        .into_iter()
        .map(|rule| rule.into_venue_instrument(checked_at_ms))
        .collect();
    (instruments, specs)
}

#[cfg(test)]
pub(super) fn instruments_from_rows(
    rows: Vec<BitgetInstrumentRow>,
    checked_at_ms: i64,
) -> Vec<VenueInstrument> {
    instruments_and_specs_from_rows(rows, checked_at_ms).0
}

fn parse_category(raw: &str, symbol: &str) -> ExchangeResult<BitgetUtaCategory> {
    match raw.to_ascii_uppercase().as_str() {
        "USDT-FUTURES" => Ok(BitgetUtaCategory::UsdtFutures),
        "USDC-FUTURES" => Ok(BitgetUtaCategory::UsdcFutures),
        "COIN-FUTURES" => Ok(BitgetUtaCategory::CoinFutures),
        "SPOT" => Ok(BitgetUtaCategory::Spot),
        _ => Err(validation_error(format!(
            "bitget contract {symbol} unsupported category {raw}"
        ))),
    }
}

fn expected_quote(category: BitgetUtaCategory, raw: &str, symbol: &str) -> ExchangeResult<String> {
    let quote = non_empty(raw.to_owned(), "quoteCoin")?.to_ascii_uppercase();
    let expected = match category {
        BitgetUtaCategory::UsdtFutures => Some("USDT"),
        BitgetUtaCategory::UsdcFutures => Some("USDC"),
        BitgetUtaCategory::CoinFutures => None,
        BitgetUtaCategory::Spot => None,
    };
    if expected.is_some_and(|expected| quote != expected) {
        return Err(validation_error(format!(
            "bitget contract {symbol} quoteCoin {quote} does not match {}",
            category.as_query()
        )));
    }
    Ok(quote)
}

fn canonical_base(native: &str, category: BitgetUtaCategory) -> String {
    match category {
        BitgetUtaCategory::UsdtFutures => native.strip_suffix("USDT").unwrap_or(native),
        BitgetUtaCategory::UsdcFutures => native.strip_suffix("PERP").unwrap_or(native),
        BitgetUtaCategory::CoinFutures => native.strip_suffix("USD_CM").unwrap_or(native),
        BitgetUtaCategory::Spot => native,
    }
    .to_owned()
}

fn listing_status(raw: &str) -> InstrumentListingStatus {
    match raw.to_ascii_lowercase().as_str() {
        "online" => InstrumentListingStatus::Trading,
        "listed" => InstrumentListingStatus::PreLaunch,
        "limit_open" | "limit_close" | "restrictedapi" => InstrumentListingStatus::Suspended,
        "offline" => InstrumentListingStatus::Delisted,
        _ => InstrumentListingStatus::Unknown,
    }
}

fn asset_class(is_rwa: bool, symbol_type: &str) -> InstrumentAssetClass {
    if !is_rwa {
        return InstrumentAssetClass::Crypto;
    }
    match symbol_type.to_ascii_lowercase().as_str() {
        "stock" => InstrumentAssetClass::Equity,
        "metal" => InstrumentAssetClass::Metal,
        "commodity" => InstrumentAssetClass::Unknown,
        _ => InstrumentAssetClass::Unknown,
    }
}

fn yes_no(raw: &str, field: &str) -> ExchangeResult<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" | "1" => Ok(true),
        "no" | "false" | "0" | "" => Ok(false),
        _ => Err(validation_error(format!(
            "bitget instrument {field} must be YES/NO: {raw:?}"
        ))),
    }
}

fn multiplier_or_precision(multiplier: &str, precision: &str, field: &str) -> ExchangeResult<f64> {
    if let Ok(value) = positive_str(multiplier, field) {
        return Ok(value);
    }
    let digits = precision.trim().parse::<i32>().map_err(|_| {
        validation_error(format!(
            "bitget instrument {field} missing: {multiplier:?}/{precision:?}"
        ))
    })?;
    if !(0..=18).contains(&digits) {
        return Err(validation_error(format!(
            "bitget instrument {field} precision out of range: {digits}"
        )));
    }
    Ok(10_f64.powi(-digits))
}

fn non_empty(value: String, field: &str) -> ExchangeResult<String> {
    if value.trim().is_empty() {
        Err(validation_error(format!(
            "bitget instrument missing {field}"
        )))
    } else {
        Ok(value)
    }
}

fn positive_str(value: &str, field: &str) -> ExchangeResult<f64> {
    match value.trim().parse::<f64>() {
        Ok(parsed) if parsed.is_finite() && parsed > 0.0 => Ok(parsed),
        _ => Err(validation_error(format!(
            "bitget instrument {field} must be positive finite: {value:?}"
        ))),
    }
}

fn optional_positive_str(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|parsed| parsed.is_finite() && *parsed > 0.0)
}

fn validation_error(message: String) -> ExchangeError {
    ExchangeError::Api {
        exchange: NAME.into(),
        code: "validation".into(),
        message,
    }
}

#[cfg(test)]
#[path = "bitget_instruments_tests.rs"]
mod tests;
