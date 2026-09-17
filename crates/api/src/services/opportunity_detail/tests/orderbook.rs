use super::*;

#[test]
fn optional_orderbook_problems_remain_section_local() {
    let envelope = envelope(
        test_opp(),
        DetailSegments {
            long_orderbook: degraded_market_envelope::<OrderBookInfo>(
                "LONG_ORDERBOOK_RATE_LIMITED",
                2_000,
            ),
            short_orderbook: degraded_market_envelope::<OrderBookInfo>(
                "SHORT_ORDERBOOK_RATE_LIMITED",
                5_000,
            ),
            history: empty_history_response(),
            long_index_composition: empty_market_envelope::<IndexCompositionSnapshot>(),
            short_index_composition: empty_market_envelope::<IndexCompositionSnapshot>(),
        },
        detail_request_meta(OpportunityDetailRequest::default()),
        Vec::new(),
        common::time::now_ms(),
    );

    assert_eq!(envelope.status, OpportunityEnvelopeStatus::Fresh);
    assert!(envelope.partial_failures.is_empty());
    assert!(envelope.error.is_none());
    assert_eq!(envelope.retry_after_ms, None);
}
