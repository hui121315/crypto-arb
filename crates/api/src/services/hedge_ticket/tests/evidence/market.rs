use super::*;

#[test]
fn leg_market_evidence_keeps_live_orderbook_source() {
    let read = MarketRead {
        value: Some(book(vec![[100.0, 2.0]], vec![[101.0, 2.0]])),
        quality: MarketQuality::Fresh,
        freshness_ms: Some(12),
        source: crate::services::market_data::MarketSource::WsPush,
        retry_after_ms: None,
        last_error: None,
    };
    let quote = orderbook_quote_from_read("hyperliquid:xyz", "MU", read);
    let spec = leg_spec("hyperliquid:xyz", "MU");
    let health = quote.health(100);
    let reference = quote
        .display_book()
        .and_then(|book| reference_price(book, OrderSide::Buy));

    let evidence = leg_market_evidence(&spec, reference, true, &health, 100);

    assert_eq!(evidence.price, Some(101.0));
    assert_eq!(evidence.health.quality, MarketDataQuality::Fresh);
    assert_eq!(evidence.health.source, MarketDataSourceKind::WsPush);
    assert!(evidence.health.problem.is_none());
}

#[test]
fn leg_market_evidence_marks_snapshot_fallback_unverified() {
    let read = MarketRead {
        value: None,
        quality: MarketQuality::RateLimited,
        freshness_ms: None,
        source: crate::services::market_data::MarketSource::RestBaseline,
        retry_after_ms: Some(2_000),
        last_error: Some("rate limited".into()),
    };
    let quote = orderbook_quote_from_read("hyperliquid:xyz", "MU", read);
    let spec = leg_spec("hyperliquid:xyz", "MU");
    let health = quote.health(100);

    let evidence = leg_market_evidence(&spec, spec.fallback_price, false, &health, 100);

    assert_eq!(evidence.price, Some(1.0));
    assert_eq!(evidence.health.quality, MarketDataQuality::Unverified);
    assert_eq!(evidence.health.source, MarketDataSourceKind::LocalCache);
    assert_eq!(evidence.health.retry_after_ms, Some(2_000));
    assert!(evidence
        .health
        .problem
        .as_ref()
        .is_some_and(|problem| problem.code == "LEG_PRICE_SNAPSHOT_FALLBACK"));
}

#[test]
fn orderbook_blockers_do_not_present_unknown_timings_as_zero() {
    let stale = MarketRead {
        value: None,
        quality: MarketQuality::StaleAllowed,
        freshness_ms: None,
        source: crate::services::market_data::MarketSource::LocalCache,
        retry_after_ms: None,
        last_error: None,
    };
    let rate_limited = MarketRead {
        quality: MarketQuality::RateLimited,
        ..stale.clone()
    };

    let stale_blocker = stale_orderbook_blocker("okx", "BTC-USDT-SWAP", &stale);
    let retry_blocker = retry_after_blocker("okx", "BTC-USDT-SWAP", &rate_limited);

    assert!(stale_blocker.contains("盘口年龄 未知"));
    assert!(retry_blocker.contains("重试时间未知"));
    assert!(!stale_blocker.contains("0ms"));
    assert!(!retry_blocker.contains("0ms"));
}
