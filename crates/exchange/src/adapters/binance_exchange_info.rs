//! Binance USDT-M `exchangeInfo` metadata parsing and cache types.

use super::binance_format::usdm_symbol_candidates;
use crate::error::{ExchangeError, ExchangeResult};
use dashmap::DashMap;
use serde::Deserialize;
use shared_types::instrument_registry::{InstrumentAssetClass, VenueInstrument};
use shared_types::instruments::{InstrumentListingStatus, InstrumentMetadataSource};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};

/// `/fapi/v1/exchangeInfo` cache.
///
/// The endpoint returns all symbol filters in one large response. Keeping the
/// TTL close to the adapter avoids per-order metadata fetches without changing
/// Binance order semantics.
#[derive(Debug, Default)]
pub(super) struct ExchangeInfoCache {
    specs: DashMap<String, BinanceInstrumentSpec>,
    fetched_at_ms: AtomicI64,
}

pub(super) const EXCHANGE_INFO_TTL_MS: i64 = 6 * 60 * 60 * 1000; // 6h
const INSTRUMENT_SCHEMA_VERSION: &str =
    "binance-usdm-futures-exchange-information-usdt-usdc-2026-07-11";

impl ExchangeInfoCache {
    pub(super) fn is_fresh(&self, now_ms: i64) -> bool {
        let fetched_at = self.fetched_at_ms.load(Ordering::Relaxed);
        fetched_at != 0
            && now_ms.saturating_sub(fetched_at) < EXCHANGE_INFO_TTL_MS
            && !self.specs.is_empty()
    }

    pub(super) fn resolve(&self, requested: &str) -> ExchangeResult<BinanceInstrumentSpec> {
        resolve_instrument_spec(&self.specs, requested)
    }

    pub(super) fn replace(
        &self,
        specs: HashMap<String, BinanceInstrumentSpec>,
        fetched_at_ms: i64,
    ) {
        self.specs.clear();
        for (symbol, spec) in specs {
            self.specs.insert(symbol, spec);
        }
        self.fetched_at_ms.store(fetched_at_ms, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone)]
pub(super) struct BinanceInstrumentSpec {
    pub(super) native_symbol: String,
    pub(super) contract_size: f64,
    pub(super) constraints: BinanceOrderConstraints,
}

impl BinanceInstrumentSpec {
    fn from_symbol(symbol: &ExchangeInfoSymbol) -> Self {
        let native_symbol = symbol.symbol.to_ascii_uppercase();
        Self {
            native_symbol,
            // USD-M order quantity is denominated in base-asset units.
            contract_size: 1.0,
            constraints: BinanceOrderConstraints::from_symbol(symbol),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ExchangeInfoResponse {
    symbols: Vec<ExchangeInfoSymbol>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExchangeInfoSymbol {
    symbol: String,
    #[serde(default)]
    base_asset: String,
    #[serde(default)]
    quote_asset: String,
    #[serde(default)]
    margin_asset: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    contract_type: String,
    #[serde(default)]
    underlying_type: String,
    #[serde(default)]
    order_types: Vec<String>,
    #[serde(default)]
    time_in_force: Vec<String>,
    filters: Vec<ExchangeInfoFilter>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExchangeInfoFilter {
    #[serde(rename = "filterType")]
    filter_type: String,
    #[serde(default)]
    min_price: String,
    #[serde(default)]
    max_price: String,
    #[serde(default)]
    tick_size: String,
    #[serde(default)]
    min_qty: String,
    #[serde(default)]
    max_qty: String,
    #[serde(default)]
    step_size: String,
    #[serde(default)]
    min_notional: String,
    #[serde(default)]
    notional: String,
}

#[derive(Debug, Clone, Default)]
pub(super) struct BinanceOrderConstraints {
    pub(super) identity: BinanceInstrumentIdentity,
    pub(super) capabilities: BinanceOrderCapabilities,
    pub(super) price: BinancePriceConstraints,
    pub(super) limit_qty: BinanceQuantityConstraints,
    pub(super) market_qty: BinanceQuantityConstraints,
    pub(super) min_notional: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct BinanceInstrumentIdentity {
    pub(super) base_asset: String,
    pub(super) quote_asset: String,
    pub(super) margin_asset: String,
    pub(super) underlying_type: String,
}

#[derive(Debug, Clone, Default)]
pub(super) struct BinanceOrderCapabilities {
    pub(super) is_trading: bool,
    pub(super) is_registry_perpetual: bool,
    pub(super) is_perpetual: bool,
    pub(super) supports_limit: bool,
    pub(super) supports_market: bool,
    pub(super) supports_gtc: bool,
    pub(super) supports_ioc: bool,
    pub(super) supports_fok: bool,
    pub(super) supports_gtx: bool,
}

#[derive(Debug, Clone, Default)]
pub(super) struct BinancePriceConstraints {
    pub(super) min_price: Option<f64>,
    pub(super) max_price: Option<f64>,
    pub(super) tick_size: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct BinanceQuantityConstraints {
    pub(super) present: bool,
    pub(super) min_qty: Option<f64>,
    pub(super) max_qty: Option<f64>,
    pub(super) step_size: Option<f64>,
}

impl BinanceOrderConstraints {
    fn from_symbol(symbol: &ExchangeInfoSymbol) -> Self {
        let mut out = Self::from_filters(&symbol.filters);
        out.identity.base_asset.clone_from(&symbol.base_asset);
        out.identity.quote_asset.clone_from(&symbol.quote_asset);
        out.identity.margin_asset.clone_from(&symbol.margin_asset);
        out.identity
            .underlying_type
            .clone_from(&symbol.underlying_type);
        out.capabilities.is_trading = symbol.status.eq_ignore_ascii_case("TRADING");
        out.capabilities.is_registry_perpetual = matches!(
            symbol.contract_type.to_ascii_uppercase().as_str(),
            "PERPETUAL" | "TRADIFI_PERPETUAL"
        );
        out.capabilities.is_perpetual = symbol.contract_type.eq_ignore_ascii_case("PERPETUAL");
        out.capabilities.supports_limit = contains_ascii(&symbol.order_types, "LIMIT");
        out.capabilities.supports_market = contains_ascii(&symbol.order_types, "MARKET");
        out.capabilities.supports_gtc = contains_ascii(&symbol.time_in_force, "GTC");
        out.capabilities.supports_ioc = contains_ascii(&symbol.time_in_force, "IOC");
        out.capabilities.supports_fok = contains_ascii(&symbol.time_in_force, "FOK");
        out.capabilities.supports_gtx = contains_ascii(&symbol.time_in_force, "GTX");
        out
    }

    fn from_filters(filters: &[ExchangeInfoFilter]) -> Self {
        let mut out = Self::default();
        for filter in filters {
            match filter.filter_type.as_str() {
                "PRICE_FILTER" => {
                    out.price = price_constraints(filter);
                }
                "LOT_SIZE" => {
                    out.limit_qty = quantity_constraints(filter);
                }
                "MARKET_LOT_SIZE" => {
                    out.market_qty = quantity_constraints(filter);
                }
                "MIN_NOTIONAL" => {
                    out.min_notional = positive_f64(&filter.notional)
                        .or_else(|| positive_f64(&filter.min_notional));
                }
                _ => {}
            }
        }
        out
    }
}

fn price_constraints(filter: &ExchangeInfoFilter) -> BinancePriceConstraints {
    BinancePriceConstraints {
        min_price: positive_f64(&filter.min_price),
        max_price: positive_f64(&filter.max_price),
        tick_size: positive_f64(&filter.tick_size),
    }
}

fn quantity_constraints(filter: &ExchangeInfoFilter) -> BinanceQuantityConstraints {
    BinanceQuantityConstraints {
        present: true,
        min_qty: positive_f64(&filter.min_qty),
        max_qty: positive_f64(&filter.max_qty),
        step_size: positive_f64(&filter.step_size),
    }
}

pub(super) fn instrument_specs_from_response(
    info: ExchangeInfoResponse,
) -> HashMap<String, BinanceInstrumentSpec> {
    info.symbols
        .into_iter()
        .map(|symbol| {
            let spec = BinanceInstrumentSpec::from_symbol(&symbol);
            (spec.native_symbol.clone(), spec)
        })
        .collect()
}

pub(super) fn resolve_instrument_spec<M>(
    specs: &M,
    requested: &str,
) -> ExchangeResult<BinanceInstrumentSpec>
where
    M: InstrumentSpecLookup,
{
    for candidate in usdm_symbol_candidates(requested) {
        if let Some(spec) = specs.lookup(&candidate) {
            return Ok(spec);
        }
    }
    Err(ExchangeError::UnsupportedSymbol(requested.to_owned()))
}

pub(super) trait InstrumentSpecLookup {
    fn lookup(&self, symbol: &str) -> Option<BinanceInstrumentSpec>;
}

impl InstrumentSpecLookup for DashMap<String, BinanceInstrumentSpec> {
    fn lookup(&self, symbol: &str) -> Option<BinanceInstrumentSpec> {
        self.get(symbol).map(|entry| entry.clone())
    }
}

impl InstrumentSpecLookup for HashMap<String, BinanceInstrumentSpec> {
    fn lookup(&self, symbol: &str) -> Option<BinanceInstrumentSpec> {
        self.get(symbol).cloned()
    }
}

/// 把官方 `exchangeInfo` 响应映射为 [`VenueInstrument`] 事实源——instrument
/// registry 启动期灌库用。
///
/// 纳入 USDT-M 标准永续，以及官方已挂牌的 `TRADIFI_PERPETUAL` observation row；
/// 后者在专用执行语义完成前保持 `execution_supported=false`。交割/其它非永续不进
/// registry，缺基/报价币种的脏条目丢弃。`source` 恒为
/// [`InstrumentMetadataSource::OfficialEndpoint`]，
/// `listing_status` 据 `status==TRADING` 三态归一（非 Trading → `Suspended`，
/// 下游 fail-closed sizing 闸门据此阻断）。`price_tick`/`qty_step`/`min_notional`
/// 直接取自官方 filters。
pub(super) fn instruments_from_response(
    info: &ExchangeInfoResponse,
    checked_at_ms: i64,
) -> Vec<VenueInstrument> {
    info.symbols
        .iter()
        .map(BinanceInstrumentSpec::from_symbol)
        .filter_map(|spec| instrument_from_spec(&spec, checked_at_ms))
        .collect()
}

fn instrument_from_spec(
    spec: &BinanceInstrumentSpec,
    checked_at_ms: i64,
) -> Option<VenueInstrument> {
    let constraints = &spec.constraints;
    if !constraints.capabilities.is_registry_perpetual {
        return None;
    }
    let base = constraints.identity.base_asset.trim();
    let quote = constraints.identity.quote_asset.trim();
    if base.is_empty() || quote.is_empty() {
        return None;
    }
    let listing_status = if constraints.capabilities.is_trading {
        InstrumentListingStatus::Trading
    } else {
        InstrumentListingStatus::Suspended
    };
    let margin = constraints.identity.margin_asset.trim();
    let margin_asset = (!margin.is_empty()).then(|| margin.to_owned());
    let asset_class = instrument_asset_class(constraints);
    let product_label = if constraints.capabilities.is_perpetual {
        "Perp"
    } else {
        "TradFi Perp"
    };
    Some(VenueInstrument {
        venue: "binance".to_owned(),
        native_symbol: spec.native_symbol.clone(),
        canonical_symbol: base.to_owned(),
        display_symbol: format!("{base}{quote} {product_label}"),
        asset_class,
        product_type: Some("perp".to_owned()),
        quote_asset: Some(quote.to_owned()),
        settle_asset: margin_asset.clone(),
        margin_asset,
        // Binance USDT-M 永续以基础币种计量下单数量，合约乘数为 1。
        contract_size: Some(spec.contract_size),
        execution_supported: constraints.capabilities.is_perpetual,
        price_tick: constraints.price.tick_size,
        qty_step: constraints.limit_qty.step_size,
        // Binance LOT_SIZE.minQty 即最小下单张数（合约乘数为 1，等于基础数量）。
        min_qty: constraints.limit_qty.min_qty,
        min_notional: constraints.min_notional,
        listing_status,
        funding_interval_ms: None,
        builder_dex: None,
        source: InstrumentMetadataSource::OfficialEndpoint,
        source_url: Some("/fapi/v1/exchangeInfo".to_owned()),
        checked_at_ms,
        schema_version: Some(INSTRUMENT_SCHEMA_VERSION.to_owned()),
    })
}

fn instrument_asset_class(constraints: &BinanceOrderConstraints) -> InstrumentAssetClass {
    match constraints
        .identity
        .underlying_type
        .trim()
        .to_ascii_uppercase()
        .as_str()
    {
        "EQUITY" => InstrumentAssetClass::Equity,
        "INDEX" => InstrumentAssetClass::Index,
        "METAL" => InstrumentAssetClass::Metal,
        "ENERGY" => InstrumentAssetClass::Energy,
        "FOREX" | "FX" => InstrumentAssetClass::Forex,
        _ if constraints.capabilities.is_perpetual => InstrumentAssetClass::Crypto,
        _ => InstrumentAssetClass::Unknown,
    }
}

fn contains_ascii(values: &[String], expected: &str) -> bool {
    values
        .iter()
        .any(|value| value.eq_ignore_ascii_case(expected))
}

fn positive_f64(value: &str) -> Option<f64> {
    value.parse::<f64>().ok().filter(|v| *v > 0.0)
}

#[cfg(test)]
#[path = "binance_exchange_info_tests.rs"]
mod tests;
