//! Hyperliquid perpetual metadata used by the order compiler.
//!
//! Official Hyperliquid docs:
//! - <https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint/perpetuals>
//! - <https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/asset-ids>
//! - <https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/tick-and-lot-size>
//!
//! `perpCategories` is the identity source for builder-deployed assets. A dex
//! name is only a namespace and must never be used as an asset-class shortcut.

use crate::adapters::hyperliquid_config::HyperliquidMarket;
use crate::adapters::hyperliquid_market_data::clean_hyperliquid_symbol;
use crate::error::{ExchangeError, ExchangeResult};
use serde::Deserialize;
use shared_types::instrument_registry::{InstrumentAssetClass, VenueInstrument};
use shared_types::instruments::{InstrumentListingStatus, InstrumentMetadataSource};
use std::collections::HashMap;

const NAME: &str = "hyperliquid";
const SETTLE: &str = "USDC";
const SCHEMA_VERSION: &str = "hyperliquid-perpetuals-meta-and-asset-ctxs-2026-06-03";
pub(super) const SPOT_SCHEMA_VERSION: &str = "hyperliquid-spot-meta-2026-08-04";
const PERP_MAX_DECIMALS: i32 = 6;
const FUNDING_INTERVAL_MS: i64 = 60 * 60 * 1_000;
const BUILDER_ASSET_BASE: u32 = 100_000;
const BUILDER_ASSET_STRIDE: u32 = 10_000;
/// Matches the production instrument-registry refresh cadence. At expiry the
/// writer must refresh official metadata before using a cached asset ID, lot,
/// or listing status again.
pub(super) const METADATA_CACHE_TTL_MS: i64 = 6 * 60 * 60 * 1_000;

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct HyperliquidUniverseRow {
    pub(super) name: String,
    #[serde(rename = "szDecimals")]
    pub(super) sz_decimals: i32,
    #[serde(default, rename = "isDelisted")]
    pub(super) is_delisted: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct HyperliquidMeta {
    pub(super) universe: Vec<HyperliquidUniverseRow>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct HyperliquidPerpDex {
    #[serde(default)]
    pub(super) name: String,
}

pub(super) type HyperliquidPerpCategories = Vec<(String, String)>;

#[derive(Debug, Clone)]
pub(super) struct HyperliquidInstrumentMetadata {
    pub(super) meta: HyperliquidMeta,
    pub(super) builder_dex_index: Option<u32>,
    pub(super) categories: HyperliquidPerpCategories,
}

#[derive(Debug, Clone)]
pub(super) struct HyperliquidInstrumentSpec {
    pub(super) asset_id: u32,
    pub(super) canonical_symbol: String,
    pub(super) price_tick: f64,
    pub(super) qty_step: f64,
    pub(super) min_qty: f64,
    pub(super) size_decimals: u8,
    pub(super) price_decimals: u8,
    pub(super) is_trading: bool,
}

pub(super) fn spot_spec_from_instrument(
    instrument: &VenueInstrument,
) -> ExchangeResult<HyperliquidInstrumentSpec> {
    let asset_id = instrument
        .native_symbol
        .strip_prefix('@')
        .ok_or_else(|| {
            validation_error("hyperliquid spot native symbol must start with @".to_owned())
        })?
        .parse::<u32>()
        .map_err(|_| validation_error("hyperliquid spot asset id must be numeric".to_owned()))?;
    let price_tick = instrument
        .price_tick
        .ok_or_else(|| validation_error("hyperliquid spot price tick is missing".to_owned()))?;
    let qty_step = instrument
        .qty_step
        .ok_or_else(|| validation_error("hyperliquid spot quantity step is missing".to_owned()))?;
    let min_qty = instrument.min_qty.unwrap_or(qty_step);
    Ok(HyperliquidInstrumentSpec {
        asset_id,
        canonical_symbol: instrument.canonical_symbol.clone(),
        price_tick,
        qty_step,
        min_qty,
        size_decimals: step_decimals(qty_step)?,
        price_decimals: step_decimals(price_tick)?,
        is_trading: instrument.listing_status == InstrumentListingStatus::Trading,
    })
}

fn step_decimals(step: f64) -> ExchangeResult<u8> {
    if !(step.is_finite() && step > 0.0) {
        return Err(validation_error(
            "hyperliquid step must be positive".to_owned(),
        ));
    }
    let text = format!("{step:.12}");
    let decimals = text
        .trim_end_matches('0')
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    u8::try_from(decimals)
        .map_err(|_| validation_error("hyperliquid step precision is too large".to_owned()))
}

pub(super) fn spot_instruments_from_metadata(
    metadata: &crate::adapters::hyperliquid_market_data::SpotMetaWrapper,
    checked_at_ms: i64,
) -> Vec<VenueInstrument> {
    let tokens = metadata
        .tokens
        .iter()
        .map(|token| (token.index, token))
        .collect::<HashMap<_, _>>();
    metadata
        .universe
        .iter()
        .enumerate()
        .filter_map(|(spot_index, pair)| {
            let base = tokens.get(pair.tokens.first()?)?;
            let quote = tokens.get(pair.tokens.get(1)?)?;
            let max_price_decimals = 8_u8.checked_sub(base.sz_decimals)?;
            let asset_id = 10_000_u32.checked_add(u32::try_from(spot_index).ok()?)?;
            Some(VenueInstrument {
                venue: NAME.to_owned(),
                native_symbol: format!("@{asset_id}"),
                canonical_symbol: base.name.to_ascii_uppercase(),
                display_symbol: format!(
                    "{}-{} Spot",
                    base.name.to_ascii_uppercase(),
                    quote.name.to_ascii_uppercase()
                ),
                asset_class: InstrumentAssetClass::Crypto,
                product_type: Some("spot".to_owned()),
                quote_asset: Some(quote.name.to_ascii_uppercase()),
                settle_asset: None,
                margin_asset: None,
                contract_size: Some(1.0),
                execution_supported: true,
                price_tick: Some(pow10_neg(i32::from(max_price_decimals))),
                qty_step: Some(pow10_neg(i32::from(base.sz_decimals))),
                min_qty: None,
                min_notional: Some(10.0),
                listing_status: InstrumentListingStatus::Trading,
                funding_interval_ms: None,
                builder_dex: None,
                source: InstrumentMetadataSource::OfficialEndpoint,
                source_url: Some("/info {\"type\":\"spotMeta\"}".to_owned()),
                checked_at_ms,
                schema_version: Some(SPOT_SCHEMA_VERSION.to_owned()),
            })
        })
        .collect()
}

#[derive(Debug, Clone)]
pub(super) struct HyperliquidInstrumentCache {
    expires_at_ms: i64,
    specs: HashMap<String, HyperliquidInstrumentSpec>,
    instruments: Vec<VenueInstrument>,
}

impl HyperliquidInstrumentCache {
    pub(super) fn instruments(&self) -> Vec<VenueInstrument> {
        self.instruments.clone()
    }

    pub(super) fn resolve(
        &self,
        symbol: &str,
        now_ms: i64,
    ) -> ExchangeResult<HyperliquidInstrumentSpec> {
        if now_ms >= self.expires_at_ms {
            return Err(validation_error(
                "hyperliquid order metadata cache expired; refresh instruments before trading"
                    .to_owned(),
            ));
        }
        let key = clean_hyperliquid_symbol(symbol).to_ascii_uppercase();
        let spec = self
            .specs
            .get(&key)
            .cloned()
            .ok_or_else(|| ExchangeError::UnsupportedSymbol(key.clone()))?;
        if !spec.is_trading {
            return Err(validation_error(format!(
                "hyperliquid instrument {} is not trading",
                spec.canonical_symbol
            )));
        }
        if !(positive(spec.price_tick) && positive(spec.qty_step) && positive(spec.min_qty)) {
            return Err(validation_error(format!(
                "hyperliquid instrument {} is missing valid price or lot metadata",
                spec.canonical_symbol
            )));
        }
        Ok(spec)
    }
}

#[derive(Debug, Clone)]
pub(super) struct HyperliquidInstrumentRule {
    base: String,
    price_tick: f64,
    qty_step: f64,
    size_decimals: u8,
    price_decimals: u8,
    is_delisted: bool,
}

impl HyperliquidInstrumentRule {
    fn from_row_for_market(
        row: HyperliquidUniverseRow,
        builder_dex: Option<&str>,
    ) -> ExchangeResult<Self> {
        let base = canonical_base(row.name, builder_dex)?;
        if !(0..=PERP_MAX_DECIMALS).contains(&row.sz_decimals) {
            return Err(validation_error(format!(
                "hyperliquid asset {base} szDecimals {} out of supported range 0..={PERP_MAX_DECIMALS}",
                row.sz_decimals
            )));
        }
        let size_decimals = u8::try_from(row.sz_decimals).map_err(|_| {
            validation_error(format!("hyperliquid invalid size decimals for {base}"))
        })?;
        let price_decimals = u8::try_from(PERP_MAX_DECIMALS - row.sz_decimals).map_err(|_| {
            validation_error(format!("hyperliquid invalid price decimals for {base}"))
        })?;
        Ok(Self {
            base,
            qty_step: pow10_neg(row.sz_decimals),
            price_tick: pow10_neg(PERP_MAX_DECIMALS - row.sz_decimals),
            size_decimals,
            price_decimals,
            is_delisted: row.is_delisted,
        })
    }

    fn spec_for_asset(&self, asset_id: u32) -> HyperliquidInstrumentSpec {
        HyperliquidInstrumentSpec {
            asset_id,
            canonical_symbol: self.base.clone(),
            price_tick: self.price_tick,
            qty_step: self.qty_step,
            min_qty: self.qty_step,
            size_decimals: self.size_decimals,
            price_decimals: self.price_decimals,
            is_trading: !self.is_delisted,
        }
    }

    fn venue_instrument_for_market(
        &self,
        market: HyperliquidMarket,
        asset_class: InstrumentAssetClass,
        checked_at_ms: i64,
    ) -> VenueInstrument {
        let builder_dex = market.dex().map(str::to_owned);
        let native_symbol = builder_dex
            .as_deref()
            .map(|dex| format!("{dex}:{}", self.base))
            .unwrap_or_else(|| self.base.clone());
        let listing_status = if self.is_delisted {
            InstrumentListingStatus::Delisted
        } else {
            InstrumentListingStatus::Trading
        };
        let source_url = builder_dex.as_deref().map_or_else(
            || "/info {\"type\":\"meta\"}".to_owned(),
            |dex| {
                format!(
                    "/info {{\"type\":\"meta\",\"dex\":\"{dex}\"}}; /info {{\"type\":\"perpCategories\"}}"
                )
            },
        );
        VenueInstrument {
            venue: market.venue().to_owned(),
            native_symbol,
            canonical_symbol: self.base.clone(),
            display_symbol: format!("{}-{SETTLE} Perp", self.base),
            asset_class,
            product_type: Some("perp".to_owned()),
            quote_asset: Some(SETTLE.to_owned()),
            settle_asset: Some(SETTLE.to_owned()),
            margin_asset: Some(SETTLE.to_owned()),
            contract_size: Some(1.0),
            execution_supported: true,
            price_tick: Some(self.price_tick),
            qty_step: Some(self.qty_step),
            min_qty: Some(self.qty_step),
            min_notional: None,
            listing_status,
            funding_interval_ms: Some(FUNDING_INTERVAL_MS),
            builder_dex,
            source: InstrumentMetadataSource::OfficialEndpoint,
            source_url: Some(source_url),
            checked_at_ms,
            schema_version: Some(SCHEMA_VERSION.to_owned()),
        }
    }
}

pub(super) fn builder_dex_index(
    dexes: &[Option<HyperliquidPerpDex>],
    dex: &str,
) -> ExchangeResult<u32> {
    let target = dex.trim();
    if target.is_empty() {
        return Err(validation_error(
            "hyperliquid builder dex is missing from configuration".to_owned(),
        ));
    }
    let Some(index) = dexes.iter().position(|entry| {
        entry
            .as_ref()
            .is_some_and(|entry| entry.name.eq_ignore_ascii_case(target))
    }) else {
        return Err(validation_error(format!(
            "hyperliquid builder dex {target} missing from perpDexs"
        )));
    };
    let index = u32::try_from(index)
        .map_err(|_| validation_error(format!("hyperliquid dex index overflows for {target}")))?;
    if index == 0 {
        return Err(validation_error(format!(
            "hyperliquid builder dex {target} resolved to core dex index"
        )));
    }
    Ok(index)
}

pub(super) fn instrument_cache_from_metadata(
    metadata: HyperliquidInstrumentMetadata,
    market: HyperliquidMarket,
    checked_at_ms: i64,
) -> ExchangeResult<HyperliquidInstrumentCache> {
    let HyperliquidInstrumentMetadata {
        meta,
        builder_dex_index,
        categories,
    } = metadata;
    let builder_dex = market.dex();
    let builder_dex_index = match (builder_dex, builder_dex_index) {
        (None, None) => None,
        (Some(_), Some(index)) => Some(index),
        (Some(dex), None) => {
            return Err(validation_error(format!(
                "hyperliquid builder dex {dex} metadata is missing its dex index"
            )));
        }
        (None, Some(_)) => {
            return Err(validation_error(
                "hyperliquid core metadata unexpectedly includes a builder dex index".to_owned(),
            ));
        }
    };
    let asset_classes = category_asset_classes(categories)?;

    let mut specs = HashMap::new();
    let mut instruments = Vec::with_capacity(meta.universe.len());
    for (local_index, row) in meta.universe.into_iter().enumerate() {
        let asset_id = perp_asset_id(builder_dex_index, local_index)?;
        let native_name = row.name.trim().to_ascii_uppercase();
        let asset_class = instrument_asset_class(market, &native_name, &asset_classes);
        let rule = HyperliquidInstrumentRule::from_row_for_market(row, builder_dex)?;
        let spec = rule.spec_for_asset(asset_id);
        if specs.insert(spec.canonical_symbol.clone(), spec).is_some() {
            return Err(validation_error(
                "hyperliquid metadata has ambiguous canonical asset".to_owned(),
            ));
        }
        instruments.push(rule.venue_instrument_for_market(market, asset_class, checked_at_ms));
    }
    if !specs.values().any(|spec| spec.is_trading) {
        return Err(validation_error(
            "hyperliquid metadata contains no executable trading instruments".to_owned(),
        ));
    }
    let checked_at_ms = checked_at_ms.max(1);
    Ok(HyperliquidInstrumentCache {
        expires_at_ms: checked_at_ms.saturating_add(METADATA_CACHE_TTL_MS),
        specs,
        instruments,
    })
}

fn category_asset_classes(
    rows: HyperliquidPerpCategories,
) -> ExchangeResult<HashMap<String, InstrumentAssetClass>> {
    let mut classes = HashMap::with_capacity(rows.len());
    for (native_symbol, category) in rows {
        let native_symbol = non_empty(native_symbol, "perp category symbol")?.to_ascii_uppercase();
        let asset_class = category_asset_class(&category);
        if let Some(existing) = classes.insert(native_symbol.clone(), asset_class) {
            if existing != asset_class {
                return Err(validation_error(format!(
                    "hyperliquid perp category is ambiguous for {native_symbol}"
                )));
            }
        }
    }
    Ok(classes)
}

fn category_asset_class(category: &str) -> InstrumentAssetClass {
    match category.trim().to_ascii_lowercase().as_str() {
        "crypto" => InstrumentAssetClass::Crypto,
        "stock" | "stocks" => InstrumentAssetClass::Equity,
        "indices" => InstrumentAssetClass::Index,
        "fx" => InstrumentAssetClass::Forex,
        _ => InstrumentAssetClass::Unknown,
    }
}

fn instrument_asset_class(
    market: HyperliquidMarket,
    native_symbol: &str,
    categories: &HashMap<String, InstrumentAssetClass>,
) -> InstrumentAssetClass {
    if market.dex().is_none() {
        InstrumentAssetClass::Crypto
    } else {
        categories
            .get(native_symbol)
            .copied()
            .unwrap_or(InstrumentAssetClass::Unknown)
    }
}

fn perp_asset_id(builder_dex_index: Option<u32>, local_index: usize) -> ExchangeResult<u32> {
    let local_index = u32::try_from(local_index)
        .map_err(|_| validation_error("hyperliquid asset index overflows".to_owned()))?;
    let Some(dex_index) = builder_dex_index else {
        return Ok(local_index);
    };
    if local_index >= BUILDER_ASSET_STRIDE {
        return Err(validation_error(format!(
            "hyperliquid builder asset index {local_index} exceeds stride {BUILDER_ASSET_STRIDE}"
        )));
    }
    let dex_offset = dex_index
        .checked_mul(BUILDER_ASSET_STRIDE)
        .ok_or_else(|| validation_error("hyperliquid builder dex offset overflows".to_owned()))?;
    BUILDER_ASSET_BASE
        .checked_add(dex_offset)
        .and_then(|asset_id| asset_id.checked_add(local_index))
        .ok_or_else(|| validation_error("hyperliquid builder asset id overflows".to_owned()))
}

fn canonical_base(raw_name: String, builder_dex: Option<&str>) -> ExchangeResult<String> {
    let raw_name = non_empty(raw_name, "name")?;
    if let Some(dex) = builder_dex {
        let Some((prefix, base)) = raw_name.split_once(':') else {
            return Err(validation_error(format!(
                "hyperliquid builder dex {dex} asset name must include its dex prefix"
            )));
        };
        if !prefix.eq_ignore_ascii_case(dex) {
            return Err(validation_error(format!(
                "hyperliquid builder asset {raw_name} does not belong to dex {dex}"
            )));
        }
        return Ok(non_empty(base.to_owned(), "builder asset base")?.to_ascii_uppercase());
    }
    if raw_name.contains(':') {
        return Err(validation_error(format!(
            "hyperliquid core asset {raw_name} unexpectedly contains a dex prefix"
        )));
    }
    Ok(raw_name.to_ascii_uppercase())
}

fn pow10_neg(exp: i32) -> f64 {
    10f64.powi(-exp)
}

fn non_empty(value: String, field: &str) -> ExchangeResult<String> {
    if value.trim().is_empty() {
        Err(validation_error(format!(
            "hyperliquid instrument missing {field}"
        )))
    } else {
        Ok(value)
    }
}

fn positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn validation_error(message: String) -> ExchangeError {
    ExchangeError::Api {
        exchange: NAME.into(),
        code: "validation".into(),
        message,
    }
}

#[cfg(test)]
#[path = "hyperliquid_instruments_tests.rs"]
mod tests;
