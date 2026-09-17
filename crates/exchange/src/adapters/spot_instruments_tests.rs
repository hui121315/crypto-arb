use super::*;

const CHECKED_AT: i64 = 1_785_772_800_000;

#[test]
fn binance_filter_contract_builds_executable_spot_spec() {
    let response: BinanceResponse = serde_json::from_str(
        r#"{"symbols":[{"symbol":"BTCUSDT","status":"TRADING","baseAsset":"BTC","quoteAsset":"USDT","filters":[{"filterType":"PRICE_FILTER","tickSize":"0.01"},{"filterType":"LOT_SIZE","stepSize":"0.00001","minQty":"0.00001"},{"filterType":"NOTIONAL","minNotional":"5"}]}]}"#,
    )
    .expect("fixture");
    let row = response.symbols.into_iter().next().expect("row");
    let row = binance_instrument(&row, CHECKED_AT).expect("instrument");
    assert_eq!(row.native_symbol, "BTCUSDT");
    assert_eq!(row.product_type.as_deref(), Some("spot"));
    assert!(row.is_hedge_constructible());
    assert!(evidence_matches(&row));
}

#[test]
fn every_cex_parser_preserves_native_pair_and_official_evidence() {
    let okx = okx_instrument(
        &OkxSpotRow {
            inst_id: "SOL-USDT".into(),
            base: "SOL".into(),
            quote: "USDT".into(),
            tick: "0.001".into(),
            lot: "0.001".into(),
            min_size: "0.01".into(),
            state: "live".into(),
        },
        CHECKED_AT,
    )
    .expect("okx");
    let gate = gate_instrument(
        &GateSpotRow {
            id: "SOL_USDT".into(),
            base: "SOL".into(),
            quote: "USDT".into(),
            precision: 3,
            amount_precision: 3,
            min_base_amount: "0.01".into(),
            min_quote_amount: "1".into(),
            trade_status: "tradable".into(),
        },
        CHECKED_AT,
    )
    .expect("gate");
    for row in [okx, gate] {
        assert!(row.is_hedge_constructible());
        assert!(evidence_matches(&row));
    }
}

#[test]
fn bybit_and_kucoin_specs_preserve_executable_precision() {
    let bybit = bybit_instrument(
        &BybitSpotRow {
            symbol: "SOLUSDT".into(),
            base: "SOL".into(),
            quote: "USDT".into(),
            status: "Trading".into(),
            price: BybitPriceFilter {
                tick_size: "0.001".into(),
            },
            lot: BybitLotFilter {
                base_precision: "0.001".into(),
                min_qty: "0.01".into(),
                min_notional: "1".into(),
            },
        },
        CHECKED_AT,
    )
    .expect("bybit");
    let kucoin = kucoin_instrument(
        &KucoinSpotRow {
            symbol: "SOL-USDT".into(),
            base: "SOL".into(),
            quote: "USDT".into(),
            price_increment: "0.001".into(),
            base_increment: "0.001".into(),
            base_min_size: "0.01".into(),
            quote_min_size: "1".into(),
            enable_trading: true,
        },
        CHECKED_AT,
    )
    .expect("kucoin");

    for row in [bybit, kucoin] {
        assert_eq!(row.price_tick, Some(0.001));
        assert_eq!(row.qty_step, Some(0.001));
        assert!(row.is_hedge_constructible());
        assert!(evidence_matches(&row));
    }
}
