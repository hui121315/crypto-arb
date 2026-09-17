//! KuCoin Futures instrument metadata backed by the native contract schema.
//!
//! Official source: <https://www.kucoin.com/docs-new/rest/futures-trading/market-data/get-all-symbols>
//! `multiplier`, `tickSize`, `lotSize`, `maxOrderQty`, and
//! `marketMaxOrderQty` are validated together by `contract_spec`; incomplete,
//! inverse, or internally inconsistent rows never become executable entries.

use super::kucoin_market_data::{
    contract_identity, contract_spec, ContractActive, KucoinContractSpec,
};
use shared_types::instrument_registry::{InstrumentAssetClass, VenueInstrument};
use shared_types::instruments::{InstrumentListingStatus, InstrumentMetadataSource};

const NAME: &str = "kucoin";
const SOURCE_URL: &str =
    "https://www.kucoin.com/docs-new/rest/futures-trading/market-data/get-all-symbols";
const SCHEMA_VERSION: &str = "kucoin-futures-native-contract-matrix-2026-07-11";

pub(super) type KucoinContractRow = ContractActive;

pub(super) fn instruments_from_rows(
    rows: &[KucoinContractRow],
    checked_at_ms: i64,
) -> Vec<VenueInstrument> {
    rows.iter()
        .filter_map(|row| instrument_from_row(row, checked_at_ms))
        .collect()
}

fn instrument_from_row(row: &KucoinContractRow, checked_at_ms: i64) -> Option<VenueInstrument> {
    let asset_class = asset_class(&row.market_type)?;
    let identity = contract_identity(row)?;
    let spec = contract_spec(row)?;
    Some(venue_instrument(
        identity,
        &spec,
        asset_class,
        checked_at_ms,
    ))
}

fn venue_instrument(
    identity: super::kucoin_market_data::KucoinContractIdentity,
    spec: &KucoinContractSpec,
    asset_class: InstrumentAssetClass,
    checked_at_ms: i64,
) -> VenueInstrument {
    let quote = identity.quote_currency;
    let settle = identity.settle_currency;
    VenueInstrument {
        venue: NAME.to_owned(),
        native_symbol: identity.native_symbol,
        canonical_symbol: identity.normalized_symbol.clone(),
        display_symbol: format!("{}-{quote} Perp", identity.normalized_symbol),
        asset_class,
        product_type: Some("perp".to_owned()),
        quote_asset: Some(quote),
        settle_asset: Some(settle.clone()),
        margin_asset: Some(settle),
        contract_size: Some(spec.order_unit),
        execution_supported: true,
        price_tick: Some(spec.price_tick),
        qty_step: Some(spec.lot_size),
        min_qty: Some(spec.lot_size),
        min_notional: None,
        listing_status: InstrumentListingStatus::Trading,
        funding_interval_ms: spec.funding_interval_ms,
        builder_dex: None,
        source: InstrumentMetadataSource::OfficialEndpoint,
        source_url: Some(SOURCE_URL.to_owned()),
        checked_at_ms,
        schema_version: Some(SCHEMA_VERSION.to_owned()),
    }
}

fn asset_class(market_type: &str) -> Option<InstrumentAssetClass> {
    match market_type.trim().to_ascii_uppercase().as_str() {
        "CRYPTO" => Some(InstrumentAssetClass::Crypto),
        "NASDAQ" => Some(InstrumentAssetClass::Equity),
        _ => None,
    }
}

#[cfg(test)]
#[path = "kucoin_instruments_tests.rs"]
mod tests;
