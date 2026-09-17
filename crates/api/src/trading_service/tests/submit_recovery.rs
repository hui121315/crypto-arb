use super::support::*;
use super::*;

#[tokio::test]
async fn ambiguous_submit_timeout_recovers_by_client_order_id_without_resubmitting() {
    let mut remote = order_info("venue-order-1", OrderStatus::Filled, 0.01);
    remote.client_order_id = Some("client-submit-timeout-recovered".to_owned());
    let (service, adapter) = service_with_submit_timeout(Some(remote));
    let intent = limit_intent("submit-timeout-recovered");

    let record = must_ok(
        service.submit(intent).await,
        "read-side order query should recover the ambiguous submit",
    );

    assert_eq!(record.state, LiveOrderState::Filled);
    assert_eq!(record.exchange_order_id.as_deref(), Some("venue-order-1"));
    assert_eq!(
        adapter.exchange_order_query_ids(),
        vec!["client-submit-timeout-recovered"]
    );
}

#[tokio::test]
async fn ambiguous_submit_timeout_stays_unknown_when_query_has_no_order() {
    let (service, adapter) = service_with_submit_timeout(None);
    let intent = limit_intent("submit-timeout-unconfirmed");

    let error = must_err(
        service.submit(intent.clone()).await,
        "an unconfirmed write must remain failed closed",
    );
    let record = must_some(
        service.get_order(&intent.id),
        "ambiguous order journal record",
    );

    assert!(matches!(
        error,
        TradingError::Exchange(ExchangeError::Timeout { seconds: 10 })
    ));
    assert_eq!(record.state, LiveOrderState::Unknown);
    assert_eq!(
        adapter.exchange_order_query_ids(),
        vec!["client-submit-timeout-unconfirmed"]
    );
}
