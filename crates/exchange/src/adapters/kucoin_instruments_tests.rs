use super::*;
use pretty_assertions::assert_eq;

#[derive(serde::Deserialize)]
struct ContractsActiveFixture {
    data: Vec<KucoinContractRow>,
}

fn matrix_rows() -> Vec<KucoinContractRow> {
    let fixture = include_str!("../../fixtures/kucoin/contracts_active_native_matrix.json");
    serde_json::from_str::<ContractsActiveFixture>(fixture)
        .expect("kucoin native matrix fixture")
        .data
}

#[test]
fn official_matrix_maps_usdt_usdc_and_verified_equity_contracts() {
    let _fixture = include_str!("../../fixtures/kucoin/contracts_active_native_matrix.json");
    let rows = matrix_rows();
    let instruments = instruments_from_rows(&rows, 1_700_000_000_000);

    assert_eq!(instruments.len(), 3);
    assert_contract(&instruments, &ExpectedContract::xbt_usdt());
    assert_contract(&instruments, &ExpectedContract::xbt_usdc());
    assert_contract(&instruments, &ExpectedContract::nvda_usdt());
    assert!(!instruments
        .iter()
        .any(|instrument| instrument.native_symbol == "XBTUSDM"));
}

struct ExpectedContract {
    native: &'static str,
    canonical: &'static str,
    quote: &'static str,
    contract_size: f64,
    price_tick: f64,
    asset_class: InstrumentAssetClass,
}

impl ExpectedContract {
    fn xbt_usdt() -> Self {
        Self::new(
            "XBTUSDTM",
            "BTC",
            "USDT",
            0.001,
            0.1,
            InstrumentAssetClass::Crypto,
        )
    }

    fn xbt_usdc() -> Self {
        Self::new(
            "XBTUSDCM",
            "BTC",
            "USDC",
            0.0001,
            0.1,
            InstrumentAssetClass::Crypto,
        )
    }

    fn nvda_usdt() -> Self {
        Self::new(
            "NVDAUSDTM",
            "NVDA",
            "USDT",
            0.01,
            0.01,
            InstrumentAssetClass::Equity,
        )
    }

    fn new(
        native: &'static str,
        canonical: &'static str,
        quote: &'static str,
        contract_size: f64,
        price_tick: f64,
        asset_class: InstrumentAssetClass,
    ) -> Self {
        Self {
            native,
            canonical,
            quote,
            contract_size,
            price_tick,
            asset_class,
        }
    }
}

fn assert_contract(instruments: &[VenueInstrument], expected: &ExpectedContract) {
    let instrument = instruments
        .iter()
        .find(|instrument| instrument.native_symbol == expected.native)
        .expect("instrument");
    assert_contract_identity(instrument, expected);
    assert_contract_sizing(instrument, expected);
    assert_contract_provenance(instrument);
}

fn assert_contract_identity(instrument: &VenueInstrument, expected: &ExpectedContract) {
    assert_eq!(instrument.canonical_symbol, expected.canonical);
    assert_eq!(instrument.quote_asset.as_deref(), Some(expected.quote));
    assert_eq!(instrument.settle_asset.as_deref(), Some(expected.quote));
    assert_eq!(instrument.margin_asset.as_deref(), Some(expected.quote));
    assert_eq!(instrument.asset_class, expected.asset_class);
    assert_eq!(instrument.listing_status, InstrumentListingStatus::Trading);
}

fn assert_contract_sizing(instrument: &VenueInstrument, expected: &ExpectedContract) {
    assert_eq!(instrument.contract_size, Some(expected.contract_size));
    assert_eq!(instrument.price_tick, Some(expected.price_tick));
    assert_eq!(instrument.qty_step, Some(1.0));
    assert_eq!(instrument.min_qty, Some(1.0));
    assert_eq!(instrument.min_notional, None);
    assert_eq!(instrument.funding_interval_ms, Some(28_800_000));
}

fn assert_contract_provenance(instrument: &VenueInstrument) {
    assert_eq!(
        instrument.source,
        InstrumentMetadataSource::OfficialEndpoint
    );
    assert_eq!(instrument.source_url.as_deref(), Some(SOURCE_URL));
    assert_eq!(instrument.schema_version.as_deref(), Some(SCHEMA_VERSION));
    assert!(instrument.is_hedge_constructible());
}

#[test]
fn incomplete_native_sizing_constraints_fail_closed() {
    let mut rows = matrix_rows();
    let xbt = rows
        .iter_mut()
        .find(|row| row.symbol == "XBTUSDTM")
        .expect("xbt row");
    xbt.market_max_order_qty = serde_json::Value::Null;

    let instruments = instruments_from_rows(&rows, 1_700_000_000_000);
    assert!(!instruments
        .iter()
        .any(|instrument| instrument.native_symbol == "XBTUSDTM"));
}

#[test]
fn inconsistent_lot_and_max_constraints_fail_closed() {
    let mut rows = matrix_rows();
    let usdc = rows
        .iter_mut()
        .find(|row| row.symbol == "XBTUSDCM")
        .expect("usdc row");
    usdc.lot_size = serde_json::json!(2);
    usdc.max_order_qty = serde_json::json!(1);

    let instruments = instruments_from_rows(&rows, 1_700_000_000_000);
    assert!(!instruments
        .iter()
        .any(|instrument| instrument.native_symbol == "XBTUSDCM"));
}
