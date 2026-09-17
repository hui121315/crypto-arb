use super::*;

#[test]
fn valid_bitget_funding_interval_rejects_unknown_values() {
    assert_eq!(valid_bitget_funding_interval(Some(1)), Some(1));
    assert_eq!(valid_bitget_funding_interval(Some(2)), Some(2));
    assert_eq!(valid_bitget_funding_interval(Some(4)), Some(4));
    assert_eq!(valid_bitget_funding_interval(Some(8)), Some(8));
    assert_eq!(valid_bitget_funding_interval(None), None);
    assert_eq!(valid_bitget_funding_interval(Some(0)), None);
    assert_eq!(valid_bitget_funding_interval(Some(3)), None);
    assert_eq!(valid_bitget_funding_interval(Some(6)), None);
}

#[test]
fn bitget_depth_limit_preserves_supported_requested_depth() {
    assert_eq!(bitget_depth_limit(0), 1);
    assert_eq!(bitget_depth_limit(1), 1);
    assert_eq!(bitget_depth_limit(7), 7);
    assert_eq!(bitget_depth_limit(20), 20);
    assert_eq!(bitget_depth_limit(1_000), 1_000);
    assert_eq!(bitget_depth_limit(2_000), 1_000);
}

#[test]
fn spot_symbol_matches_recognises_pair_symbol() {
    let symbols = ["BTC/USDT".to_owned()];
    assert!(spot_symbol_matches("BTCUSDT", Some(&symbols)));
    assert!(!spot_symbol_matches("ETHUSDT", Some(&symbols)));
    assert!(spot_symbol_matches("BTCUSDT", None));
    // Inputs missing a known quote suffix have no parseable pair.
    assert!(!spot_symbol_matches("WHATEVER", None));
}
