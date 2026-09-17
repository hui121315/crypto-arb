use super::*;
use crate::OrderbookDepthLedgerInput;
use shared_types::{
    ExecutionLedgerPayload, ExecutionMode, OrderSide, OrderSource, OrderType, RiskDecision,
    TimeInForce,
};

#[test]
fn execution_ledger_jsonl_replays_orderbook_evidence_after_restart() {
    let path = temp_ledger_path("orderbook-replay");
    {
        let journal = OrderJournal::new_with_ledger_path(path.clone());
        let intent = intent("hedge-1-long", "c1");
        submit_intent(&journal, &intent);
        journal
            .record_orderbook_evidence(
                &intent.id,
                &OrderbookDepthLedgerInput {
                    reference_price: Some(100.0),
                    bid: Some(99.9),
                    ask: Some(100.1),
                    mid: Some(100.0),
                    open_vwap_price: Some(100.1),
                    open_slippage_bps: Some(1.0),
                    close_vwap_price: Some(99.9),
                    close_slippage_bps: Some(1.0),
                    depth_usd_5bps: Some(500.0),
                    depth_usd_10bps: Some(1000.0),
                    depth_usd_20bps: Some(1500.0),
                    max_notional_usd: Some(1500.0),
                    market_timestamp_ms: Some(20),
                    health: None,
                    reason: None,
                    quality: shared_types::ExecutionLedgerQuality::Actual,
                },
                OrderUpdateSource::Internal,
                21,
            )
            .expect("orderbook ledger event");
    }

    let restored = OrderJournal::new_with_ledger_path(path.clone());
    let events = restored.ledger_events();

    assert!(events.iter().any(|event| {
        event
            .event_id
            .starts_with("orderbook_evidence:hedge-1-long:")
            && event.event_type == shared_types::ExecutionLedgerEventType::OrderbookEvidence
            && matches!(
                &event.payload,
                ExecutionLedgerPayload::OrderbookEvidence(record)
                    if record.max_notional_usd == Some(1500.0)
            )
    }));
    let _ = std::fs::remove_file(path);
}

fn intent(id: &str, client_order_id: &str) -> OrderIntent {
    OrderIntent {
        id: id.into(),
        source: OrderSource::ArbitragePreview,
        strategy: Some(shared_types::StrategyKind::PerpCross),
        mode: ExecutionMode::DryRun,
        exchange: "mock".into(),
        symbol: "BTC".into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 1.0,
        price: Some(10.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: client_order_id.into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

fn submit_intent(journal: &OrderJournal, intent: &OrderIntent) {
    journal.insert_created(intent.clone(), 1);
    journal
        .mark_risk_checked(&intent.id, RiskDecision::allow(10.0), 2)
        .expect("risk checked");
    journal.mark_submitted(&intent.id, 3).expect("submitted");
}

fn temp_ledger_path(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let mut path = std::env::temp_dir();
    path.push(format!(
        "crossline-{label}-{}-{nanos}.jsonl",
        std::process::id()
    ));
    path
}
