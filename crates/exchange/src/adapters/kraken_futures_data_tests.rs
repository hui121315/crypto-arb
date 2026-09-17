use super::*;

#[test]
fn parses_official_ticker_into_price_funding_and_mark_views() {
    let fixture = include_str!("../../fixtures/kraken/futures_ticker_pf_xbtusd.json");
    let update = parse_ticker_frame(fixture)
        .expect("ticker parses")
        .expect("ticker update");

    assert_eq!(update.ticker.symbol, "BTC");
    assert_eq!(update.ticker.bid, 64_489.0);
    assert_eq!(update.ticker.ask, 64_490.0);
    assert_eq!(update.ticker.volume_24h, 280_317_810.719_8);
    assert_eq!(update.mark_index.index_price, Some(64_489.07));
    assert_eq!(update.mark_index.open_interest, Some(2_198.592_1));

    let funding = update.funding.expect("perpetual funding");
    assert_eq!(funding.funding_interval, 1);
    assert_eq!(funding.next_funding_time, 1_785_985_200_000);
    assert_eq!(funding.rate, 0.000_003_764_275);
    assert_eq!(funding.rate_8h, funding.rate);
    assert_eq!(funding.predicted_rate, Some(0.000_006_163_375));
}

#[test]
fn parses_book_snapshot_and_contiguous_delta_contract() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../fixtures/kraken/futures_book_pf_xbtusd.json"
    ))
    .expect("fixture json");

    let snapshot = parse_book_frame(&fixture["snapshot"].to_string()).expect("snapshot parses");
    let FuturesBookFrame::Snapshot(snapshot) = snapshot else {
        panic!("expected snapshot");
    };
    assert_eq!(snapshot.sequence, 291_492_228);
    assert_eq!(snapshot.bids[0].0, Decimal::from(64_516));
    assert_eq!(
        snapshot.asks[0].1,
        Decimal::from_str("0.031").expect("decimal")
    );

    let delta = parse_book_frame(&fixture["delta"].to_string()).expect("delta parses");
    let FuturesBookFrame::Delta(delta) = delta else {
        panic!("expected delta");
    };
    assert_eq!(delta.sequence, snapshot.sequence + 1);
    assert_eq!(delta.side, BookSide::Buy);
    assert_eq!(
        delta.quantity,
        Decimal::from_str("1.5924").expect("decimal")
    );
}

#[test]
fn instruments_authorize_linear_perp_and_keep_inverse_observation_only() {
    let fixture = include_str!("../../fixtures/kraken/futures_instruments_pf_xbtusd.json");
    let rows = parse_instruments(fixture).expect("instruments parse");

    let linear = rows
        .iter()
        .find(|row| row.native_symbol == "PF_XBTUSD")
        .unwrap();
    assert_eq!(linear.canonical_symbol, "BTC");
    assert_eq!(linear.qty_step, Some(0.0001));
    assert_eq!(linear.min_qty, Some(0.0001));
    assert_eq!(linear.funding_interval_ms, Some(3_600_000));
    assert!(linear.execution_supported);
    assert!(linear.is_hedge_constructible());

    let integer_lot = rows
        .iter()
        .find(|row| row.native_symbol == "PF_PEPEUSD")
        .unwrap();
    assert_eq!(integer_lot.qty_step, Some(1_000.0));
    assert_eq!(integer_lot.min_qty, Some(1_000.0));
    assert!(integer_lot.is_hedge_constructible());

    let inverse = rows
        .iter()
        .find(|row| row.native_symbol == "PI_XBTUSD")
        .unwrap();
    assert!(!inverse.execution_supported);
    assert!(inverse.is_observation_only());
}

#[test]
fn ignores_subscription_ack_frames() {
    assert!(parse_ticker_frame(
        r#"{"event":"subscribed","feed":"ticker","product_ids":["PF_XBTUSD"]}"#,
    )
    .expect("ack parses")
    .is_none());
    assert_eq!(
        parse_book_frame(r#"{"event":"subscribed","feed":"book"}"#).expect("ack parses"),
        FuturesBookFrame::Ignore
    );
}
