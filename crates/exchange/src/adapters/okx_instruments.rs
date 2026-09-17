//! OKX instrument metadata used by live order sizing.
//!
//! Official OKX V5 docs checked before moving this code:
//! - `GET /api/v5/public/instruments`
//! - `POST /api/v5/trade/order`

use crate::adapter::strip_common_suffixes;
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use common::time::now_ms;
use dashmap::DashMap;
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;
use serde::Deserialize;
use shared_types::instrument_registry::{InstrumentAssetClass, VenueInstrument};
use shared_types::instruments::{InstrumentListingStatus, InstrumentMetadataSource};
use shared_types::{OrderIntent, OrderType};
use std::sync::atomic::{AtomicI64, Ordering};

const NAME: &str = "okx";
const INSTRUMENT_SCHEMA_VERSION: &str = "okx-v5-public-get-instruments-2026-06-03";
const ALIGNMENT_EPS: f64 = 1e-9;
const CONTRACT_VALUE_TTL_MS: i64 = 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone)]
pub(super) struct OkxInstrumentRule {
    pub(super) inst_id: String,
    inst_id_code: Option<u64>,
    contract_value: f64,
    contract_value_currency: String,
    lot_size: f64,
    min_size: f64,
    tick_size: f64,
    state: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct OkxOrderSizing {
    pub(super) sz: String,
    pub(super) px: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct OkxInstrumentRow {
    #[serde(default, rename = "instId")]
    pub(super) inst_id: String,
    #[serde(default, rename = "instIdCode")]
    pub(super) inst_id_code: Option<u64>,
    #[serde(default, rename = "ctVal")]
    pub(super) contract_value: String,
    #[serde(default, rename = "ctValCcy")]
    pub(super) contract_value_currency: String,
    #[serde(default, rename = "lotSz")]
    pub(super) lot_size: String,
    #[serde(default, rename = "minSz")]
    pub(super) min_size: String,
    #[serde(default, rename = "tickSz")]
    pub(super) tick_size: String,
    #[serde(default)]
    pub(super) state: String,
}

#[derive(Debug, Default)]
pub(super) struct OkxContractValues {
    rules: DashMap<String, OkxInstrumentRule>,
    fetched_at_ms: AtomicI64,
}

impl OkxContractValues {
    pub(super) async fn refresh_all(
        &self,
        http: &HttpClient,
        base_url: &str,
    ) -> ExchangeResult<()> {
        let rows = super::okx_public_rest::swap_instruments_rest(http, base_url).await?;
        self.apply_rows(rows)
    }

    pub(super) async fn contract_value(
        &self,
        http: &HttpClient,
        base_url: &str,
        inst_id: &str,
    ) -> ExchangeResult<f64> {
        // Official contract value semantics: https://www.okx.com/docs-v5/en/#public-data-rest-api-get-instruments
        if let Some(value) = self.cached_contract_value(inst_id) {
            return Ok(value);
        }
        self.refresh_all(http, base_url).await?;
        self.cached_contract_value(inst_id)
            .ok_or_else(|| ExchangeError::UnsupportedSymbol(inst_id.to_owned()))
    }

    pub(super) fn venue_instruments(&self, checked_at_ms: i64) -> Vec<VenueInstrument> {
        let mut instruments = self
            .rules
            .iter()
            .map(|entry| entry.value().clone().into_venue_instrument(checked_at_ms))
            .collect::<Vec<_>>();
        instruments.sort_unstable_by(|left, right| left.native_symbol.cmp(&right.native_symbol));
        instruments
    }

    pub(super) fn apply_ws_rules(&self, rules: Vec<OkxInstrumentRule>) {
        for rule in rules {
            self.rules.insert(rule.inst_id.clone(), rule);
        }
    }

    fn apply_rows(&self, rows: Vec<OkxInstrumentRow>) -> ExchangeResult<()> {
        let rules = rows
            .into_iter()
            .filter_map(|row| OkxInstrumentRule::from_row(row).ok())
            .collect::<Vec<_>>();
        if rules.is_empty() {
            return Err(validation_error(
                "okx instrument metadata contained no valid linear swaps".to_owned(),
            ));
        }
        self.fetched_at_ms.store(0, Ordering::Release);
        self.rules.clear();
        for rule in rules {
            self.rules.insert(rule.inst_id.clone(), rule);
        }
        self.fetched_at_ms.store(now_ms(), Ordering::Release);
        Ok(())
    }

    fn cached_contract_value(&self, inst_id: &str) -> Option<f64> {
        if !self.is_fresh() {
            return None;
        }
        self.rules.get(inst_id).map(|rule| rule.contract_value)
    }

    fn is_fresh(&self) -> bool {
        let fetched_at = self.fetched_at_ms.load(Ordering::Acquire);
        fetched_at > 0
            && !self.rules.is_empty()
            && now_ms().saturating_sub(fetched_at) < CONTRACT_VALUE_TTL_MS
    }
}

impl OkxInstrumentRule {
    pub(super) fn from_row(row: OkxInstrumentRow) -> ExchangeResult<Self> {
        let rule = Self {
            inst_id: non_empty(row.inst_id, "instId")?,
            inst_id_code: row.inst_id_code,
            contract_value: positive_number(&row.contract_value, "ctVal")?,
            contract_value_currency: non_empty(row.contract_value_currency, "ctValCcy")?,
            lot_size: positive_number(&row.lot_size, "lotSz")?,
            min_size: positive_number(&row.min_size, "minSz")?,
            tick_size: positive_number(&row.tick_size, "tickSz")?,
            state: non_empty(row.state, "state")?,
        };
        rule.validate_contract_currency()?;
        Ok(rule)
    }

    pub(super) fn ws_inst_id_code(&self) -> ExchangeResult<u64> {
        self.inst_id_code.ok_or_else(|| {
            validation_error(format!(
                "okx instrument {} missing instIdCode required by WS order/cancel",
                self.inst_id
            ))
        })
    }

    fn validate_contract_currency(&self) -> ExchangeResult<()> {
        let base = strip_common_suffixes(&self.inst_id);
        if self.contract_value_currency.eq_ignore_ascii_case(&base) {
            return Ok(());
        }
        Err(validation_error(format!(
            "okx ctValCcy {} does not match base {} for {}; cannot size base quantity safely",
            self.contract_value_currency, base, self.inst_id
        )))
    }

    fn validate_state_for(&self, intent: &OrderIntent) -> ExchangeResult<()> {
        if self.state == "live" {
            return Ok(());
        }
        Err(validation_error(format!(
            "okx instrument {} state {} is not live for requested {:?} order",
            self.inst_id, self.state, intent.order_type
        )))
    }

    /// 映射为注册表 [`VenueInstrument`]。OKX SWAP 官方不给最小名义，只给最小
    /// 张数 `minSz`：映射为 `min_qty`（`min_notional` 留空）。`ctVal` 为每张合约
    /// 的基础币数 → `contract_size`；`validate_contract_currency` 已保证 `ctValCcy`
    /// 与 base 一致，可安全按基础数量 sizing。
    fn into_venue_instrument(self, checked_at_ms: i64) -> VenueInstrument {
        let base = strip_common_suffixes(&self.inst_id);
        let quote = quote_from_inst_id(&self.inst_id).unwrap_or_else(|| "USDT".to_owned());
        let listing_status = match self.state.as_str() {
            "live" => InstrumentListingStatus::Trading,
            "preopen" => InstrumentListingStatus::PreLaunch,
            "suspend" => InstrumentListingStatus::Suspended,
            "expired" => InstrumentListingStatus::Delisted,
            _ => InstrumentListingStatus::Unknown,
        };
        VenueInstrument {
            venue: NAME.to_owned(),
            native_symbol: self.inst_id.clone(),
            canonical_symbol: base.clone(),
            display_symbol: format!("{base}-{quote} Perp"),
            asset_class: InstrumentAssetClass::Crypto,
            product_type: Some("perp".to_owned()),
            quote_asset: Some(quote.clone()),
            settle_asset: Some(quote.clone()),
            margin_asset: Some(quote),
            // OKX `ctVal` = 每张合约基础币数；lotSz/minSz 以合约张数计。
            contract_size: Some(self.contract_value),
            execution_supported: true,
            price_tick: Some(self.tick_size),
            qty_step: Some(self.lot_size),
            min_qty: Some(self.min_size),
            min_notional: None,
            listing_status,
            funding_interval_ms: None,
            builder_dex: None,
            source: InstrumentMetadataSource::OfficialEndpoint,
            source_url: Some("/api/v5/public/instruments".to_owned()),
            checked_at_ms,
            schema_version: Some(INSTRUMENT_SCHEMA_VERSION.to_owned()),
        }
    }
}

/// 将官方 `instruments` 行映射为注册表条目；fail-closed 跳过 ctValCcy 与 base
/// 不符（如币本位合约）或缺必需精度的行，只登记能安全 sizing 的 SWAP。
#[cfg(test)]
pub(super) fn instruments_from_rows(
    rows: Vec<OkxInstrumentRow>,
    checked_at_ms: i64,
) -> Vec<VenueInstrument> {
    rows.into_iter()
        .filter_map(|row| OkxInstrumentRule::from_row(row).ok())
        .map(|rule| rule.into_venue_instrument(checked_at_ms))
        .collect()
}

/// 从 OKX instId（`BASE-QUOTE-SWAP`）取报价资产。
fn quote_from_inst_id(inst_id: &str) -> Option<String> {
    let mut parts = inst_id.split('-');
    let _base = parts.next()?;
    let quote = parts.next()?;
    (!quote.is_empty()).then(|| quote.to_ascii_uppercase())
}

pub(super) fn sizing_from_instrument(
    intent: &OrderIntent,
    rule: &OkxInstrumentRule,
) -> ExchangeResult<OkxOrderSizing> {
    rule.validate_state_for(intent)?;
    let contracts = aligned_contracts(intent.quantity, rule)?;
    Ok(OkxOrderSizing {
        sz: number_param(contracts),
        px: order_price(intent, rule)?,
    })
}

pub(super) fn number_param(value: f64) -> String {
    if let Some(decimal) = Decimal::from_f64(value) {
        return decimal.normalize().to_string();
    }
    let formatted = format!("{value:.12}");
    let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" {
        "0".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn aligned_contracts(intent_quantity: f64, rule: &OkxInstrumentRule) -> ExchangeResult<f64> {
    let quantity = positive_value(intent_quantity, "quantity")?;
    let contracts = quantity / rule.contract_value;
    if contracts + ALIGNMENT_EPS < rule.min_size {
        return Err(validation_error(format!(
            "okx quantity {quantity} is below minSz {} contracts for {}",
            rule.min_size, rule.inst_id
        )));
    }
    aligned_value(
        contracts,
        rule.lot_size,
        format!(
            "okx quantity {quantity} is not aligned to lotSz {} contracts for {}",
            rule.lot_size, rule.inst_id
        ),
    )
}

fn order_price(intent: &OrderIntent, rule: &OkxInstrumentRule) -> ExchangeResult<Option<String>> {
    match intent.order_type {
        OrderType::Market => Ok(None),
        OrderType::Limit | OrderType::PostOnly => {
            let price = intent
                .price
                .ok_or_else(|| validation_error("okx limit order requires px".to_owned()))?;
            aligned_value(
                positive_value(price, "price")?,
                rule.tick_size,
                format!(
                    "okx price {price} is not aligned to tickSz {} for {}",
                    rule.tick_size, rule.inst_id
                ),
            )?;
            Ok(Some(number_param(price)))
        }
    }
}

fn aligned_value(value: f64, step: f64, message: String) -> ExchangeResult<f64> {
    let units = value / step;
    let rounded = units.round();
    if (units - rounded).abs() <= ALIGNMENT_EPS * rounded.abs().max(1.0) {
        return Ok(rounded * step);
    }
    Err(validation_error(message))
}

fn non_empty(value: String, field: &str) -> ExchangeResult<String> {
    if value.trim().is_empty() {
        Err(validation_error(format!("okx instrument missing {field}")))
    } else {
        Ok(value)
    }
}

fn positive_number(value: &str, field: &str) -> ExchangeResult<f64> {
    let number = value
        .parse::<f64>()
        .map_err(|error| validation_error(format!("okx instrument invalid {field}: {error}")))?;
    positive_value(number, field)
}

fn positive_value(value: f64, field: &str) -> ExchangeResult<f64> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(validation_error(format!(
            "okx instrument {field} must be positive finite: {value}"
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
#[path = "okx_instruments_tests.rs"]
mod tests;
