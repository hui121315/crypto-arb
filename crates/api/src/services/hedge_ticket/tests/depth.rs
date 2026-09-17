use super::*;

#[test]
fn buy_depth_uses_asks_inside_configured_bps() {
    let book = book(vec![[99.0, 1.0]], vec![[100.0, 2.0], [100.1, 3.0]]);

    let depth = depth_usd_within_bps(&book, OrderSide::Buy, Some(100.0), 5.0).unwrap_or_default();

    assert_eq!(depth, 200.0);
}

#[test]
fn sell_depth_uses_bids_inside_configured_bps() {
    let book = book(vec![[100.0, 2.0], [99.0, 3.0]], vec![[101.0, 1.0]]);

    let depth = depth_usd_within_bps(&book, OrderSide::Sell, Some(100.0), 5.0).unwrap_or_default();

    assert_eq!(depth, 200.0);
}

#[test]
fn vwap_slippage_uses_requested_notional() {
    let book = book(
        vec![[100.0, 1.0], [99.5, 2.0]],
        vec![[101.0, 1.0], [102.0, 2.0]],
    );

    let buy_vwap = vwap_price_for_notional(&book, OrderSide::Buy, 202.0).unwrap_or_default();
    let buy_slippage =
        slippage_bps(Some(101.0), Some(buy_vwap), OrderSide::Buy).unwrap_or_default();
    let sell_vwap = vwap_price_for_notional(&book, OrderSide::Sell, 200.0).unwrap_or_default();
    let sell_slippage =
        slippage_bps(Some(100.0), Some(sell_vwap), OrderSide::Sell).unwrap_or_default();

    assert!(buy_vwap > 101.0);
    assert!(buy_slippage > 0.0);
    assert!(sell_vwap < 100.0);
    assert!(sell_slippage > 0.0);
}

#[test]
fn stale_market_rejects_zero_timestamp() {
    let mut book = book(vec![[100.0, 1.0]], vec![[101.0, 1.0]]);
    book.timestamp = 0;

    assert!(stale_market(Some(&book), 40_000));
}

#[test]
fn depth_guard_reports_unavailable_depth_without_zero_dollar_claim() {
    let leg = HedgeLegQuote {
        role: HedgeLegRole::Long,
        exchange: "hyperliquid:xyz".into(),
        symbol: "SNDK".into(),
        side: OrderSide::Buy,
        reference_price: None,
        bid: None,
        ask: None,
        mid: None,
        open_vwap_price: None,
        open_slippage_bps: None,
        close_vwap_price: None,
        close_slippage_bps: None,
        depth_usd_5bps: None,
        depth_usd_10bps: None,
        depth_usd_20bps: None,
        max_notional_usd: None,
        market_evidence: None,
        depth_health: None,
        depth_reason: None,
        funding_bps: None,
        next_funding_time: 0,
        funding_interval_hours: 0,
        market_timestamp_ms: None,
        blockers: Vec::new(),
    };

    let guard = depth_guard("depth", "深度", &leg, 375.0);

    assert!(!guard.passed);
    assert!(guard.detail.contains("深度暂不可用"));
    assert!(!guard.detail.contains("$0"));
}

#[test]
fn depth_guard_reuses_orderbook_root_cause_when_depth_missing() {
    let mut leg = empty_leg();
    leg.blockers = vec!["hyperliquid:xyz SNDK orderbook 触发限频退避，2000ms 后重试".into()];

    let guard = depth_guard("depth", "深度", &leg, 375.0);

    assert!(!guard.passed);
    assert_eq!(guard.detail, leg.blockers[0]);
}

#[test]
fn depth_guard_names_canonical_five_bps_band() {
    let mut leg = empty_leg();
    leg.depth_usd_5bps = Some(6.0);
    leg.max_notional_usd = Some(6.0);

    let guard = depth_guard("depth", "深度", &leg, 750.0);

    assert!(!guard.passed);
    assert!(guard.detail.contains("买入腿0.05% 滑点带内可成交深度 $6"));
}

#[test]
fn executable_depth_never_substitutes_wider_twenty_bps_liquidity() {
    let mut leg = empty_leg();
    leg.depth_usd_20bps = Some(10_000.0);
    leg.max_notional_usd = Some(10_000.0);

    assert_eq!(executable_depth(&leg), None);

    leg.depth_usd_5bps = Some(250.0);
    assert_eq!(executable_depth(&leg), Some(250.0));
}

#[test]
fn stale_orderbook_quote_is_display_only_not_executable_depth() {
    let read = MarketRead {
        value: Some(book(vec![[100.0, 2.0]], vec![[101.0, 2.0]])),
        quality: MarketQuality::StaleAllowed,
        freshness_ms: Some(45_000),
        source: crate::services::market_data::MarketSource::LocalCache,
        retry_after_ms: Some(2_000),
        last_error: Some("rate limited".into()),
    };

    let quote = orderbook_quote_from_read("hyperliquid:xyz", "SNDK", read);
    let reference = quote
        .display_book()
        .and_then(|book| reference_price(book, OrderSide::Buy));
    let depth = quote
        .executable_book()
        .and_then(|book| depth_usd_within_bps(book, OrderSide::Buy, reference, 20.0));

    assert_eq!(reference, Some(101.0));
    assert!(depth.is_none());
    assert!(quote.executable_book().is_none());
    assert!(quote
        .blockers
        .first()
        .is_some_and(|blocker| blocker.contains("短时缓存")));
}

#[test]
fn fresh_rest_orderbook_is_display_only_not_executable_depth() {
    let read = MarketRead {
        value: Some(book(vec![[100.0, 2.0]], vec![[101.0, 2.0]])),
        quality: MarketQuality::Fresh,
        freshness_ms: Some(5),
        source: crate::services::market_data::MarketSource::RestBaseline,
        retry_after_ms: None,
        last_error: None,
    };

    let quote = orderbook_quote_from_read("gate", "BTC", read);

    assert!(quote.display_book().is_some());
    assert!(quote.executable_book().is_none());
    assert!(quote
        .blockers
        .first()
        .is_some_and(|blocker| blocker.contains("等待按需 WS 深度首帧")));
}
