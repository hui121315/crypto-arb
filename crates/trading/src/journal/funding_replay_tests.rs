use super::*;
use super::funding_tests::{arbitrage_intent, mark_filled, submit_intent};
use shared_types::HedgeLegRole;

#[test]
fn cross_venue_funding_replay_preserves_run_leg_and_order_fact_links() {
    let ledger_path = temp_storage_path("funding-ledger");
    let snapshot_path = temp_storage_path("funding-snapshots");
    let long = arbitrage_intent("hedge-1-long", "c-long", "hyperliquid", "BTC-USDC");
    let short = arbitrage_intent("hedge-1-short", "c-short", "okx", "BTC-USDT");

    {
        let journal = OrderJournal::new_with_storage_paths(
            Some(ledger_path.clone()),
            Some(snapshot_path.clone()),
        );
        attach_context(&journal, &long, HedgeLegRole::Long);
        attach_context(&journal, &short, HedgeLegRole::Short);
        submit_intent(&journal, &long);
        submit_intent(&journal, &short);
        mark_filled(&journal, &long, "hl-order");
        mark_filled(&journal, &short, "okx-order");
    }

    {
        let journal = OrderJournal::new_with_storage_paths(
            Some(ledger_path.clone()),
            Some(snapshot_path.clone()),
        );
        let long_event = journal
            .record_funding_by_venue_symbol_reported(
                "hyperliquid",
                "BTC",
                &FundingLedgerInput {
                    venue_event_id: "hl-funding:BTC:20".into(),
                    amount: -0.12,
                    currency: "USDC".into(),
                    funding_time_ms: 20,
                },
                OrderUpdateSource::PrivateWs,
                21,
            )
            .expect("replayed long funding ledger event");
        let short_event = journal
            .record_funding_by_venue_symbol_reported(
                "okx",
                "BTC",
                &FundingLedgerInput {
                    venue_event_id: "okx-funding:BTC:20".into(),
                    amount: 0.08,
                    currency: "USDT".into(),
                    funding_time_ms: 20,
                },
                OrderUpdateSource::PrivateWs,
                21,
            )
            .expect("replayed short funding ledger event");

        assert_funding_link(&long_event, &long, HedgeLegRole::Long, "hl-order");
        assert_funding_link(&short_event, &short, HedgeLegRole::Short, "okx-order");
    }

    let restored = OrderJournal::new_with_storage_paths(
        Some(ledger_path.clone()),
        Some(snapshot_path.clone()),
    );
    let events = restored.ledger_events();
    assert!(events.iter().any(|event| {
        event.event_id == "funding_payment:hedge-1-long:private_ws:hl-funding:BTC:20"
            && event.order.run_id.as_deref() == Some("run-1")
            && event.order.ticket_id.as_deref() == Some("ticket-1")
            && event.order.leg_role == Some(HedgeLegRole::Long)
            && event.source == OrderUpdateSource::PrivateWs
            && event.order.identity.internal_order_id == "hedge-1-long"
    }));
    assert!(events.iter().any(|event| {
        event.event_id == "funding_payment:hedge-1-short:private_ws:okx-funding:BTC:20"
            && event.order.run_id.as_deref() == Some("run-1")
            && event.order.ticket_id.as_deref() == Some("ticket-1")
            && event.order.leg_role == Some(HedgeLegRole::Short)
            && event.source == OrderUpdateSource::PrivateWs
            && event.order.identity.internal_order_id == "hedge-1-short"
    }));

    let _ = std::fs::remove_file(ledger_path);
    let _ = std::fs::remove_file(snapshot_path);
}

fn attach_context(journal: &OrderJournal, intent: &OrderIntent, role: HedgeLegRole) {
    journal.attach_execution_ledger_context(
        &intent.id,
        ExecutionLedgerOrderContext::new("run-1".into(), "ticket-1".into(), role),
    );
}

fn assert_funding_link(
    event: &ExecutionLedgerEvent,
    intent: &OrderIntent,
    role: HedgeLegRole,
    exchange_order_id: &str,
) {
    assert_eq!(event.source, OrderUpdateSource::PrivateWs);
    assert_eq!(event.order.run_id.as_deref(), Some("run-1"));
    assert_eq!(event.order.ticket_id.as_deref(), Some("ticket-1"));
    assert_eq!(event.order.leg_role, Some(role));
    assert_eq!(event.order.identity.internal_order_id, intent.id);
    assert_eq!(
        event.order.identity.exchange_order_id.as_deref(),
        Some(exchange_order_id)
    );
}

fn temp_storage_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "crypto-arb-{label}-{}-{}.jsonl",
        std::process::id(),
        common::time::now_ms(),
    ))
}
