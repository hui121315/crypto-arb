#![allow(clippy::panic)]

use super::super::*;

use super::fixtures::find_runtime_health;

const VENUE: &str = "kucoin";
const GRACE_MS: i64 = 7_000;

fn symbols(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn partial_ws_coverage_warms_before_deadline_then_fails_closed() {
    let cache = MarketDataCache::default();
    let requested = symbols(&["BTCUSDTM", "ETHUSDTM"]);
    let missing = symbols(&["ETHUSDTM"]);
    let started_at_ms = common::time::now_ms();

    cache.record_ws_runtime_at(sample(&requested, 1, &missing), started_at_ms);

    let rows = cache.runtime_health_snapshot();
    let warming = find_runtime_health(&rows, VENUE, MARKET_OP_WS_TICKER_SNAPSHOT);
    assert_eq!(warming.quality, MarketQuality::Warming);
    assert_eq!(warming.retry_after_ms, Some(GRACE_MS as u64));
    assert!(warming.problem.is_none());

    let status = cache.snapshot_status(started_at_ms);
    let shared = status
        .rows
        .iter()
        .find(|row| {
            row.venue == VENUE
                && row.operation == shared_types::MarketDataSnapshotOperation::WsTicker
        })
        .unwrap_or_else(|| panic!("missing kucoin ticker status"));
    assert_eq!(
        shared.health.quality,
        shared_types::MarketDataQuality::Unverified
    );
    assert_eq!(
        shared
            .health
            .problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some("MARKET_DATA_WARMING")
    );

    cache.record_ws_runtime_at(sample(&requested, 1, &missing), started_at_ms + GRACE_MS);

    let rows = cache.runtime_health_snapshot();
    let expired = find_runtime_health(&rows, VENUE, MARKET_OP_WS_TICKER_SNAPSHOT);
    assert_eq!(expired.quality, MarketQuality::Missing);
    assert!(expired.retry_after_ms.is_none());
    assert!(expired.problem.is_some());
}

#[test]
fn changing_plan_preserves_each_missing_symbols_own_age() {
    let cache = MarketDataCache::default();
    let requested = symbols(&["BTCUSDTM", "ETHUSDTM", "SOLUSDTM"]);
    let started_at_ms = common::time::now_ms();

    let eth_missing = symbols(&["ETHUSDTM"]);
    cache.record_ws_runtime_at(sample(&requested, 1, &eth_missing), started_at_ms);
    let both_missing = symbols(&["ETHUSDTM", "SOLUSDTM"]);
    cache.record_ws_runtime_at(sample(&requested, 1, &both_missing), started_at_ms + 6_000);
    cache.record_ws_runtime_at(
        sample(&requested, 1, &both_missing),
        started_at_ms + GRACE_MS,
    );

    let rows = cache.runtime_health_snapshot();
    assert_eq!(
        find_runtime_health(&rows, VENUE, MARKET_OP_WS_TICKER_SNAPSHOT).quality,
        MarketQuality::Missing
    );

    let sol_missing = symbols(&["SOLUSDTM"]);
    cache.record_ws_runtime_at(
        sample(&requested, 2, &sol_missing),
        started_at_ms + GRACE_MS,
    );

    let rows = cache.runtime_health_snapshot();
    let warming = find_runtime_health(&rows, VENUE, MARKET_OP_WS_TICKER_SNAPSHOT);
    assert_eq!(warming.quality, MarketQuality::Warming);
    assert_eq!(warming.retry_after_ms, Some(6_000));
}

#[test]
fn full_ws_coverage_clears_warmup_state() {
    let cache = MarketDataCache::default();
    let requested = symbols(&["BTCUSDTM", "ETHUSDTM"]);
    let started_at_ms = common::time::now_ms();

    let missing = symbols(&["ETHUSDTM"]);
    cache.record_ws_runtime_at(sample(&requested, 1, &missing), started_at_ms);
    cache.record_ws_runtime_at(sample(&requested, requested.len(), &[]), started_at_ms + 1);

    let rows = cache.runtime_health_snapshot();
    assert_eq!(
        find_runtime_health(&rows, VENUE, MARKET_OP_WS_TICKER_SNAPSHOT).quality,
        MarketQuality::Fresh
    );
    assert!(cache.ws_warmups.is_empty());
}

#[test]
fn kucoin_spot_sparse_event_stream_is_healthy_after_any_real_rows() {
    let cache = MarketDataCache::default();
    let requested = symbols(&["BTC-USDT", "QUIET-USDT"]);
    let sample = WsRuntimeSample {
        venue: VENUE,
        operation: MARKET_OP_WS_SPOT_SNAPSHOT,
        source: MarketSource::WsPush,
        requested_symbols: &requested,
        rows: 1,
        missing_symbols: &[],
        grace_ms: GRACE_MS,
    };

    cache.record_ws_runtime_at(sample, common::time::now_ms());

    let rows = cache.runtime_health_snapshot();
    assert_eq!(
        find_runtime_health(&rows, VENUE, MARKET_OP_WS_SPOT_SNAPSHOT).quality,
        MarketQuality::Fresh
    );
}

fn sample<'a>(
    requested_symbols: &'a [String],
    rows: usize,
    missing_symbols: &'a [String],
) -> WsRuntimeSample<'a> {
    WsRuntimeSample {
        venue: VENUE,
        operation: MARKET_OP_WS_TICKER_SNAPSHOT,
        source: MarketSource::WsPush,
        requested_symbols,
        rows,
        missing_symbols,
        grace_ms: GRACE_MS,
    }
}
