use super::*;
use shared_types::{
    ListPage, ListStatus, ReviewDataSource, ReviewLedgerStatus, VenueOperationHealth,
    VenueOperationStatus, VenueQualityEnvelope, VenueQualitySource,
};

#[test]
fn latest_response_accepts_only_matching_token() {
    assert!(is_latest_response(3, 3));
    assert!(!is_latest_response(3, 2));
    assert!(!is_latest_response(3, 4));
}

#[test]
fn review_retry_deadline_uses_typed_retry_after() {
    assert_eq!(review_retry_deadline_ms(Some(2_000), 10_000), Some(12_000));
    assert_eq!(review_retry_deadline_ms(Some(0), 10_000), None);
    assert_eq!(review_retry_deadline_ms(None, 10_000), None);
}

#[test]
fn review_poll_waits_until_retry_deadline() {
    assert!(!review_poll_allowed(Some(12_000), 11_999));
    assert!(review_poll_allowed(Some(12_000), 12_000));
    assert!(review_poll_allowed(None, 11_999));
}

#[test]
fn review_retry_deadline_uses_error_problem() {
    let retrying: Result<ReviewEnvelope<u8>, ApiProblem> =
        Err(ApiProblem::new("RATE_LIMITED", "slow").with_retry_after_ms(Some(2_000)));
    let success: Result<ReviewEnvelope<u8>, ApiProblem> = Ok(review_envelope(Vec::new()));

    assert_eq!(
        review_retry_deadline_for_result(&retrying, 10_000),
        Some(12_000)
    );
    assert_eq!(review_retry_deadline_for_result(&success, 10_000), None);
}

#[test]
fn review_retry_deadline_uses_degraded_envelope_problems() {
    let envelope = review_envelope(vec![1_u8]).with_page(
        page(1),
        ListStatus::Degraded,
        vec![
            ApiProblem::new("REVIEW_LEDGER_INCOMPLETE", "ledger degraded")
                .with_retry_after_ms(Some(2_000)),
            ApiProblem::new("REVIEW_STORAGE_STALE", "storage stale")
                .with_retry_after_ms(Some(3_000)),
        ],
    );

    assert_eq!(
        review_retry_deadline_for_result(&Ok(envelope), 10_000),
        Some(13_000)
    );
}

#[test]
fn review_retry_deadline_uses_storage_health_retry_after() {
    let mut envelope = review_envelope(Vec::<u8>::new()).with_storage_health(storage_health(
        Some(1_000),
        Some(
            ApiProblem::new("REVIEW_STORAGE_STALE", "storage stale")
                .with_retry_after_ms(Some(4_000)),
        ),
    ));
    envelope.problems = vec![
        ApiProblem::new("REVIEW_LEDGER_INCOMPLETE", "ledger degraded")
            .with_retry_after_ms(Some(2_000)),
    ];

    assert_eq!(
        review_retry_deadline_for_result(&Ok(envelope), 10_000),
        Some(14_000)
    );
}

#[test]
fn venue_quality_retry_deadline_uses_envelope_retry_after() {
    let mut envelope =
        VenueQualityEnvelope::new(Vec::new(), 1_000, VenueQualitySource::RuntimeSamples);
    envelope.retry_after_ms = Some(15_000);

    assert_eq!(
        review_retry_deadline_for_result(&Ok(envelope), 10_000),
        Some(25_000)
    );
}

fn review_envelope(rows: Vec<u8>) -> ReviewEnvelope<u8> {
    ReviewEnvelope::new(
        rows,
        1_000,
        30,
        ReviewDataSource::ExecutionLedger,
        Some(ReviewLedgerStatus::PartialEvidence),
        Vec::new(),
    )
}

fn page(returned_count: usize) -> ListPage {
    ListPage {
        limit: returned_count,
        max_limit: 30,
        start_offset: 0,
        returned_count,
        total_rows: returned_count,
        has_more: false,
        next_cursor: None,
        ..ListPage::default()
    }
}

fn storage_health(
    retry_after_ms: Option<u64>,
    problem: Option<ApiProblem>,
) -> VenueOperationHealth {
    VenueOperationHealth {
        venue: "review".to_owned(),
        operation: "storage:review_execution_ledger".to_owned(),
        status: VenueOperationStatus::Warn,
        source: "test".to_owned(),
        message: "storage stale".to_owned(),
        supported: Some(true),
        configured: Some(true),
        requested: None,
        rows: None,
        freshness_ms: None,
        retry_after_ms,
        latency_ms: None,
        latency_p95_ms: None,
        error: None,
        evidence: None,
        problem,
        observed_at_ms: 1_000,
    }
}
