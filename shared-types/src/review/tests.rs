use super::*;
use crate::execution_ledger::{
    ExecutionFillConfidence, ExecutionLedgerEvent, ExecutionLedgerEventType,
    ExecutionLedgerOrderRef, ExecutionLedgerPayload, ExecutionLedgerQuality, FillLedgerSnapshot,
    SlippageLedgerRecord,
};
use crate::live_trading::{OrderUpdateSource, VenueOrderIdentity};
use crate::venues::{VenueOperationHealth, VenueOperationStatus};
use crate::{
    ClientOrderIdPolicy, CloseRunCostReconciliation, CloseRunStatus, CloseRunUnwindPlanStatus,
    OrderSide,
};

#[test]
fn review_envelope_serializes_storage_health_contract() {
    let envelope = ReviewEnvelope::new(
        Vec::<u8>::new(),
        1_000,
        30,
        ReviewDataSource::ExecutionLedger,
        Some(ReviewLedgerStatus::NoLedgerEvents),
        Vec::new(),
    )
    .with_page(
        ListPage::default(),
        ListStatus::Degraded,
        vec![ApiProblem::new("REVIEW_STORAGE_STALE", "storage stale")],
    )
    .with_storage_health(storage_health())
    .with_request_id(Some("req-review-1".to_owned()));

    let text = serde_json::to_string(&envelope).unwrap_or_default();

    assert!(text.contains("\"storageHealth\""));
    assert!(text.contains("\"operation\":\"storage:review_execution_ledger\""));
    assert!(text.contains("\"configured\":false"));
    assert!(text.contains("\"requestId\":\"req-review-1\""));
    assert_eq!(
        envelope.problems[0].request_id.as_deref(),
        Some("req-review-1")
    );
}

#[test]
fn review_envelope_deserializes_without_storage_health() {
    let envelope: ReviewEnvelope<u8> =
        serde_json::from_str(r#"{"rows":[],"generatedAtMs":1,"days":1,"source":"execution_ledger","rowCount":0,"page":{"limit":0,"maxLimit":0,"startOffset":0,"returnedCount":0,"totalRows":0,"hasMore":false},"status":"fresh"}"#)
            .unwrap_or_else(|error| panic!("deserialize ReviewEnvelope without storage health: {error}"));

    assert!(envelope.storage_health.is_none());
    assert!(envelope.funding_payment_ingest.is_none());
}

#[test]
fn review_envelope_serializes_funding_payment_ingest_report() {
    let report = FundingPaymentIngestReport {
        observed_at_ms: 1_700,
        window_start_ms: Some(1_000),
        window_end_ms: Some(2_000),
        fetched: 2,
        mapped: 2,
        ledger_events: 1,
        skipped: 1,
        duplicate_or_already_recorded: 1,
        ..FundingPaymentIngestReport::default()
    };
    let envelope = ReviewEnvelope::new(
        Vec::<u8>::new(),
        1_000,
        30,
        ReviewDataSource::ExecutionLedger,
        Some(ReviewLedgerStatus::PartialEvidence),
        vec![ReviewPnlField::Funding],
    )
    .with_funding_payment_ingest(report);

    let value = serde_json::to_value(envelope).unwrap_or_default();

    assert_eq!(value["fundingPaymentIngest"]["fetched"], 2);
    assert_eq!(
        value["fundingPaymentIngest"]["duplicateOrAlreadyRecorded"],
        1
    );
}

#[test]
fn pnl_evidence_keeps_lowest_fill_confidence() {
    let mut evidence = ReviewPnlEvidence::default();

    evidence.record_fill_confidence(ExecutionFillConfidence::VenueFill);
    evidence.record_fill_confidence(ExecutionFillConfidence::AdapterAck);
    evidence.record_fill_confidence(ExecutionFillConfidence::OrderQuery);

    assert_eq!(
        evidence.fill_confidence,
        Some(ExecutionFillConfidence::AdapterAck)
    );
    assert_eq!(evidence.fill_confidence_score, Some(0.65));
}

#[test]
fn pnl_evidence_requires_slippage_for_every_fill() {
    let fill_a = fill_event_for("fill-a", "order-a");
    let fill_b = fill_event_for("fill-b", "order-b");
    let slip_a = slippage_event_for(&fill_a, "slip-a");
    let slip_b = slippage_event_for(&fill_b, "slip-b");
    let mut evidence = ReviewPnlEvidence {
        fill_event_ids: vec!["fill-a".into(), "fill-b".into()],
        slippage_event_ids: vec!["slip-a".into()],
        ..ReviewPnlEvidence::default()
    };
    for event in [&fill_a, &fill_b, &slip_a, &slip_b] {
        evidence.record_ledger_event(event);
    }

    assert!(!evidence.has_complete_slippage_evidence());

    evidence.slippage_event_ids.push("slip-b".into());

    assert!(evidence.has_complete_slippage_evidence());
}

#[test]
fn pnl_evidence_records_ledger_event_drilldown_once() {
    let event = fill_event();
    let mut evidence = ReviewPnlEvidence::default();

    evidence.record_ledger_event(&event);
    evidence.record_ledger_event(&event);

    assert_eq!(evidence.ledger_events.len(), 1);
    let detail = &evidence.ledger_events[0];
    assert_eq!(detail.event_id, "fill-1");
    assert_eq!(detail.event_type, ExecutionLedgerEventType::FillSnapshot);
    assert_eq!(detail.source, OrderUpdateSource::PrivateWs);
    assert_eq!(detail.order.run_id.as_deref(), Some("run-1"));
    assert_eq!(detail.order.ticket_id.as_deref(), Some("ticket-1"));
    assert_eq!(detail.order.reduce_only, Some(false));
    assert_eq!(detail.order.exchange, "binance");
    assert_eq!(detail.order.identity.internal_order_id, "order-1");
    assert_eq!(detail.timing.occurred_at_ms, 1_000);
    assert_fill_drilldown(detail);
    assert_ledger_event_json(&evidence);
}

#[test]
fn review_ledger_payload_variants_round_trip() {
    let payloads = vec![
        ReviewLedgerPayloadEvidence::Fee {
            amount: 0.1,
            currency: Some("USDT".into()),
            quality: ExecutionLedgerQuality::Actual,
        },
        ReviewLedgerPayloadEvidence::Funding {
            amount: 0.2,
            currency: "USDT".into(),
            funding_time_ms: 1_000,
            quality: ExecutionLedgerQuality::Actual,
        },
        ReviewLedgerPayloadEvidence::Slippage {
            amount_usd: 0.3,
            reference_price: 99.9,
            fill_price: 100.0,
            quantity: 2.0,
            quality: ExecutionLedgerQuality::Estimated,
        },
        ReviewLedgerPayloadEvidence::Orderbook {
            reference_price: Some(100.0),
            bid: Some(99.9),
            ask: Some(100.1),
            mid: Some(100.0),
            open_vwap_price: None,
            open_slippage_bps: None,
            close_vwap_price: None,
            close_slippage_bps: None,
            depth_usd_5bps: Some(1_000.0),
            depth_usd_10bps: None,
            depth_usd_20bps: None,
            max_notional_usd: Some(1_000.0),
            market_timestamp_ms: Some(1_000),
            health: None,
            reason: None,
            quality: ExecutionLedgerQuality::Actual,
        },
    ];

    for payload in payloads {
        let encoded = serde_json::to_value(&payload).expect("review payload serializes");
        let decoded: ReviewLedgerPayloadEvidence =
            serde_json::from_value(encoded).expect("review payload deserializes");
        assert_eq!(decoded, payload);
    }
}

#[test]
fn pnl_evidence_records_close_run_linkage_once() {
    let mut evidence = ReviewPnlEvidence::default();
    let linkage = ReviewCloseRunEvidence {
        close_run_id: "close-1".into(),
        status: CloseRunStatus::Compensated,
        run_id: "run-1".into(),
        ticket_id: "ticket-1".into(),
        opportunity_id: "opp-1".into(),
        matched_notional_usd: 100.0,
        unwind_status: Some(CloseRunUnwindPlanStatus::Compensated),
        compensation_attempt_count: 1,
        cost_reconciliation: Some(CloseRunCostReconciliation {
            evidence_event_ids: vec!["funding-1".into(), "manual-1".into()],
            total_actual_cost_usd: Some(1.25),
            funding_event_ids: vec!["funding-1".into()],
            manual_handling_event_ids: vec!["manual-1".into()],
            ..CloseRunCostReconciliation::default()
        }),
    };

    evidence.record_close_run_evidence(linkage.clone());
    evidence.record_close_run_evidence(linkage);

    assert_eq!(evidence.close_run_evidence.len(), 1);
    let value = serde_json::to_value(evidence).unwrap_or_default();
    assert_eq!(value["closeRunEvidence"][0]["closeRunId"], "close-1");
    assert_eq!(value["closeRunEvidence"][0]["status"], "compensated");
    assert_eq!(
        value["closeRunEvidence"][0]["costReconciliation"]["evidenceEventIds"][1],
        "manual-1"
    );
}

fn assert_fill_drilldown(detail: &ReviewLedgerEventEvidence) {
    let ReviewLedgerPayloadEvidence::Fill {
        quantity,
        average_price,
        quality,
        confidence,
        fee,
        ..
    } = &detail.payload
    else {
        panic!("expected fill payload");
    };
    assert_eq!(*quantity, 2.0);
    assert_eq!(*average_price, 100.0);
    assert_eq!(*quality, ExecutionLedgerQuality::Actual);
    assert_eq!(*confidence, ExecutionFillConfidence::VenueFill);
    assert_eq!(fee.as_ref().map(|fee| fee.amount), Some(0.2));
}

fn assert_ledger_event_json(evidence: &ReviewPnlEvidence) {
    let value = serde_json::to_value(evidence).unwrap_or_default();
    assert_eq!(value["ledgerEvents"][0]["eventId"], "fill-1");
    assert_eq!(
        value["ledgerEvents"][0]["order"]["identity"]["internalOrderId"],
        "order-1"
    );
    assert_eq!(value["ledgerEvents"][0]["order"]["reduceOnly"], false);
    assert_eq!(value["ledgerEvents"][0]["payload"]["type"], "fill");
    assert_eq!(
        value["ledgerEvents"][0]["payload"]["data"]["fee"]["amount"],
        0.2
    );
}

fn fill_event() -> ExecutionLedgerEvent {
    ExecutionLedgerEvent {
        event_id: "fill-1".into(),
        event_type: ExecutionLedgerEventType::FillSnapshot,
        source: OrderUpdateSource::PrivateWs,
        order: ExecutionLedgerOrderRef {
            run_id: Some("run-1".into()),
            ticket_id: Some("ticket-1".into()),
            leg_role: None,
            reduce_only: Some(false),
            exchange: "binance".into(),
            symbol: "BTCUSDT".into(),
            side: OrderSide::Buy,
            identity: VenueOrderIdentity {
                internal_order_id: "order-1".into(),
                public_client_order_id: "client-1".into(),
                product: crate::FeeProduct::Perp,
                venue_client_order_id: None,
                exchange_order_id: Some("ex-1".into()),
                client_order_id_policy: Some(ClientOrderIdPolicy::default()),
                transport_metadata: Default::default(),
            },
        },
        payload: ExecutionLedgerPayload::FillSnapshot(FillLedgerSnapshot {
            quantity: 2.0,
            average_price: 100.0,
            quote_value: 200.0,
            quality: ExecutionLedgerQuality::Actual,
            confidence: ExecutionFillConfidence::VenueFill,
            fee: Some(crate::execution_ledger::FeeLedgerSnapshot {
                amount: 0.2,
                currency: Some("USDT".into()),
                quality: ExecutionLedgerQuality::Actual,
            }),
        }),
        occurred_at_ms: 1_000,
        captured_at_ms: 1_005,
    }
}

fn fill_event_for(event_id: &str, order_id: &str) -> ExecutionLedgerEvent {
    let mut event = fill_event();
    event.event_id = event_id.into();
    event.order.identity.internal_order_id = order_id.into();
    event
}

fn slippage_event_for(fill: &ExecutionLedgerEvent, event_id: &str) -> ExecutionLedgerEvent {
    let mut event = fill.clone();
    event.event_id = event_id.into();
    event.event_type = ExecutionLedgerEventType::Slippage;
    event.payload = ExecutionLedgerPayload::Slippage(SlippageLedgerRecord {
        amount_usd: 0.1,
        reference_price: 99.9,
        fill_price: 100.0,
        quantity: 1.0,
        quality: ExecutionLedgerQuality::Actual,
    });
    event
}

fn storage_health() -> VenueOperationHealth {
    VenueOperationHealth {
        venue: "system".into(),
        operation: "storage:review_execution_ledger".into(),
        status: VenueOperationStatus::Warn,
        source: "review_store".into(),
        message: "review store is ephemeral".into(),
        supported: Some(true),
        configured: Some(false),
        requested: Some(0),
        rows: Some(0),
        freshness_ms: Some(0),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: None,
        evidence: None,
        problem: None,
        observed_at_ms: 1_000,
    }
}
