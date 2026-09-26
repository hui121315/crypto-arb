use super::*;
use shared_types::{
    CloseRunCostReconciliation, ExecutedTrade, ExecutionLedgerQuality, FeeProduct, OrderSide,
    ReviewLedgerEventTiming, ReviewLedgerOrderEvidence, ReviewPnlEvidence, StrategyKind,
    VenueOrderIdentity,
};

#[test]
fn ledger_event_drilldown_surfaces_sources_ids_and_payloads() {
    let mut row = trade();
    row.evidence.ledger_events = vec![
        ledger_event(
            "fill-long",
            ExecutionLedgerEventType::FillSnapshot,
            OrderUpdateSource::PrivateWs,
            ReviewLedgerPayloadEvidence::Fill {
                quantity: 1.0,
                average_price: 100.0,
                quote_value: 100.0,
                quality: ExecutionLedgerQuality::Actual,
                confidence: ExecutionFillConfidence::VenueFill,
                fee: None,
            },
        ),
        ledger_event(
            "book-long",
            ExecutionLedgerEventType::OrderbookEvidence,
            OrderUpdateSource::Internal,
            ReviewLedgerPayloadEvidence::Orderbook {
                reference_price: Some(100.0),
                bid: Some(99.9),
                ask: Some(100.1),
                mid: Some(100.0),
                open_vwap_price: Some(100.1),
                open_slippage_bps: Some(1.0),
                close_vwap_price: Some(99.9),
                close_slippage_bps: None,
                depth_usd_5bps: Some(500.0),
                depth_usd_10bps: Some(1000.0),
                depth_usd_20bps: Some(1500.0),
                max_notional_usd: Some(1500.0),
                market_timestamp_ms: Some(900),
                health: None,
                reason: None,
                quality: ExecutionLedgerQuality::Actual,
            },
        ),
    ];
    row.evidence
        .record_close_run_evidence(ReviewCloseRunEvidence {
            close_run_id: "close-1".into(),
            status: CloseRunStatus::Compensated,
            run_id: "run-1".into(),
            ticket_id: "ticket-1".into(),
            opportunity_id: "opp-1".into(),
            matched_notional_usd: 100.0,
            unwind_status: Some(CloseRunUnwindPlanStatus::Compensated),
            compensation_attempt_count: 1,
            cost_reconciliation: Some(CloseRunCostReconciliation {
                close_fee_usd: Some(0.2),
                close_slippage_usd: Some(0.3),
                compensation_fee_usd: Some(0.4),
                compensation_slippage_usd: Some(0.5),
                funding_usd: Some(-0.6),
                manual_handling_usd: Some(0.7),
                total_actual_cost_usd: Some(1.5),
                evidence_event_ids: vec![
                    "close-fee-1".into(),
                    "close-slip-1".into(),
                    "comp-fee-1".into(),
                    "comp-slip-1".into(),
                    "funding-1".into(),
                    "manual-1".into(),
                ],
                close_fee_event_ids: vec!["close-fee-1".into()],
                close_slippage_event_ids: vec!["close-slip-1".into()],
                compensation_fee_event_ids: vec!["comp-fee-1".into()],
                compensation_slippage_event_ids: vec!["comp-slip-1".into()],
                funding_event_ids: vec!["funding-1".into()],
                manual_handling_event_ids: vec!["manual-1".into()],
                ..CloseRunCostReconciliation::default()
            }),
        });

    let summary = ledger_event_drilldown_summary(&row);

    assert!(summary.contains("明细 2 条"));
    assert!(summary.contains("私有 WS"));
    assert!(summary.contains("内部状态"));
    assert!(summary.contains("fill-long fill"));
    assert!(summary.contains("run:run-1 ticket:ticket-1 via 私有 WS"));
    assert!(summary.contains("数量 1 · 成交价 100"));
    assert!(summary.contains("book-long book"));
    assert!(summary.contains("run:run-1 ticket:ticket-1 via 内部状态"));
    assert!(summary.contains("20bps 深度 $1500"));
    assert!(summary.contains("CloseRun 1 条"));
    assert!(summary.contains("close-1 compensated"));
    assert!(summary.contains("cost:6"));
    assert!(summary.contains("close_fee:$0.2/1"));
    assert!(summary.contains("close_slip:$0.3/1"));
    assert!(summary.contains("comp_fee:$0.4/1"));
    assert!(summary.contains("comp_slip:$0.5/1"));
    assert!(summary.contains("funding:$-0.6/1"));
    assert!(summary.contains("manual:$0.7/1"));
    assert!(summary.contains("total:$1.5 missing:none"));
}

fn trade() -> ExecutedTrade {
    ExecutedTrade {
        id: "hedge-1".into(),
        strategy: StrategyKind::PerpCross,
        symbol: "BTC".into(),
        long_venue: "binance".into(),
        short_venue: "okx".into(),
        opened_at_ms: 0,
        closed_at_ms: Some(1),
        holding_minutes: Some(1),
        gross_pnl_usd: 1.0,
        fee_usd: 0.1,
        funding_usd: 0.0,
        slippage_usd: 0.0,
        net_pnl_usd: 0.9,
        evidence: ReviewPnlEvidence::default(),
        actual_fields: Vec::new(),
        estimated_fields: Vec::new(),
        missing_fields: Vec::new(),
        long_orders: Vec::new(),
        short_orders: Vec::new(),
    }
}

fn ledger_event(
    event_id: &str,
    event_type: ExecutionLedgerEventType,
    source: OrderUpdateSource,
    payload: ReviewLedgerPayloadEvidence,
) -> ReviewLedgerEventEvidence {
    ReviewLedgerEventEvidence {
        event_id: event_id.into(),
        event_type,
        source,
        order: ReviewLedgerOrderEvidence {
            run_id: Some("run-1".into()),
            ticket_id: Some("ticket-1".into()),
            leg_role: None,
            reduce_only: Some(false),
            exchange: "binance".into(),
            symbol: "BTCUSDT".into(),
            side: OrderSide::Buy,
            identity: VenueOrderIdentity {
                account_scope: None,
                internal_order_id: "order-1".into(),
                public_client_order_id: "client-1".into(),
                product: FeeProduct::Perp,
                venue_client_order_id: None,
                exchange_order_id: Some("ex-1".into()),
                client_order_id_policy: None,
                transport_metadata: Default::default(),
            },
        },
        timing: ReviewLedgerEventTiming {
            occurred_at_ms: 1,
            captured_at_ms: 2,
        },
        payload,
    }
}
