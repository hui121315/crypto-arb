use super::super::*;
use super::*;

#[tokio::test]
async fn executed_envelope_from_trading_carries_funding_payment_ingest_report() {
    let service = TradingService::new_mock();
    let report = service
        .ingest_private_funding_payments(Some(1), Some(20))
        .await;

    let envelope =
        executed_envelope_from_trading(&service, &[], 1, &ReviewPageQuery::default()).await;

    assert!(report.unsupported);
    assert!(envelope
        .funding_payment_ingest
        .as_ref()
        .is_some_and(|report| report.unsupported));
}
