use super::*;

#[test]
fn coverage_prefers_executable_contract_for_duplicate_canonical_symbol() {
    let usdt = instrument("bitget", "ADAUSDT");
    let mut coin = instrument("bitget", "ADAUSD_CM");
    coin.canonical_symbol = "ADA".into();
    coin.quote_asset = Some("USD".into());
    coin.settle_asset = Some("ADA".into());
    coin.margin_asset = Some("ADA".into());
    coin.execution_supported = false;

    for candidates in [
        vec![coin.clone(), usdt.clone()],
        vec![usdt.clone(), coin.clone()],
    ] {
        let selected = preferred_coverage_instrument(candidates, NOW);
        assert_eq!(
            selected.map(|instrument| instrument.native_symbol),
            Some("ADAUSDT".into())
        );
    }

    let registry = InstrumentRegistry::default();
    assert_eq!(registry.replace_venue("bitget", vec![coin, usdt]), 2);
    let coverage = registry.coverage("ADA", NOW);
    let bitget = coverage.venues.iter().find(|entry| entry.venue == "bitget");
    assert!(bitget.is_some(), "bitget coverage row must exist");
    let Some(bitget) = bitget else {
        return;
    };
    assert_eq!(bitget.native_symbol, "ADAUSDT");
    assert_eq!(bitget.state, VenueListingState::Listed);
    assert!(bitget.execution_ready);
}
