use super::*;

#[test]
fn prepared_leg_reprices_targets_from_one_ws_snapshot() {
    let market_timestamp_ms = 9_000;
    let prepared = PreparedLegQuote {
        spec: LegSpec {
            exchange: "mock".to_owned(),
            symbol: "BTC/USDT".to_owned(),
            book_kind: LegBookKind::Spot,
            side: OrderSide::Buy,
            fallback_price: None,
            funding_rate: 0.0,
            next_funding_time: 0,
            funding_interval_hours: 0,
        },
        role: HedgeLegRole::Long,
        now_ms: 10_000,
        depth_bps: 20.0,
        blockers: Vec::new(),
        orderbook: orderbook_quote_from_read(
            "mock",
            "BTC/USDT",
            MarketRead {
                value: Some(OrderBookInfo {
                    symbol: "BTC/USDT".to_owned(),
                    exchange: "mock".to_owned(),
                    bids: vec![[99.9, 1.0], [99.8, 2.0]],
                    asks: vec![[100.0, 1.0], [100.1, 2.0]],
                    timestamp: market_timestamp_ms,
                }),
                quality: MarketQuality::Fresh,
                freshness_ms: Some(1_000),
                source: MarketSource::WsPush,
                retry_after_ms: None,
                last_error: None,
            },
        ),
    };

    let initial = prepared.build_for_notional(100.0);
    let paired = prepared.build_for_base_quantity(2.0);

    assert_eq!(initial.open_vwap_price, Some(100.0));
    assert_eq!(paired.open_vwap_price, Some(100.05));
    assert_eq!(initial.market_timestamp_ms, Some(market_timestamp_ms));
    assert_eq!(paired.market_timestamp_ms, Some(market_timestamp_ms));
    assert_eq!(initial.depth_usd_20bps, paired.depth_usd_20bps);
    assert_eq!(initial.market_evidence, paired.market_evidence);
}
