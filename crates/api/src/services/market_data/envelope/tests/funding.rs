use super::super::*;

#[test]
fn spot_tick_envelope_uses_runtime_coverage() {
    let envelope = spot_ticks_envelope(
        Vec::<shared_types::SpotTick>::new(),
        0,
        &[
            MarketRuntimeHealth {
                venue: MARKET_AGGREGATE_VENUE.to_owned(),
                operation: MARKET_OP_REST_SPOT_TICKS,
                quality: MarketQuality::Missing,
                source: MarketSource::RestBaseline,
                requested: 8,
                rows: 0,
                retry_after_ms: None,
                last_error: Some("empty".into()),
                problem: None,
                observed_at_ms: 10,
            },
            MarketRuntimeHealth {
                venue: "gate".to_owned(),
                operation: MARKET_OP_REST_SPOT_TICKS,
                quality: MarketQuality::RateLimited,
                source: MarketSource::RestBaseline,
                requested: 1,
                rows: 0,
                retry_after_ms: Some(1_500),
                last_error: Some("rate limited".into()),
                problem: Some(
                    shared_types::ExchangeProblem::new(
                        "gate",
                        MARKET_OP_REST_SPOT_TICKS,
                        "rate limited",
                    )
                    .with_retry_after_ms(Some(1_500))
                    .with_source("exchange-fanout"),
                ),
                observed_at_ms: 10,
            },
        ],
        Vec::new(),
        None,
        None,
    );

    assert_eq!(
        envelope
            .health
            .coverage
            .as_ref()
            .map(|coverage| coverage.requested),
        Some(8)
    );
    assert_eq!(
        envelope
            .health
            .coverage
            .as_ref()
            .map(|coverage| coverage.coverage_pct),
        Some(0.0)
    );
    assert_eq!(
        envelope
            .health
            .problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some("MARKET_DATA_MISSING")
    );
    assert_eq!(envelope.retry_after_ms, Some(1_500));
    assert_eq!(envelope.fanout.len(), 1);
    assert_eq!(envelope.fanout[0].venue, "gate");
    assert_eq!(envelope.fanout[0].health.retry_after_ms, Some(1_500));
    assert_eq!(
        envelope.fanout[0]
            .health
            .problem
            .as_ref()
            .and_then(|problem| problem.details.as_ref())
            .and_then(|details| details.get("operation"))
            .and_then(|value| value.as_str()),
        Some(MARKET_OP_REST_SPOT_TICKS)
    );
}

#[test]
fn funding_rates_envelope_marks_empty_snapshot_missing() {
    let envelope = funding_rates_envelope(
        Vec::new(),
        MarketSource::LocalCache,
        1_000,
        &[
            MarketRuntimeHealth {
                venue: MARKET_AGGREGATE_VENUE.to_owned(),
                operation: MARKET_OP_REST_FUNDING_RATES,
                quality: MarketQuality::Missing,
                source: MarketSource::RestBaseline,
                requested: 2,
                rows: 0,
                retry_after_ms: None,
                last_error: Some("empty".into()),
                problem: None,
                observed_at_ms: 999,
            },
            MarketRuntimeHealth {
                venue: "okx".to_owned(),
                operation: MARKET_OP_REST_FUNDING_RATES,
                quality: MarketQuality::RateLimited,
                source: MarketSource::RestBaseline,
                requested: 1,
                rows: 0,
                retry_after_ms: Some(2_000),
                last_error: Some("rate limited".into()),
                problem: None,
                observed_at_ms: 999,
            },
        ],
        Vec::new(),
    );

    assert!(envelope.data.is_empty());
    assert_eq!(envelope.health.quality, SharedQuality::Missing);
    assert_eq!(
        envelope
            .health
            .coverage
            .as_ref()
            .map(|coverage| (coverage.requested, coverage.received)),
        Some((2, 0))
    );
    assert_eq!(envelope.fanout.len(), 1);
    assert_eq!(envelope.fanout[0].venue, "okx");
    assert_eq!(envelope.retry_after_ms, Some(2_000));
    assert_eq!(
        envelope.fanout[0].operation,
        shared_types::MarketDataSnapshotOperation::FundingRates
    );
    assert_eq!(
        envelope.fanout[0].health.quality,
        SharedQuality::RateLimited
    );
    assert_eq!(
        envelope
            .health
            .problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some("MARKET_DATA_MISSING")
    );
}

#[test]
fn funding_rates_envelope_uses_cache_health_when_rows_are_present() {
    let envelope = funding_rates_envelope(
        vec![funding("BTC", "binance", 900)],
        MarketSource::LocalCache,
        1_000,
        &[
            MarketRuntimeHealth {
                venue: MARKET_AGGREGATE_VENUE.to_owned(),
                operation: MARKET_OP_REST_FUNDING_RATES,
                quality: MarketQuality::RateLimited,
                source: MarketSource::RestBaseline,
                requested: 3,
                rows: 1,
                retry_after_ms: Some(2_000),
                last_error: Some("rate limited".into()),
                problem: None,
                observed_at_ms: 999,
            },
            MarketRuntimeHealth {
                venue: "okx".to_owned(),
                operation: MARKET_OP_REST_FUNDING_RATES,
                quality: MarketQuality::RateLimited,
                source: MarketSource::RestBaseline,
                requested: 1,
                rows: 0,
                retry_after_ms: Some(2_000),
                last_error: Some("rate limited".into()),
                problem: None,
                observed_at_ms: 999,
            },
        ],
        Vec::new(),
    );

    assert_eq!(envelope.health.quality, SharedQuality::Fresh);
    assert_eq!(envelope.health.source, MarketDataSourceKind::LocalCache);
    assert_eq!(envelope.retry_after_ms, Some(2_000));
    assert_eq!(envelope.health.retry_after_ms, None);
    assert!(envelope.health.problem.is_none());
    assert_eq!(
        envelope
            .health
            .coverage
            .as_ref()
            .map(|coverage| (coverage.requested, coverage.received)),
        Some((1, 1))
    );
    assert_eq!(envelope.fanout.len(), 1);
}

#[test]
fn funding_rates_envelope_carries_row_evidence() {
    let row_evidence = vec![row_evidence("binance", "BTC")];

    let envelope = funding_rates_envelope(
        vec![funding("BTC", "binance", 900)],
        MarketSource::LocalCache,
        1_000,
        &[],
        row_evidence.clone(),
    );

    assert_eq!(envelope.row_evidence, row_evidence);
}

#[test]
fn spot_ticks_envelope_carries_row_evidence() {
    let row_evidence = vec![MarketDataRowEvidence {
        operation: shared_types::MarketDataSnapshotOperation::SpotTicks,
        ..row_evidence("binance", "BTCUSDT")
    }];

    let envelope = spot_ticks_envelope(vec!["BTCUSDT"], 1, &[], row_evidence.clone(), None, None);

    assert_eq!(envelope.row_evidence, row_evidence);
}

fn funding(symbol: &str, exchange: &str, timestamp: i64) -> FundingRateData {
    FundingRateData {
        symbol: symbol.to_owned(),
        exchange: exchange.to_owned(),
        rate: 0.0001,
        rate_8h: 0.0001,
        predicted_rate: None,
        next_funding_time: 1_000,
        funding_interval: 8,
        volume_24h: 1_000.0,
        timestamp,
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    }
}

fn row_evidence(venue: &str, symbol: &str) -> MarketDataRowEvidence {
    MarketDataRowEvidence {
        venue: venue.to_owned(),
        symbol: symbol.to_owned(),
        operation: shared_types::MarketDataSnapshotOperation::FundingRates,
        health: MarketDataHealth {
            quality: SharedQuality::Fresh,
            source: MarketDataSourceKind::RestBaseline,
            freshness_ms: Some(100),
            retry_after_ms: None,
            last_error: None,
            observed_at_ms: 1_000,
            coverage: Some(coverage(1, 1)),
            problem: None,
        },
    }
}
