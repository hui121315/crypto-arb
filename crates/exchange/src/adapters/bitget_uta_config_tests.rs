use super::*;

#[test]
fn category_query_is_upper_case() {
    assert_eq!(BitgetUtaCategory::UsdtFutures.as_query(), "USDT-FUTURES");
    assert_eq!(BitgetUtaCategory::UsdcFutures.as_query(), "USDC-FUTURES");
    assert_eq!(BitgetUtaCategory::CoinFutures.as_query(), "COIN-FUTURES");
    assert_eq!(BitgetUtaCategory::Spot.as_query(), "SPOT");
}

#[test]
fn category_ws_inst_type_is_lower_case() {
    assert_eq!(
        BitgetUtaCategory::UsdtFutures.as_ws_inst_type(),
        "usdt-futures"
    );
    assert_eq!(
        BitgetUtaCategory::UsdcFutures.as_ws_inst_type(),
        "usdc-futures"
    );
    assert_eq!(
        BitgetUtaCategory::CoinFutures.as_ws_inst_type(),
        "coin-futures"
    );
    assert_eq!(BitgetUtaCategory::Spot.as_ws_inst_type(), "spot");
}

#[test]
fn ws_args_normalize_symbol_and_inst_type() {
    let args = BitgetUtaWsArgs::new(BitgetUtaCategory::UsdtFutures, "ticker", "btcusdt");
    assert_eq!(args.inst_type, "usdt-futures");
    assert_eq!(args.topic, "ticker");
    assert_eq!(args.symbol, "BTCUSDT");
}

#[test]
fn ws_args_serialize_matches_v3_shape() {
    // ccxt websocket-client-v3.ts canonical example:
    //   {op:"subscribe",args:[{instType:"spot",topic:"ticker",symbol:"BTCUSDT"}]}
    let args = BitgetUtaWsArgs::new(BitgetUtaCategory::Spot, "ticker", "BTCUSDT");
    let json = serde_json::to_value(&args).expect("serialize ws args");
    assert_eq!(
        json,
        serde_json::json!({
            "instType": "spot",
            "topic": "ticker",
            "symbol": "BTCUSDT",
        })
    );
}

#[test]
fn ws_endpoints_are_v3() {
    // Hard-pin to keep grep / future migrations explicit.
    assert!(PROD_WS_PUBLIC.contains("/v3/ws/public"));
    assert!(PROD_WS_PRIVATE.contains("/v3/ws/private"));
    assert!(DEMO_WS_PUBLIC.contains("wspap.bitget.com"));
    assert!(DEMO_WS_PRIVATE.contains("wspap.bitget.com"));
}
