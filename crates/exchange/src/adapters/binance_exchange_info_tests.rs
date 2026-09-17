use super::*;

#[test]
fn exchange_info_parses_official_fixture_constraints() {
    let fixture = include_str!("../../fixtures/binance/usdm_exchange_info_btcusdt.json");
    let info: ExchangeInfoResponse =
        serde_json::from_str(fixture).expect("exchangeInfo fixture is valid");

    let specs = instrument_specs_from_response(info);
    let btc = specs.get("BTCUSDT").expect("BTCUSDT exists");

    assert_eq!(btc.native_symbol, "BTCUSDT");
    assert_eq!(btc.contract_size, 1.0);
    assert_exchange_capabilities(&btc.constraints);
    assert_asset_identity(&btc.constraints);
    assert_filter_constraints(&btc.constraints);
}

fn assert_exchange_capabilities(btc: &BinanceOrderConstraints) {
    assert!(btc.capabilities.is_trading);
    assert!(btc.capabilities.is_registry_perpetual);
    assert!(btc.capabilities.is_perpetual);
    assert!(btc.capabilities.supports_limit);
    assert!(btc.capabilities.supports_market);
    assert!(btc.capabilities.supports_gtc);
    assert!(btc.capabilities.supports_ioc);
    assert!(btc.capabilities.supports_fok);
    assert!(btc.capabilities.supports_gtx);
}

fn assert_asset_identity(btc: &BinanceOrderConstraints) {
    assert_eq!(btc.identity.base_asset, "BTC");
    assert_eq!(btc.identity.quote_asset, "USDT");
    assert_eq!(btc.identity.margin_asset, "USDT");
}

fn assert_filter_constraints(btc: &BinanceOrderConstraints) {
    assert_eq!(btc.price.min_price, Some(0.10));
    assert_eq!(btc.price.max_price, Some(1_000_000.0));
    assert_eq!(btc.price.tick_size, Some(0.10));
    assert_qty_constraints(&btc.limit_qty, Some(0.001), Some(100.0), Some(0.001));
    assert_qty_constraints(&btc.market_qty, Some(0.01), Some(50.0), Some(0.01));
    assert_eq!(btc.min_notional, Some(100.0));
}

fn assert_qty_constraints(
    qty: &BinanceQuantityConstraints,
    min_qty: Option<f64>,
    max_qty: Option<f64>,
    step_size: Option<f64>,
) {
    assert!(qty.present);
    assert_eq!(qty.min_qty, min_qty);
    assert_eq!(qty.max_qty, max_qty);
    assert_eq!(qty.step_size, step_size);
}

#[test]
fn positive_parser_ignores_zero_empty_and_invalid_values() {
    assert_eq!(positive_f64("0"), None);
    assert_eq!(positive_f64(""), None);
    assert_eq!(positive_f64("not-a-number"), None);
    assert_eq!(positive_f64("0.001"), Some(0.001));
}

#[test]
fn cache_replaces_and_serves_fresh_constraints() {
    let cache = ExchangeInfoCache::default();
    let mut specs = HashMap::new();
    specs.insert(
        "BTCUSDT".to_owned(),
        BinanceInstrumentSpec {
            native_symbol: "BTCUSDT".to_owned(),
            contract_size: 1.0,
            constraints: BinanceOrderConstraints {
                limit_qty: BinanceQuantityConstraints {
                    present: true,
                    min_qty: Some(0.001),
                    ..Default::default()
                },
                ..Default::default()
            },
        },
    );

    cache.replace(specs, 1_000);

    assert!(cache.is_fresh(1_001));
    assert_eq!(
        cache
            .resolve("BTC")
            .ok()
            .and_then(|row| row.constraints.limit_qty.min_qty),
        Some(0.001)
    );
    assert!(cache.resolve("ETH").is_err());
}

#[test]
fn resolver_preserves_explicit_usdc_and_never_crosses_quotes() {
    let fixture = include_str!("../../fixtures/binance/usdm_exchange_info_usdt_usdc.json");
    let info: ExchangeInfoResponse = serde_json::from_str(fixture).expect("dual quote fixture");
    let specs = instrument_specs_from_response(info);

    assert_eq!(
        resolve_instrument_spec(&specs, "BTC-USDC-SWAP")
            .expect("explicit USDC")
            .native_symbol,
        "BTCUSDC"
    );
    assert_eq!(
        resolve_instrument_spec(&specs, "BTC")
            .expect("base defaults to verified USDT")
            .native_symbol,
        "BTCUSDT"
    );
    assert!(matches!(
        resolve_instrument_spec(&specs, "ETHUSDC"),
        Err(ExchangeError::UnsupportedSymbol(symbol)) if symbol == "ETHUSDC"
    ));
}

#[test]
fn registry_projection_uses_compiled_usdt_and_usdc_specs() {
    let fixture = include_str!("../../fixtures/binance/usdm_exchange_info_usdt_usdc.json");
    let info: ExchangeInfoResponse = serde_json::from_str(fixture).expect("dual quote fixture");
    let rows = instruments_from_response(&info, 123);

    assert_eq!(rows.len(), 2);
    let usdc = rows
        .iter()
        .find(|row| row.native_symbol == "BTCUSDC")
        .expect("USDC instrument");
    assert_eq!(usdc.quote_asset.as_deref(), Some("USDC"));
    assert_eq!(usdc.margin_asset.as_deref(), Some("USDC"));
    assert_eq!(usdc.contract_size, Some(1.0));
    assert_eq!(usdc.price_tick, Some(0.1));
    assert_eq!(usdc.qty_step, Some(0.001));
    assert_eq!(usdc.min_notional, Some(5.0));
    assert_eq!(
        usdc.schema_version.as_deref(),
        Some(INSTRUMENT_SCHEMA_VERSION)
    );
    assert!(usdc.has_official_provenance());
}

#[test]
fn tradifi_perpetual_is_listed_evidence_but_remains_execution_blocked() {
    // Live official GET /fapi/v1/exchangeInfo shape observed for AMATUSDT.
    let payload = r#"{
        "symbols": [{
            "symbol": "AMATUSDT",
            "baseAsset": "AMAT",
            "quoteAsset": "USDT",
            "marginAsset": "USDT",
            "status": "TRADING",
            "contractType": "TRADIFI_PERPETUAL",
            "underlyingType": "EQUITY",
            "orderTypes": ["LIMIT", "MARKET"],
            "timeInForce": ["GTC", "IOC", "FOK", "GTX"],
            "filters": [
                {"filterType":"PRICE_FILTER","tickSize":"0.01"},
                {"filterType":"LOT_SIZE","minQty":"0.01","stepSize":"0.01"},
                {"filterType":"MIN_NOTIONAL","notional":"5"}
            ]
        }]
    }"#;
    let info: ExchangeInfoResponse = serde_json::from_str(payload).expect("TradFi fixture");

    let rows = instruments_from_response(&info, 123);

    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.native_symbol, "AMATUSDT");
    assert_eq!(row.canonical_symbol, "AMAT");
    assert_eq!(row.asset_class, InstrumentAssetClass::Equity);
    assert_eq!(row.listing_status, InstrumentListingStatus::Trading);
    assert!(!row.execution_supported);
    assert!(row.has_official_provenance());
    assert!(!row.is_hedge_constructible());
}
