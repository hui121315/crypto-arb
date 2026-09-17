use super::*;
use async_trait::async_trait;
use exchange::{ExchangeCapabilities, ExchangeResult, LiveTradingAdapter};
use shared_types::{
    CancelOrderRequest, ExecutionMode, LiveOrderState, OrderAck, OrderInfo, OrderSide, OrderSource,
    OrderType, PositionInfo, StrategyKind,
};
use std::sync::Arc;

struct FundingPaymentAdapter {
    rows: Vec<FundingPaymentData>,
}

#[async_trait]
impl LiveTradingAdapter for FundingPaymentAdapter {
    fn name(&self) -> &'static str {
        "funding_payment_test"
    }

    fn capabilities(&self) -> ExchangeCapabilities {
        ExchangeCapabilities::testnet_limit_only()
    }

    async fn place_order(&self, intent: &OrderIntent) -> ExchangeResult<OrderAck> {
        Ok(OrderAck {
            internal_order_id: intent.id.clone(),
            exchange_order_id: Some("x1".into()),
            client_order_id: intent.client_order_id.clone(),
            identity_update: Default::default(),
            state: LiveOrderState::Accepted,
            accepted_at_ms: intent.created_at_ms,
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        })
    }

    async fn cancel_order(&self, request: &CancelOrderRequest) -> ExchangeResult<OrderAck> {
        Ok(OrderAck {
            internal_order_id: request.internal_order_id.clone(),
            exchange_order_id: request.exchange_order_id.clone(),
            client_order_id: request.client_order_id.clone(),
            identity_update: Default::default(),
            state: LiveOrderState::Cancelled,
            accepted_at_ms: 1,
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        })
    }

    async fn get_order(
        &self,
        _symbol: &str,
        _client_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        Ok(None)
    }

    async fn get_open_orders(&self, _symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        Ok(Vec::new())
    }

    async fn get_positions(&self, _symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>> {
        Ok(Vec::new())
    }

    async fn get_funding_payments(
        &self,
        _symbol: Option<&str>,
        _start_time_ms: Option<i64>,
        _end_time_ms: Option<i64>,
    ) -> ExchangeResult<Vec<FundingPaymentData>> {
        Ok(self.rows.clone())
    }
}

#[tokio::test]
async fn private_funding_payment_ingest_is_idempotent_for_repeated_lookback() {
    let service = TradingService::new_mock();
    seed_filled_arbitrage_order(&service);
    service.engine.set_adapter(Arc::new(FundingPaymentAdapter {
        rows: vec![payment_row()],
    }));

    let first = service
        .ingest_private_funding_payments(Some(1), Some(20))
        .await;
    let second = service
        .ingest_private_funding_payments(Some(1), Some(20))
        .await;

    assert_eq!(first.fetched, 1);
    assert_eq!(first.ledger_events, 1);
    assert_eq!(second.fetched, 1);
    assert_eq!(second.ledger_events, 0);
    assert_eq!(second.skipped, 1);
    assert_eq!(second.duplicate_or_already_recorded, 1);
    assert_eq!(second.unmatched_or_ambiguous_order, 0);
    assert_eq!(
        service
            .latest_private_funding_payment_ingest_report()
            .as_ref()
            .map(|report| report.duplicate_or_already_recorded),
        Some(1)
    );
    let funding_events = service
        .journal
        .ledger_events()
        .into_iter()
        .filter(|event| event.event_type == shared_types::ExecutionLedgerEventType::FundingPayment)
        .collect::<Vec<_>>();
    assert_eq!(funding_events.len(), 1);
    assert_eq!(
        funding_events[0].source,
        shared_types::OrderUpdateSource::FundingPoller
    );
    assert_ne!(
        funding_events[0].source,
        shared_types::OrderUpdateSource::PrivateWs
    );
    assert!(funding_events[0].event_id.contains(":funding_poller:"));
}

#[test]
fn funding_payment_ingest_batch_preserves_events_for_runtime_projection() {
    let service = TradingService::new_mock();
    seed_filled_arbitrage_order(&service);

    let batch = service.ingest_private_funding_payment_rows(vec![payment_row()], Some(1), Some(20));

    assert_eq!(batch.report.fetched, 1);
    assert_eq!(batch.report.ledger_events, 1);
    assert_eq!(batch.ledger_events.len(), 1);
    let event = &batch.ledger_events[0];
    assert_eq!(event.source, shared_types::OrderUpdateSource::FundingPoller);
    assert!(service
        .journal
        .ledger_events()
        .iter()
        .any(|stored| stored.event_id == event.event_id));
}

#[test]
fn funding_storage_failure_does_not_report_ledger_events_as_ingested() {
    let service = TradingService::new_mock();
    let report = PrivateFundingPaymentIngestReport {
        fetched: 2,
        mapped: 2,
        ledger_events: 2,
        ..PrivateFundingPaymentIngestReport::default()
    };

    let failed =
        service.record_private_funding_payment_storage_error(report, "commit ACK timed out");

    assert_eq!(failed.fetched, 2);
    assert_eq!(failed.mapped, 2);
    assert_eq!(failed.ledger_events, 0);
    assert_eq!(
        failed.fetch_error.as_deref(),
        Some("funding ledger durability failed: commit ACK timed out")
    );
    assert!(!failed.is_success());
    assert_eq!(
        service
            .latest_private_funding_payment_ingest_report()
            .as_ref()
            .map(|report| report.ledger_events),
        Some(0)
    );
}

#[tokio::test]
async fn private_funding_payment_ingest_reports_unmatched_or_ambiguous_skip() {
    let service = TradingService::new_mock();
    service.engine.set_adapter(Arc::new(FundingPaymentAdapter {
        rows: vec![payment_row()],
    }));

    let report = service
        .ingest_private_funding_payments(Some(1), Some(20))
        .await;

    assert_eq!(report.fetched, 1);
    assert_eq!(report.mapped, 1);
    assert_eq!(report.ledger_events, 0);
    assert_eq!(report.skipped, 1);
    assert_eq!(report.duplicate_or_already_recorded, 0);
    assert_eq!(report.no_matching_order, 1);
    assert_eq!(report.unmatched_or_ambiguous_order, 1);
    assert_eq!(
        report.skip_reasons[0].reason,
        shared_types::FundingPaymentIngestSkipReason::NoMatchingOrder
    );
}

#[tokio::test]
async fn private_funding_payment_ingest_reports_missing_fill_anchor_skip() {
    let service = TradingService::new_mock();
    seed_submitted_arbitrage_order(&service);
    service.engine.set_adapter(Arc::new(FundingPaymentAdapter {
        rows: vec![payment_row()],
    }));

    let report = service
        .ingest_private_funding_payments(Some(1), Some(20))
        .await;

    assert_eq!(report.fetched, 1);
    assert_eq!(report.mapped, 1);
    assert_eq!(report.ledger_events, 0);
    assert_eq!(report.skipped, 1);
    assert_eq!(report.no_filled_anchor, 1);
    assert_eq!(report.unmatched_or_ambiguous_order, 1);
    assert_eq!(
        report.skip_reasons[0].reason,
        shared_types::FundingPaymentIngestSkipReason::NoFilledAnchor
    );
}

#[tokio::test]
async fn private_funding_payment_ingest_treats_mock_adapter_as_unsupported() {
    let service = TradingService::new_mock();

    let report = service
        .ingest_private_funding_payments(Some(1), Some(20))
        .await;

    assert!(report.is_success());
    assert!(report.unsupported);
    assert_eq!(report.fetched, 0);
    assert_eq!(
        service
            .latest_private_funding_payment_ingest_report()
            .as_ref()
            .map(|report| report.unsupported),
        Some(true)
    );
}

fn seed_filled_arbitrage_order(service: &TradingService) {
    seed_submitted_arbitrage_order(service);
    let _ = service.journal.apply_ack(&OrderAck {
        internal_order_id: "hedge-1-long".into(),
        exchange_order_id: Some("x1".into()),
        client_order_id: "client-1".into(),
        identity_update: Default::default(),
        state: LiveOrderState::Filled,
        accepted_at_ms: 4,
        message: None,
        filled_quantity: Some(1.0),
        filled_price: Some(50_000.0),
        filled_fee: Some(0.01),
    });
}

fn seed_submitted_arbitrage_order(service: &TradingService) {
    let intent = OrderIntent {
        id: "hedge-1-long".into(),
        source: OrderSource::ArbitragePreview,
        strategy: Some(StrategyKind::PerpCross),
        mode: ExecutionMode::DryRun,
        exchange: "binance".into(),
        symbol: "BTCUSDT".into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 1.0,
        price: Some(50_000.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: shared_types::TimeInForce::Gtc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "client-1".into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    };
    service.journal.insert_created(intent, 1);
    let _ = service
        .journal
        .mark_risk_checked("hedge-1-long", RiskDecision::allow(50_000.0), 2);
    let _ = service.journal.mark_submitted("hedge-1-long", 3);
}

fn payment_row() -> FundingPaymentData {
    FundingPaymentData {
        venue: "binance".into(),
        symbol: "BTCUSDT".into(),
        amount: -0.12,
        currency: "USDT".into(),
        funding_time_ms: 10,
        venue_event_id: "binance-funding:BTCUSDT:10".into(),
    }
}

include!("tests/kucoin_symbols.rs");
