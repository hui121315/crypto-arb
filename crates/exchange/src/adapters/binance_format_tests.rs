use super::*;

#[test]
fn number_param_avoids_scientific_notation_for_small_values() {
    assert_eq!(number_param(0.000_000_01), "0.00000001");
    assert_eq!(number_param(0.000_000_001), "0.000000001");
    assert_eq!(number_param(0.000_000_000_1), "0.0000000001");
    assert!(!number_param(1e-10).contains('e'));
}

#[test]
fn number_param_handles_normal_and_zero() {
    assert_eq!(number_param(1.0), "1");
    assert_eq!(number_param(0.5), "0.5");
    assert_eq!(number_param(30_000.123), "30000.123");
    assert_eq!(number_param(0.0), "0");
}

#[test]
fn number_param_handles_negative_and_extreme() {
    assert_eq!(number_param(-1.5), "-1.5");
    let output = number_param(-1e-13);
    assert!(output == "0" || output.starts_with('-') || output.starts_with("0"));
}

#[test]
fn number_param_strips_trailing_zeros() {
    assert_eq!(number_param(1.230_000_000), "1.23");
    assert_eq!(number_param(100.0), "100");
}

#[test]
fn serialize_query_preserves_order() {
    let query = serialize_query(&[
        ("symbol", "BTCUSDT"),
        ("side", "BUY"),
        ("timestamp", "1700000000000"),
    ]);
    assert_eq!(query, "symbol=BTCUSDT&side=BUY&timestamp=1700000000000");
}

#[test]
fn serialize_query_percent_encodes_special_chars() {
    let query = serialize_query(&[("newClientOrderId", "id with space&value=x")]);
    assert!(query.contains("id+with+space"));
    assert!(query.contains("%26"));
    assert!(query.contains("%3D"));
}

#[test]
fn serialize_query_handles_empty() {
    assert_eq!(serialize_query(&[]), "");
}

#[test]
fn is_usdm_perp_accepts_usdt_and_usdc() {
    assert!(is_usdm_perp("BTCUSDT"));
    assert!(is_usdm_perp("ETHUSDC"));
    assert!(is_usdm_perp("SOLUSDC"));
    assert!(is_usdm_perp("DOGEUSDT"));
}

#[test]
fn stream_symbol_preserves_explicit_usdc_and_defaults_bare_base_to_usdt() {
    assert_eq!(usdm_stream_symbol("BTC"), "btcusdt");
    assert_eq!(usdm_stream_symbol("BTCUSDT"), "btcusdt");
    assert_eq!(usdm_stream_symbol("BTCUSDC"), "btcusdc");
    assert_eq!(usdm_stream_symbol("BTC-USDC-SWAP"), "btcusdc");
}

#[test]
fn is_usdm_perp_rejects_other_quotes() {
    assert!(!is_usdm_perp("BTCBUSD"));
    assert!(!is_usdm_perp("BTCUSD"));
    assert!(!is_usdm_perp("BTC"));
    assert!(!is_usdm_perp(""));
}

#[test]
fn build_spot_symbols_param_normalizes_separators() {
    let raw = vec![
        "btc/usdt".to_owned(),
        "eth-usdt".to_owned(),
        "sol_usdc".to_owned(),
    ];
    let result = build_spot_symbols_param(&raw).expect("should encode");
    assert!(result.contains("BTCUSDT"));
    assert!(result.contains("ETHUSDT"));
    assert!(result.contains("SOLUSDC"));
}

#[test]
fn build_spot_symbols_param_requires_exact_quote_identity() {
    let raw = vec!["BTC".to_owned(), "ETHUSDT".to_owned()];
    assert_eq!(build_spot_symbols_param(&raw), None);
}

#[test]
fn build_spot_symbols_param_deduplicates_exact_pairs() {
    let raw = vec![
        "btc/usdt".to_owned(),
        "BTC-USDT".to_owned(),
        "ETHBTC".to_owned(),
    ];
    let result = build_spot_symbols_param(&raw).expect("exact pairs should encode");
    let symbols: Vec<String> = serde_json::from_str(&result).expect("symbols json");
    assert_eq!(symbols, ["BTCUSDT", "ETHBTC"]);
}

#[test]
fn build_spot_symbols_param_empty_input_returns_none() {
    let raw: Vec<String> = Vec::new();
    assert!(build_spot_symbols_param(&raw).is_none());
}

#[test]
fn usdm_candidates_preserve_explicit_quote_and_fallback_base_order() {
    assert_eq!(usdm_symbol_candidates("BTC-USDC-SWAP"), ["BTCUSDC"]);
    assert_eq!(usdm_symbol_candidates("BTCUSDT"), ["BTCUSDT"]);
    // 裸 base 展开千倍族候选（顺序即优先级）；上市集在
    // `Binance::to_exchange_symbol` 里负责挑真实存在的那一个。
    assert_eq!(
        usdm_symbol_candidates("BTC"),
        ["BTCUSDT", "1000BTCUSDT", "1MBTCUSDT", "BTCUSDC"]
    );
    assert_eq!(
        usdm_symbol_candidates("SHIB"),
        ["SHIBUSDT", "1000SHIBUSDT", "1MSHIBUSDT", "SHIBUSDC"]
    );
}
