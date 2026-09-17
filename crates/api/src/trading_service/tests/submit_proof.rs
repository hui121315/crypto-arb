use super::support::*;
use super::*;

#[tokio::test]
async fn live_submit_problem_records_rate_limit_transport_context() {
    let (service, _adapter) = service_with_submit_rate_limit_adapter(6);
    service.update_risk_config(|config| config.live_trading_enabled = true);
    let mut intent = limit_intent("proof-submit-rate-limit");
    intent.mode = ExecutionMode::Live;

    let result = common::request_id::scope("req-live-submit-429".to_owned(), async {
        service.submit(intent).await
    })
    .await;
    let error = must_err(result, "live submit should rate limit");
    let rows = service
        .live_order_proof_health
        .snapshot(common::time::now_ms());
    let proof = must_some(
        rows.into_iter().find(|row| row.venue == "mock"),
        "live submit problem proof row",
    );

    assert!(matches!(
        error,
        trading::TradingError::Exchange(ExchangeError::RateLimited {
            retry_after_secs: 6
        })
    ));
    assert_eq!(proof.status, shared_types::VenueOperationStatus::Blocked);
    assert_eq!(proof.request_id.as_deref(), Some("req-live-submit-429"));
    assert_eq!(proof.retry_after_ms, Some(6_000));
    assert_eq!(
        proof.last_problem.as_ref().map(|problem| problem.status),
        Some(Some(429))
    );
    assert_eq!(
        proof
            .last_problem
            .as_ref()
            .and_then(|problem| problem.request_id.as_deref()),
        Some("req-live-submit-429")
    );
}
