//! Bybit V5 linear perpetual instrument metadata for order sizing and identity.
//!
//! Official Bybit V5 docs checked before writing this code:
//! - `GET /v5/market/instruments-info?category=linear` (cursor-paginated)
//!
//! Bybit linear perps size orders directly in **base coin** (e.g. BTC), so the
//! registry `contract_size` is `1`. We map `priceFilter.tickSize` →
//! `price_tick`, `lotSizeFilter.qtyStep` → `qty_step`,
//! `lotSizeFilter.minOrderQty` → `min_qty`, and `lotSizeFilter.minNotionalValue`
//! → `min_notional`. USDT and USDC perpetuals retain their official native
//! `symbol`/`quoteCoin`/`settleCoin`; dated futures are skipped fail-closed.

use crate::error::{ExchangeError, ExchangeResult};
use serde::Deserialize;
use shared_types::instrument_registry::{InstrumentAssetClass, VenueInstrument};
use shared_types::instruments::{InstrumentListingStatus, InstrumentMetadataSource};

const NAME: &str = "bybit";
const SOURCE_URL: &str = "/v5/market/instruments-info?category=linear";
const SCHEMA_VERSION: &str = "bybit-v5-get-instruments-info-2026-07-13";
const MINUTE_MS: i64 = 60 * 1000;

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct BybitPriceFilter {
    #[serde(default, rename = "tickSize")]
    pub(super) tick_size: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct BybitLotSizeFilter {
    #[serde(default, rename = "minOrderQty")]
    pub(super) min_order_qty: String,
    #[serde(default, rename = "qtyStep")]
    pub(super) qty_step: String,
    #[serde(default, rename = "minNotionalValue")]
    pub(super) min_notional_value: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct BybitInstrumentRow {
    #[serde(default)]
    pub(super) symbol: String,
    #[serde(default, rename = "contractType")]
    pub(super) contract_type: String,
    #[serde(default)]
    pub(super) status: String,
    #[serde(default, rename = "baseCoin")]
    pub(super) base_coin: String,
    #[serde(default, rename = "quoteCoin")]
    pub(super) quote_coin: String,
    #[serde(default, rename = "settleCoin")]
    pub(super) settle_coin: String,
    #[serde(default, rename = "symbolType")]
    pub(super) symbol_type: String,
    #[serde(default, rename = "displayName")]
    pub(super) display_name: String,
    #[serde(default, rename = "fundingInterval")]
    pub(super) funding_interval: i64,
    #[serde(default, rename = "priceFilter")]
    pub(super) price_filter: BybitPriceFilter,
    #[serde(default, rename = "lotSizeFilter")]
    pub(super) lot_size_filter: BybitLotSizeFilter,
}

/// Bybit `instruments-info` 单页响应（带游标），用于串联分页拉全量 linear 合约。
#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct BybitInstrumentsPage {
    #[serde(default)]
    pub(super) list: Vec<BybitInstrumentRow>,
    #[serde(default, rename = "nextPageCursor")]
    pub(super) next_page_cursor: String,
}

#[derive(Debug, Clone)]
pub(super) struct BybitInstrumentRule {
    symbol: String,
    base: String,
    quote: String,
    settle: String,
    display_name: String,
    asset_class: InstrumentAssetClass,
    price_tick: f64,
    qty_step: f64,
    min_qty: f64,
    min_notional: Option<f64>,
    funding_interval_ms: Option<i64>,
    status: String,
}

impl BybitInstrumentRule {
    pub(super) fn from_row(row: BybitInstrumentRow) -> ExchangeResult<Self> {
        let symbol = non_empty(row.symbol, "symbol")?;
        let settle = non_empty(row.settle_coin, "settleCoin")?.to_ascii_uppercase();
        if !matches!(settle.as_str(), "USDT" | "USDC") {
            return Err(validation_error(format!(
                "bybit contract {symbol} settleCoin {settle} is not a supported linear settle asset"
            )));
        }
        if !row.contract_type.eq_ignore_ascii_case("LinearPerpetual") {
            return Err(validation_error(format!(
                "bybit contract {symbol} contractType {} is not a perpetual",
                row.contract_type
            )));
        }
        let asset_class = bybit_asset_class(&row.symbol_type, &row.base_coin);
        let base = non_empty(row.base_coin, "baseCoin")?.to_ascii_uppercase();
        let quote = non_empty(row.quote_coin, "quoteCoin")?.to_ascii_uppercase();
        if quote != settle {
            return Err(validation_error(format!(
                "bybit contract {symbol} quoteCoin {quote} does not match settleCoin {settle}"
            )));
        }
        let funding_interval_ms =
            (row.funding_interval > 0).then_some(row.funding_interval * MINUTE_MS);
        Ok(Self {
            base,
            quote,
            settle,
            symbol,
            display_name: row.display_name,
            asset_class,
            price_tick: positive_str(&row.price_filter.tick_size, "tickSize")?,
            qty_step: positive_str(&row.lot_size_filter.qty_step, "qtyStep")?,
            min_qty: positive_str(&row.lot_size_filter.min_order_qty, "minOrderQty")?,
            min_notional: optional_positive_str(&row.lot_size_filter.min_notional_value),
            funding_interval_ms,
            status: row.status,
        })
    }

    /// 映射为注册表 [`VenueInstrument`]。Bybit linear perp 直接以基础币计量下单，
    /// `contract_size` 取 1；`tickSize`/`qtyStep`/`minOrderQty`/`minNotionalValue`
    /// 一一映射。
    fn into_venue_instrument(self, checked_at_ms: i64) -> VenueInstrument {
        let listing_status = match self.status.as_str() {
            "Trading" => InstrumentListingStatus::Trading,
            "" => InstrumentListingStatus::Unknown,
            _ => InstrumentListingStatus::Delisted,
        };
        let display_symbol = if self.display_name.trim().is_empty() {
            format!("{}-{} Perp", self.base, self.settle)
        } else {
            format!("{} Perp", self.display_name.trim())
        };
        VenueInstrument {
            venue: NAME.to_owned(),
            native_symbol: self.symbol,
            canonical_symbol: self.base.clone(),
            display_symbol,
            asset_class: self.asset_class,
            product_type: Some("perp".to_owned()),
            quote_asset: Some(self.quote),
            settle_asset: Some(self.settle.clone()),
            margin_asset: Some(self.settle),
            contract_size: Some(1.0),
            execution_supported: true,
            price_tick: Some(self.price_tick),
            qty_step: Some(self.qty_step),
            min_qty: Some(self.min_qty),
            min_notional: self.min_notional,
            listing_status,
            funding_interval_ms: self.funding_interval_ms,
            builder_dex: None,
            source: InstrumentMetadataSource::OfficialEndpoint,
            source_url: Some(SOURCE_URL.to_owned()),
            checked_at_ms,
            schema_version: Some(SCHEMA_VERSION.to_owned()),
        }
    }
}

/// 将官方 `instruments-info` 行映射为注册表条目；fail-closed 跳过不支持的结算币、
/// 非永续或缺必需精度的行，只登记能安全 sizing 的 linear 永续。
pub(super) fn instruments_from_rows(
    rows: Vec<BybitInstrumentRow>,
    checked_at_ms: i64,
) -> Vec<VenueInstrument> {
    rows.into_iter()
        .filter_map(|row| BybitInstrumentRule::from_row(row).ok())
        .map(|rule| rule.into_venue_instrument(checked_at_ms))
        .collect()
}

fn bybit_asset_class(symbol_type: &str, base_coin: &str) -> InstrumentAssetClass {
    match symbol_type.trim().to_ascii_lowercase().as_str() {
        "stock" => InstrumentAssetClass::Equity,
        "commodity" => match base_coin.trim().to_ascii_uppercase().as_str() {
            "XAU" | "XAG" => InstrumentAssetClass::Metal,
            "CL" | "BZ" => InstrumentAssetClass::Energy,
            _ => InstrumentAssetClass::Unknown,
        },
        "" | "innovation" => InstrumentAssetClass::Crypto,
        _ => InstrumentAssetClass::Unknown,
    }
}

fn non_empty(value: String, field: &str) -> ExchangeResult<String> {
    if value.trim().is_empty() {
        Err(validation_error(format!(
            "bybit instrument missing {field}"
        )))
    } else {
        Ok(value)
    }
}

fn positive_str(value: &str, field: &str) -> ExchangeResult<f64> {
    match value.trim().parse::<f64>() {
        Ok(parsed) if parsed.is_finite() && parsed > 0.0 => Ok(parsed),
        _ => Err(validation_error(format!(
            "bybit instrument {field} must be positive finite: {value:?}"
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
#[path = "bybit_instruments_tests.rs"]
mod tests;
