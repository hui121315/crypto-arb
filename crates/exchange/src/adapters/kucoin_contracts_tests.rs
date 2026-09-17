use super::super::kucoin_market_data::parse_funding;
use super::*;
use serde_json::json;

fn contract(symbol: &str, base: &str, quote: &str, multiplier: f64) -> ContractActive {
    serde_json::from_value(json!({
        "symbol": symbol,
        "baseCurrency": base,
        "quoteCurrency": quote,
        "settleCurrency": quote,
        "multiplier": multiplier,
        "tickSize": 0.1,
        "lotSize": 1,
        "maxOrderQty": 1000,
        "marketMaxOrderQty": 500,
        "isInverse": false,
        "marketType": "CRYPTO",
        "status": "Open"
    }))
    .expect("contract row")
}

fn native_matrix() -> Vec<ContractActive> {
    let fixture = include_str!("../../fixtures/kucoin/contracts_active_native_matrix.json");
    let wrap: crate::adapters::kucoin_response::KucoinResponse<Vec<ContractActive>> =
        serde_json::from_str(fixture).expect("kucoin native matrix fixture");
    wrap.into_data("contracts/active").expect("contract rows")
}

#[test]
fn metadata_resolution_is_quote_aware_and_ambiguous_base_fails_closed() {
    let cache = KucoinContractMultipliers::default();
    cache.apply_contracts(native_matrix());

    assert_eq!(
        cache.cached_native_symbol("BTC-USDT"),
        Some("XBTUSDTM".to_owned())
    );
    assert_eq!(
        cache.cached_native_symbol("BTC-USDC"),
        Some("XBTUSDCM".to_owned())
    );
    assert_eq!(
        cache.cached_native_symbol("BTC-USD"),
        Some("XBTUSDM".to_owned())
    );
    assert_eq!(
        cache.cached_native_symbol("XBTUSDM"),
        Some("XBTUSDM".to_owned())
    );
    assert_eq!(cache.cached_native_symbol("BTC"), None);
    assert_eq!(cache.cached_unit("BTC"), None);
    assert_eq!(cache.cached_unit("BTC-USDT"), Some(0.001));
    assert_eq!(cache.cached_unit("BTC-USDC"), Some(0.0001));
    assert_eq!(cache.cached_unit("BTC-USD"), None);
}

#[test]
fn unique_special_contract_resolves_without_suffix_guessing() {
    let cache = KucoinContractMultipliers::default();
    cache.apply_contracts(native_matrix());

    assert_eq!(
        cache.cached_native_symbol("NVDA"),
        Some("NVDAUSDTM".to_owned())
    );
    assert_eq!(
        cache.cached_native_symbol("NVDA-USDT"),
        Some("NVDAUSDTM".to_owned())
    );
    assert_eq!(cache.cached_unit("NVDA"), Some(0.01));
}

#[test]
fn incomplete_sizing_metadata_never_enters_execution_cache() {
    let cache = KucoinContractMultipliers::default();
    let mut invalid = contract("BADUSDTM", "BAD", "USDT", 0.5);
    invalid.tick_size = serde_json::Value::Null;
    cache.apply_contracts(vec![invalid]);

    assert_eq!(
        cache.cached_native_symbol("BAD"),
        Some("BADUSDTM".to_owned())
    );
    assert_eq!(cache.cached_unit("BAD"), None);
    assert!(cache.cached_spec("BAD").is_err());
}

#[test]
fn contracts_active_fixture_preserves_official_multiplier_and_funding() {
    let fixture = include_str!("../../fixtures/kucoin/contracts_active_xbt_eth_usdtm.json");
    let wrap: crate::adapters::kucoin_response::KucoinResponse<Vec<ContractActive>> =
        serde_json::from_str(fixture).expect("kucoin contracts fixture");
    let rows = wrap.into_data("contracts/active").expect("contract rows");
    let xbt = rows
        .iter()
        .find(|row| row.symbol == "XBTUSDTM")
        .expect("xbt contract");
    let spec = contract_spec(xbt).expect("xbt spec");
    let funding = parse_funding(xbt).expect("xbt funding parses");

    assert_eq!(spec.native_symbol, "XBTUSDTM");
    let identity = contract_identity(xbt).expect("xbt identity");
    assert_eq!(identity.normalized_symbol, "BTC");
    assert_eq!(identity.quote_currency, "USDT");
    assert_eq!(identity.settle_currency, "USDT");
    assert_eq!(spec.order_unit, 0.001);
    assert_eq!(spec.price_tick, 0.1);
    assert_eq!(spec.lot_size, 1.0);
    assert_eq!(funding.symbol, "BTC");
    assert_eq!(funding.funding_interval, 8);
}
