//! Gate.io USDT-margined perpetual instrument metadata used by live order sizing.
//!
//! Official Gate v4 docs checked before writing this code:
//! - `GET /api/v4/futures/usdt/contracts`
//!
//! Gate USDT perps are quanto contracts: order quantity is an **integer number
//! of contracts**, each worth `quanto_multiplier` base coins. We therefore map
//! `quanto_multiplier` → `contract_size`, `order_size_min` → `min_qty` (in
//! contracts) and a fixed `qty_step` of 1 contract, mirroring the OKX adapter.
//! `min_notional` stays empty (Gate exposes no official minimum notional).

use crate::error::{ExchangeError, ExchangeResult};
use serde::Deserialize;
use shared_types::instrument_registry::{InstrumentAssetClass, VenueInstrument};
use shared_types::instruments::{InstrumentListingStatus, InstrumentMetadataSource};

const NAME: &str = "gate";
const SOURCE_URL: &str = "/api/v4/futures/usdt/contracts";
const SETTLE: &str = "USDT";

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GateContractRow {
    #[serde(default)]
    pub(super) name: String,
    #[serde(default, rename = "quanto_multiplier")]
    pub(super) quanto_multiplier: String,
    #[serde(default, rename = "order_price_round")]
    pub(super) order_price_round: String,
    #[serde(default, rename = "order_size_min")]
    pub(super) order_size_min: i64,
    #[serde(default, rename = "funding_interval")]
    pub(super) funding_interval: i64,
    #[serde(default, rename = "in_delisting")]
    pub(super) in_delisting: bool,
    #[serde(default)]
    pub(super) status: String,
}

#[derive(Debug, Clone)]
pub(super) struct GateInstrumentRule {
    name: String,
    base: String,
    contract_size: f64,
    price_tick: f64,
    min_qty: f64,
    funding_interval_ms: Option<i64>,
    in_delisting: bool,
    status: String,
}

impl GateInstrumentRule {
    pub(super) fn from_row(row: GateContractRow) -> ExchangeResult<Self> {
        let name = non_empty(row.name, "name")?;
        let (base, quote) = name
            .rsplit_once('_')
            .ok_or_else(|| validation_error(format!("gate contract {name} is not BASE_QUOTE")))?;
        if !quote.eq_ignore_ascii_case(SETTLE) {
            return Err(validation_error(format!(
                "gate contract {name} settle {quote} is not {SETTLE}; cannot size base quantity safely"
            )));
        }
        if base.is_empty() {
            return Err(validation_error(format!(
                "gate contract {name} has empty base"
            )));
        }
        let funding_interval_ms = (row.funding_interval > 0).then_some(row.funding_interval * 1000);
        Ok(Self {
            base: base.to_ascii_uppercase(),
            name,
            contract_size: positive_number(&row.quanto_multiplier, "quanto_multiplier")?,
            price_tick: positive_number(&row.order_price_round, "order_price_round")?,
            min_qty: positive_int(row.order_size_min, "order_size_min")?,
            funding_interval_ms,
            in_delisting: row.in_delisting,
            status: row.status,
        })
    }

    /// 映射为注册表 [`VenueInstrument`]。Gate 永续以「张数」下单（整数步进），
    /// 每张合约 `quanto_multiplier` 个基础币 → `contract_size`；`order_size_min`
    /// 即最小张数 → `min_qty`（`min_notional` 留空，Gate 不提供官方最小名义）。
    fn into_venue_instrument(self, checked_at_ms: i64) -> VenueInstrument {
        let listing_status = if self.in_delisting {
            InstrumentListingStatus::Delisted
        } else {
            match self.status.as_str() {
                "trading" => InstrumentListingStatus::Trading,
                _ => InstrumentListingStatus::Unknown,
            }
        };
        VenueInstrument {
            venue: NAME.to_owned(),
            native_symbol: self.name,
            canonical_symbol: self.base.clone(),
            display_symbol: format!("{}-{SETTLE} Perp", self.base),
            asset_class: InstrumentAssetClass::Crypto,
            product_type: Some("perp".to_owned()),
            quote_asset: Some(SETTLE.to_owned()),
            settle_asset: Some(SETTLE.to_owned()),
            margin_asset: Some(SETTLE.to_owned()),
            contract_size: Some(self.contract_size),
            execution_supported: true,
            price_tick: Some(self.price_tick),
            qty_step: Some(1.0),
            min_qty: Some(self.min_qty),
            min_notional: None,
            listing_status,
            funding_interval_ms: self.funding_interval_ms,
            builder_dex: None,
            source: InstrumentMetadataSource::OfficialEndpoint,
            source_url: Some(SOURCE_URL.to_owned()),
            checked_at_ms,
            schema_version: None,
        }
    }
}

/// 将官方 `contracts` 行映射为注册表条目；fail-closed 跳过非 USDT 结算或缺必需
/// 精度的行，只登记能安全 sizing 的 USDT 永续。
pub(super) fn instruments_from_rows(
    rows: Vec<GateContractRow>,
    checked_at_ms: i64,
) -> Vec<VenueInstrument> {
    rows.into_iter()
        .filter_map(|row| GateInstrumentRule::from_row(row).ok())
        .map(|rule| rule.into_venue_instrument(checked_at_ms))
        .collect()
}

fn non_empty(value: String, field: &str) -> ExchangeResult<String> {
    if value.trim().is_empty() {
        Err(validation_error(format!("gate instrument missing {field}")))
    } else {
        Ok(value)
    }
}

fn positive_number(value: &str, field: &str) -> ExchangeResult<f64> {
    let number = value
        .parse::<f64>()
        .map_err(|error| validation_error(format!("gate instrument invalid {field}: {error}")))?;
    positive_value(number, field)
}

fn positive_int(value: i64, field: &str) -> ExchangeResult<f64> {
    if value > 0 {
        Ok(value as f64)
    } else {
        Err(validation_error(format!(
            "gate instrument {field} must be positive: {value}"
        )))
    }
}

fn positive_value(value: f64, field: &str) -> ExchangeResult<f64> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(validation_error(format!(
            "gate instrument {field} must be positive finite: {value}"
        )))
    }
}

fn validation_error(message: String) -> ExchangeError {
    ExchangeError::Api {
        exchange: NAME.into(),
        code: "validation".into(),
        message,
    }
}

#[cfg(test)]
#[path = "gate_instruments_tests.rs"]
mod tests;
