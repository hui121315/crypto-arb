use super::*;
use rust_decimal::Decimal;
use shared_types::{
    MarketDataHealth, MarketDataSnapshotOperation, MarketDataSourceKind, VenueListingState,
};

#[test]
fn filters_symbol_base_quote_venue_and_fresh_only_without_mixing_stale_rows() {
    let ticks = vec![
        tick("binance", "BTCUSDT", 30),
        tick("okx", "BTC/USDT", 20),
        tick("binance", "BTCUSDC", 10),
    ];
    let evidence = vec![
        evidence("binance", "BTCUSDT", MarketDataQuality::Fresh),
        evidence("okx", "BTC/USDT", MarketDataQuality::StaleAllowed),
        evidence("binance", "BTCUSDC", MarketDataQuality::Fresh),
    ];
    let query = SpotTicksQuery {
        base: Some("BTC".into()),
        quote: Some("USDT".into()),
        venue: Some("BINANCE".into()),
        limit: Some(10),
        fresh_only: true,
        ..Default::default()
    };

    let filtered = apply_query(&ticks, evidence, &query);

    assert_eq!(filtered.data.ticks.len(), 1);
    assert_eq!(filtered.data.ticks[0].symbol, "BTCUSDT");
    assert_eq!(filtered.row_evidence.len(), 1);
    assert_eq!(filtered.data.page.total_rows, 1);
    assert_eq!(
        (filtered.coverage.requested, filtered.coverage.received),
        (1, 1)
    );
}

#[test]
fn legacy_base_symbol_query_is_bounded_and_reports_cursor_limit_problems() {
    let ticks = vec![
        tick("binance", "BTCUSDT", 30),
        tick("binance", "BTCUSDC", 20),
        tick("okx", "BTC/USDT", 10),
    ];
    let evidence = ticks
        .iter()
        .map(|row| evidence(&row.venue, &row.symbol, MarketDataQuality::Fresh))
        .collect();
    let query = SpotTicksQuery {
        symbol: Some("BTC".into()),
        limit: Some(0),
        cursor: Some("bad".into()),
        ..Default::default()
    };

    let filtered = apply_query(&ticks, evidence, &query);

    assert_eq!(filtered.data.page.limit, 1);
    assert_eq!(filtered.data.page.max_limit, SPOT_TICKS_MAX_LIMIT);
    assert_eq!(filtered.data.page.start_offset, 0);
    assert_eq!(filtered.data.page.total_rows, 3);
    assert!(filtered.data.page.has_more);
    assert!(filtered
        .data
        .query_problems
        .iter()
        .any(|problem| problem.code == codes::LIST_LIMIT_CLAMPED));
    assert!(filtered
        .data
        .query_problems
        .iter()
        .any(|problem| problem.code == codes::LIST_CURSOR_INVALID));
}

#[tokio::test]
async fn page_and_query_problem_keep_the_scoped_request_id() {
    let query = SpotTicksQuery {
        limit: Some(0),
        ..Default::default()
    };

    let filtered = common::request_id::scope("req-spot-page".into(), async {
        apply_query(&[], Vec::new(), &query)
    })
    .await;

    assert_eq!(filtered.data.request_id.as_deref(), Some("req-spot-page"));
    assert_eq!(
        filtered.data.query_problems[0].request_id.as_deref(),
        Some("req-spot-page")
    );
}

#[test]
fn base_listing_coverage_includes_explicit_or_observed_base_without_marking_it_executable() {
    let registry = InstrumentRegistry::default();
    let query = SpotTicksQuery::for_symbol("BTCUSDT");

    let coverage = base_listing_coverage(&registry, &[tick("binance", "BTCUSDT", 1)], &query, 1);

    assert_eq!(coverage.len(), 1);
    assert_eq!(coverage[0].canonical_symbol, "BTC");
    assert!(!coverage[0].venues.is_empty());
    assert!(coverage[0]
        .venues
        .iter()
        .all(|entry| entry.state == VenueListingState::Unknown));
    assert!(!coverage[0].is_arbitrage_constructible(1));
}

#[test]
fn kraken_slash_pair_keeps_its_usd_quote_identity() {
    assert_eq!(
        split_spot_pair("SOL/USD"),
        Some(("SOL".to_owned(), "USD".to_owned()))
    );
}

fn tick(venue: &str, symbol: &str, received_at_ms: i64) -> SpotTick {
    SpotTick {
        venue: venue.into(),
        symbol: symbol.into(),
        bid: Decimal::new(100, 0),
        ask: Decimal::new(101, 0),
        last: Decimal::new(100, 0),
        bid_size: Some(Decimal::new(1, 0)),
        ask_size: Some(Decimal::new(1, 0)),
        volume_24h: Decimal::new(1_000_000, 0),
        exchange_ts_ms: Some(received_at_ms),
        received_at_ms,
    }
}

fn evidence(venue: &str, symbol: &str, quality: MarketDataQuality) -> MarketDataRowEvidence {
    MarketDataRowEvidence {
        venue: venue.into(),
        symbol: symbol.into(),
        operation: MarketDataSnapshotOperation::SpotTicks,
        health: MarketDataHealth {
            quality,
            source: MarketDataSourceKind::RestBaseline,
            freshness_ms: Some(1),
            retry_after_ms: None,
            last_error: None,
            observed_at_ms: 1,
            coverage: None,
            problem: None,
        },
    }
}
