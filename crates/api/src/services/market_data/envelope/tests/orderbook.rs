use super::super::*;

#[test]
fn missing_orderbook_envelope_keeps_retry_after_problem() {
    let read = MarketRead {
        value: None,
        quality: MarketQuality::RateLimited,
        freshness_ms: None,
        source: MarketSource::LocalCache,
        retry_after_ms: Some(2_000),
        last_error: Some("rate limited".into()),
    };

    let envelope = orderbook_envelope(read, "hyperliquid:xyz", "SNDK", 5, 1_000);

    assert!(envelope.data.is_none());
    assert_eq!(envelope.row_cap.as_ref().map(|cap| cap.max_rows), Some(5));
    assert_eq!(
        envelope.row_cap.as_ref().map(|cap| cap.returned_count),
        Some(0)
    );
    assert_eq!(envelope.health.quality, SharedQuality::RateLimited);
    assert_eq!(envelope.retry_after_ms, Some(2_000));
    assert_eq!(envelope.health.retry_after_ms, Some(2_000));
    assert_eq!(
        envelope
            .health
            .problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some("MARKET_DATA_RATE_LIMITED")
    );
}

#[test]
fn index_composition_envelope_preserves_unsupported_problem() {
    let read = MarketRead {
        value: None,
        quality: MarketQuality::Unsupported,
        freshness_ms: None,
        source: MarketSource::LocalCache,
        retry_after_ms: None,
        last_error: Some("unsupported".into()),
    };

    let envelope = index_composition_envelope(read, "okx", "SPX", 1_000);

    assert!(envelope.data.is_none());
    assert_eq!(envelope.retry_after_ms, None);
    assert_eq!(envelope.health.quality, SharedQuality::Unsupported);
    assert_eq!(
        envelope
            .health
            .problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some("MARKET_DATA_UNSUPPORTED")
    );
}

#[test]
fn index_composition_list_envelope_marks_empty_cache_missing() {
    let envelope = index_composition_list_envelope(Vec::new(), 1_000);

    assert!(envelope.data.is_empty());
    assert_eq!(envelope.health.quality, SharedQuality::Missing);
    assert_eq!(envelope.health.observed_at_ms, 1_000);
    assert_eq!(
        envelope
            .health
            .problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some("MARKET_DATA_MISSING")
    );
}
