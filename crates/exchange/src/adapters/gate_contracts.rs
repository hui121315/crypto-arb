//! Gate futures contract identity and metadata cache.
//!
//! Gate identifies a futures market with two values: the `{settle}` path
//! segment and the native contract name. The endpoint settle is authoritative;
//! it must not be reconstructed from a canonical symbol or contract suffix.
//!
//! Official schema: <https://www.gate.com/docs/developers/apiv4/en/futures/>
//! `contract_type` classifies non-crypto contracts such as stocks, metals,
//! indices and forex. Unknown non-empty classifications remain fail-closed.

use super::gate_public_rest::futures_get;
use super::gate_response::parse_err;
use super::gate_trade_data::validation_error;
use crate::adapter::strip_common_suffixes;
use crate::error::ExchangeResult;
use crate::http::HttpClient;
use common::time::now_ms;
use dashmap::DashMap;
use serde::Deserialize;
#[cfg(test)]
use serde::Serialize;
use serde_json::Value;
use shared_types::instrument_registry::{InstrumentAssetClass, VenueInstrument};
use shared_types::{InstrumentListingStatus, InstrumentMetadataSource};
use std::sync::atomic::{AtomicI64, Ordering};

const CONTRACT_CACHE_TTL_MS: i64 = 24 * 60 * 60 * 1000;
const DEFAULT_SETTLE: &str = "usdt";
const CONTRACT_SCHEMA_VERSION: &str = "gate-apiv4-list-futures-contracts-2026-06-03";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GateContractIdentity {
    pub(super) settle: String,
    pub(super) native_symbol: String,
}

#[derive(Debug, Clone)]
pub(super) struct GateContractSpec {
    pub(super) identity: GateContractIdentity,
    pub(super) asset_class: InstrumentAssetClass,
    pub(super) contract_size: f64,
    pub(super) price_tick: f64,
    pub(super) qty_step: Option<f64>,
    pub(super) min_qty: f64,
    pub(super) max_qty: f64,
    pub(super) market_max_qty: f64,
    pub(super) listing_status: InstrumentListingStatus,
    pub(super) leverage_min: f64,
    pub(super) leverage_max: f64,
    pub(super) maintenance_rate: f64,
    pub(super) maker_fee_rate: f64,
    pub(super) taker_fee_rate: f64,
    pub(super) funding_interval_hours: u32,
    pub(super) funding_next_apply_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(Serialize))]
pub(super) struct GateContractMetadataRow {
    name: String,
    #[serde(rename = "type")]
    contract_kind: String,
    #[serde(default)]
    contract_type: String,
    quanto_multiplier: Value,
    #[serde(default)]
    in_delisting: bool,
    status: String,
    funding_interval: i64,
    #[serde(default)]
    funding_next_apply: i64,
    #[serde(flatten)]
    order: GateContractOrderMetadata,
    #[serde(flatten)]
    risk: GateContractRiskMetadata,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(Serialize))]
struct GateContractOrderMetadata {
    order_price_round: Value,
    order_size_min: Value,
    order_size_max: Value,
    market_order_size_max: Value,
    #[serde(default)]
    enable_decimal: bool,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(Serialize))]
struct GateContractRiskMetadata {
    leverage_min: Value,
    leverage_max: Value,
    maintenance_rate: Value,
    maker_fee_rate: Value,
    taker_fee_rate: Value,
}

impl GateContractSpec {
    fn from_row(row: &GateContractMetadataRow, endpoint_settle: &str) -> ExchangeResult<Self> {
        let identity = GateContractIdentity::from_endpoint(endpoint_settle, &row.name)?;
        if !row.contract_kind.eq_ignore_ascii_case("direct") {
            return Err(validation_error(format!(
                "gate contract {} unsupported type {}",
                identity.native_symbol, row.contract_kind
            )));
        }
        let min_qty = if row.order.enable_decimal {
            non_negative_number(&row.order.order_size_min, "order_size_min")?
        } else {
            positive_number(&row.order.order_size_min, "order_size_min")?
        };
        let max_qty = positive_number(&row.order.order_size_max, "order_size_max")?;
        if min_qty > max_qty {
            return Err(validation_error(format!(
                "gate contract {} order_size_min exceeds order_size_max",
                identity.native_symbol
            )));
        }
        let market_max_qty = market_max_qty(&row.order.market_order_size_max, max_qty)?;
        let leverage_min = positive_number(&row.risk.leverage_min, "leverage_min")?;
        let leverage_max = positive_number(&row.risk.leverage_max, "leverage_max")?;
        if leverage_min > leverage_max {
            return Err(validation_error(format!(
                "gate contract {} leverage_min exceeds leverage_max",
                identity.native_symbol
            )));
        }
        let funding_interval_hours = funding_interval_hours(row.funding_interval)?;
        Ok(Self {
            identity,
            asset_class: contract_asset_class(&row.contract_type),
            contract_size: positive_number(&row.quanto_multiplier, "quanto_multiplier")?,
            price_tick: positive_number(&row.order.order_price_round, "order_price_round")?,
            // Gate only guarantees integer lots when `enable_decimal=false`.
            // The contract response has no decimal lot increment, so decimal
            // contracts remain observation-only instead of inventing a step.
            qty_step: (!row.order.enable_decimal).then_some(1.0),
            min_qty,
            max_qty,
            market_max_qty,
            listing_status: listing_status(&row.status, row.in_delisting),
            leverage_min,
            leverage_max,
            maintenance_rate: positive_number(&row.risk.maintenance_rate, "maintenance_rate")?,
            maker_fee_rate: finite_number(&row.risk.maker_fee_rate, "maker_fee_rate")?,
            taker_fee_rate: non_negative_number(&row.risk.taker_fee_rate, "taker_fee_rate")?,
            funding_interval_hours,
            funding_next_apply_ms: row
                .funding_next_apply
                .checked_mul(1000)
                .filter(|value| *value > 0),
        })
    }

    fn into_venue_instrument(self, checked_at_ms: i64) -> VenueInstrument {
        let execution_supported = self.has_complete_execution_metadata();
        let settle = self.identity.settle.to_ascii_uppercase();
        let canonical = strip_common_suffixes(&self.identity.native_symbol);
        let quote = native_quote(&self.identity.native_symbol);
        VenueInstrument {
            venue: "gate".to_owned(),
            native_symbol: self.identity.native_symbol,
            canonical_symbol: canonical.clone(),
            display_symbol: format!("{canonical}-{settle} Perp"),
            asset_class: self.asset_class,
            product_type: Some("perp".to_owned()),
            quote_asset: quote,
            settle_asset: Some(settle.clone()),
            margin_asset: Some(settle),
            contract_size: Some(self.contract_size),
            execution_supported,
            price_tick: Some(self.price_tick),
            qty_step: self.qty_step,
            min_qty: (self.min_qty > 0.0).then_some(self.min_qty),
            min_notional: None,
            listing_status: self.listing_status,
            funding_interval_ms: Some(i64::from(self.funding_interval_hours) * 3_600_000),
            builder_dex: None,
            source: InstrumentMetadataSource::OfficialEndpoint,
            source_url: Some(format!(
                "/api/v4/futures/{}/contracts",
                self.identity.settle
            )),
            checked_at_ms,
            schema_version: Some(CONTRACT_SCHEMA_VERSION.to_owned()),
        }
    }

    fn has_complete_execution_metadata(&self) -> bool {
        valid_native_symbol(&self.identity.native_symbol)
            && self.qty_step.as_ref().is_some_and(valid_positive)
            && valid_positive(&self.min_qty)
            && valid_positive(&self.max_qty)
            && self.max_qty >= self.min_qty
            && valid_positive(&self.market_max_qty)
            && self.market_max_qty <= self.max_qty
            && valid_positive(&self.leverage_min)
            && valid_positive(&self.leverage_max)
            && self.leverage_min <= self.leverage_max
            && valid_positive(&self.maintenance_rate)
            && self.maker_fee_rate.is_finite()
            && self.taker_fee_rate.is_finite()
            && self.taker_fee_rate >= 0.0
    }
}

impl GateContractIdentity {
    fn from_endpoint(settle: &str, native_symbol: &str) -> ExchangeResult<Self> {
        let settle = settle.trim().to_ascii_lowercase();
        if !matches!(settle.as_str(), "usdt" | "btc") {
            return Err(validation_error(format!(
                "gate unsupported futures settle {settle}"
            )));
        }
        let native_symbol = native_symbol.trim().to_ascii_uppercase();
        if !valid_contract_identity(&native_symbol) {
            return Err(validation_error(format!(
                "gate invalid native contract {native_symbol}"
            )));
        }
        let expected_quote = settle.to_ascii_uppercase();
        if native_quote(&native_symbol).as_deref() != Some(expected_quote.as_str()) {
            return Err(validation_error(format!(
                "gate contract {native_symbol} does not match endpoint settle {settle}"
            )));
        }
        Ok(Self {
            settle,
            native_symbol,
        })
    }
}

#[derive(Debug, Default)]
pub(super) struct GateContractCache {
    specs_by_native: DashMap<String, GateContractSpec>,
    native_by_normalized: DashMap<String, String>,
    verified_at_by_native: DashMap<String, i64>,
    fetched_at_ms: AtomicI64,
    refresh_lock: tokio::sync::Mutex<()>,
}

impl GateContractCache {
    pub(super) fn cached_native_symbol(&self, symbol: &str) -> Option<String> {
        let native = symbol.trim().to_ascii_uppercase();
        if self.specs_by_native.contains_key(&native) && self.native_is_fresh(&native) {
            return Some(native);
        }
        let normalized = strip_common_suffixes(symbol);
        self.native_by_normalized
            .get(&normalized)
            .map(|value| value.clone())
            .filter(|native| self.native_is_fresh(native))
    }

    pub(super) async fn order_unit(
        &self,
        http: &HttpClient,
        base_url: &str,
        symbol: &str,
    ) -> ExchangeResult<f64> {
        let native = self.verified_native_symbol(http, base_url, symbol).await?;
        self.get_unit(&native).ok_or_else(|| {
            validation_error(format!("gate missing verified contract size for {native}"))
        })
    }

    pub(super) async fn market_unit(
        &self,
        http: &HttpClient,
        base_url: &str,
        symbol: &str,
    ) -> ExchangeResult<f64> {
        let native = self.verified_native_symbol(http, base_url, symbol).await?;
        self.specs_by_native
            .get(&native)
            .map(|spec| spec.contract_size)
            .filter(valid_positive)
            .ok_or_else(|| validation_error(format!("gate missing contract size for {native}")))
    }

    pub(super) async fn verified_native_symbol(
        &self,
        http: &HttpClient,
        base_url: &str,
        symbol: &str,
    ) -> ExchangeResult<String> {
        let candidate = executable_candidate(symbol)?;
        if let Some(native) = self.cached_native_symbol(&candidate) {
            return Ok(native);
        }
        let row = fetch_contract(http, base_url, DEFAULT_SETTLE, &candidate).await?;
        let spec = GateContractSpec::from_row(&row, DEFAULT_SETTLE)?;
        if spec.identity.native_symbol != candidate {
            return Err(validation_error(format!(
                "gate contract identity mismatch: requested {candidate}, received {}",
                spec.identity.native_symbol
            )));
        }
        let native = spec.identity.native_symbol.clone();
        self.insert_spec(spec);
        Ok(native)
    }

    pub(super) async fn refresh_all(
        &self,
        http: &HttpClient,
        base_url: &str,
    ) -> ExchangeResult<()> {
        if self.has_fresh_all_cache() {
            return Ok(());
        }
        let _refresh_guard = self.refresh_lock.lock().await;
        if self.has_fresh_all_cache() {
            return Ok(());
        }
        let rows = fetch_contracts(http, base_url, DEFAULT_SETTLE).await?;
        let specs: Vec<_> = rows
            .into_iter()
            .filter_map(
                |row| match GateContractSpec::from_row(&row, DEFAULT_SETTLE) {
                    Ok(spec) => Some(spec),
                    Err(error) => {
                        tracing::warn!(%error, "gate contract metadata row rejected");
                        None
                    }
                },
            )
            .collect();
        if specs.is_empty() {
            return Err(validation_error(
                "gate contract metadata contained no valid contracts".to_owned(),
            ));
        }
        for spec in specs {
            self.insert_spec(spec);
        }
        self.fetched_at_ms.store(now_ms(), Ordering::Release);
        Ok(())
    }

    pub(super) fn venue_instruments(&self, checked_at_ms: i64) -> Vec<VenueInstrument> {
        self.specs_by_native
            .iter()
            .map(|entry| entry.value().clone().into_venue_instrument(checked_at_ms))
            .collect()
    }

    pub(super) fn get_unit(&self, symbol: &str) -> Option<f64> {
        let native = self
            .cached_native_symbol(symbol)
            .unwrap_or_else(|| symbol.trim().to_ascii_uppercase());
        self.specs_by_native
            .get(&native)
            .filter(|spec| spec.has_complete_execution_metadata())
            .map(|spec| spec.contract_size)
            .filter(valid_positive)
    }

    pub(super) fn funding_interval_hours(&self, symbol: &str) -> Option<u32> {
        let native = self.cached_native_symbol(symbol)?;
        self.specs_by_native
            .get(&native)
            .map(|spec| spec.funding_interval_hours)
            .filter(|hours| *hours > 0)
    }

    pub(super) fn funding_schedule(&self, symbol: &str, at_ms: i64) -> Option<(u32, i64)> {
        let native = self.cached_native_symbol(symbol)?;
        let spec = self.specs_by_native.get(&native)?;
        if spec.listing_status != InstrumentListingStatus::Trading {
            return None;
        }
        let interval_hours = spec.funding_interval_hours;
        let interval_ms = i64::from(interval_hours).checked_mul(3_600_000)?;
        let next_apply_ms = roll_schedule_forward(spec.funding_next_apply_ms?, interval_ms, at_ms)?;
        Some((interval_hours, next_apply_ms))
    }

    fn insert_spec(&self, spec: GateContractSpec) {
        let native = spec.identity.native_symbol.clone();
        self.native_by_normalized
            .insert(strip_common_suffixes(&native), native.clone());
        self.verified_at_by_native.insert(native.clone(), now_ms());
        self.specs_by_native.insert(native, spec);
    }

    #[cfg(test)]
    pub(super) fn seed_unit(&self, symbol: &str, unit: f64) {
        let spec = test_spec(symbol, unit);
        self.insert_spec(spec);
        self.fetched_at_ms.store(now_ms(), Ordering::Release);
    }

    fn has_fresh_all_cache(&self) -> bool {
        let last = self.fetched_at_ms.load(Ordering::Acquire);
        !self.specs_by_native.is_empty()
            && last != 0
            && now_ms().saturating_sub(last) < CONTRACT_CACHE_TTL_MS
    }

    fn native_is_fresh(&self, native: &str) -> bool {
        self.verified_at_by_native
            .get(native)
            .is_some_and(|verified_at| {
                now_ms().saturating_sub(*verified_at) < CONTRACT_CACHE_TTL_MS
            })
    }
}

pub(super) fn native_symbol_hint(symbol: &str) -> Option<String> {
    let upper = symbol.trim().to_ascii_uppercase();
    if valid_native_symbol(&upper) {
        return Some(upper);
    }
    for separator in ['/', '-'] {
        let mut parts = upper.split(separator);
        let Some(base) = parts.next() else {
            continue;
        };
        let Some(quote) = parts.next() else {
            continue;
        };
        if parts.next().is_none() && valid_symbol_part(base) && valid_symbol_part(quote) {
            return Some(format!("{base}_{quote}"));
        }
    }
    None
}

fn executable_candidate(symbol: &str) -> ExchangeResult<String> {
    if let Some(native) = native_symbol_hint(symbol) {
        let quote = native_quote(&native).unwrap_or_default();
        if !quote.eq_ignore_ascii_case(DEFAULT_SETTLE) {
            return Err(validation_error(format!(
                "gate contract {native} quote/settle {quote} cannot use futures/{DEFAULT_SETTLE}"
            )));
        }
        return Ok(native);
    }
    let normalized = strip_common_suffixes(symbol);
    if !valid_symbol_part(&normalized) {
        return Err(validation_error(format!(
            "gate invalid canonical contract {symbol}"
        )));
    }
    Ok(format!(
        "{normalized}_{}",
        DEFAULT_SETTLE.to_ascii_uppercase()
    ))
}

async fn fetch_contracts(
    http: &HttpClient,
    base_url: &str,
    settle: &str,
) -> ExchangeResult<Vec<GateContractMetadataRow>> {
    let url = format!("{base_url}/api/v4/futures/{settle}/contracts");
    let resp = http.execute_with_retry(|| futures_get(http, &url)).await?;
    resp.json().await.map_err(|error| parse_err(&error))
}

async fn fetch_contract(
    http: &HttpClient,
    base_url: &str,
    settle: &str,
    symbol: &str,
) -> ExchangeResult<GateContractMetadataRow> {
    let url = format!("{base_url}/api/v4/futures/{settle}/contracts/{symbol}");
    let resp = http.execute_with_retry(|| futures_get(http, &url)).await?;
    resp.json().await.map_err(|error| parse_err(&error))
}

fn listing_status(status: &str, in_delisting: bool) -> InstrumentListingStatus {
    if in_delisting {
        return InstrumentListingStatus::Delisted;
    }
    match status.trim().to_ascii_lowercase().as_str() {
        "trading" => InstrumentListingStatus::Trading,
        "prelaunch" => InstrumentListingStatus::PreLaunch,
        "delisting" | "delisted" => InstrumentListingStatus::Delisted,
        "circuit_breaker" => InstrumentListingStatus::Suspended,
        _ => InstrumentListingStatus::Unknown,
    }
}

fn contract_asset_class(contract_type: &str) -> InstrumentAssetClass {
    match contract_type.trim().to_ascii_lowercase().as_str() {
        "" => InstrumentAssetClass::Crypto,
        "stocks" => InstrumentAssetClass::Equity,
        "metals" => InstrumentAssetClass::Metal,
        "indices" => InstrumentAssetClass::Index,
        "forex" => InstrumentAssetClass::Forex,
        _ => InstrumentAssetClass::Unknown,
    }
}

fn market_max_qty(value: &Value, order_max: f64) -> ExchangeResult<f64> {
    let parsed = non_negative_number(value, "market_order_size_max")?;
    if parsed == 0.0 {
        Ok(order_max)
    } else {
        // Gate documents zero as a fallback. For any explicit value, enforce
        // the stricter of the generic and market-order caps.
        Ok(parsed.min(order_max))
    }
}

fn funding_interval_hours(seconds: i64) -> ExchangeResult<u32> {
    if seconds <= 0 || seconds % 3_600 != 0 {
        return Err(validation_error(format!(
            "gate invalid funding_interval {seconds}"
        )));
    }
    u32::try_from(seconds / 3_600)
        .ok()
        .filter(|hours| (1..=24).contains(hours))
        .ok_or_else(|| validation_error(format!("gate invalid funding_interval {seconds}")))
}

fn roll_schedule_forward(anchor_ms: i64, interval_ms: i64, at_ms: i64) -> Option<i64> {
    if anchor_ms <= 0 || interval_ms <= 0 {
        return None;
    }
    if anchor_ms > at_ms {
        return Some(anchor_ms);
    }
    let periods = at_ms
        .saturating_sub(anchor_ms)
        .checked_div(interval_ms)?
        .checked_add(1)?;
    anchor_ms.checked_add(periods.checked_mul(interval_ms)?)
}

fn positive_number(value: &Value, field: &str) -> ExchangeResult<f64> {
    let number = finite_number(value, field)?;
    if number > 0.0 {
        Ok(number)
    } else {
        Err(validation_error(format!(
            "gate contract {field} must be positive: {number}"
        )))
    }
}

fn non_negative_number(value: &Value, field: &str) -> ExchangeResult<f64> {
    let number = finite_number(value, field)?;
    if number >= 0.0 {
        Ok(number)
    } else {
        Err(validation_error(format!(
            "gate contract {field} must be non-negative: {number}"
        )))
    }
}

fn finite_number(value: &Value, field: &str) -> ExchangeResult<f64> {
    let parsed = match value {
        Value::String(text) => text.parse::<f64>(),
        Value::Number(number) => number.to_string().parse::<f64>(),
        _ => {
            return Err(validation_error(format!(
                "gate contract invalid {field} type"
            )))
        }
    }
    .map_err(|error| validation_error(format!("gate contract invalid {field}: {error}")))?;
    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(validation_error(format!(
            "gate contract {field} must be finite"
        )))
    }
}

fn native_quote(native_symbol: &str) -> Option<String> {
    native_symbol
        .rsplit_once('_')
        .map(|(_, quote)| quote.to_owned())
}

fn valid_native_symbol(symbol: &str) -> bool {
    let mut parts = symbol.split('_');
    matches!((parts.next(), parts.next(), parts.next()), (Some(base), Some(quote), None) if valid_symbol_part(base) && valid_symbol_part(quote))
}

fn valid_contract_identity(symbol: &str) -> bool {
    symbol
        .rsplit_once('_')
        .is_some_and(|(base, quote)| valid_contract_base(base) && valid_symbol_part(quote))
}

fn valid_contract_base(value: &str) -> bool {
    !value.is_empty() && value.chars().all(char::is_alphanumeric)
}

fn valid_symbol_part(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn valid_positive(value: &f64) -> bool {
    value.is_finite() && *value > 0.0
}

#[cfg(test)]
fn test_spec(symbol: &str, unit: f64) -> GateContractSpec {
    GateContractSpec {
        identity: GateContractIdentity {
            settle: DEFAULT_SETTLE.to_owned(),
            native_symbol: symbol.to_ascii_uppercase(),
        },
        asset_class: InstrumentAssetClass::Crypto,
        contract_size: unit,
        price_tick: 1.0,
        qty_step: Some(1.0),
        min_qty: 1.0,
        max_qty: 1.0,
        market_max_qty: 1.0,
        listing_status: InstrumentListingStatus::Trading,
        leverage_min: 1.0,
        leverage_max: 1.0,
        maintenance_rate: 0.01,
        maker_fee_rate: 0.0,
        taker_fee_rate: 0.0,
        funding_interval_hours: 8,
        funding_next_apply_ms: Some(now_ms() + 8 * 3_600_000),
    }
}

#[cfg(test)]
#[path = "gate_contracts_tests.rs"]
mod tests;
