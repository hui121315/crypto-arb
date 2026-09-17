use super::*;
use shared_types::{
    ExecutionMode, FundingPaymentIngestSkipReason, OrderSide, OrderSource, OrderType,
};

#[test]
fn record_funding_by_venue_symbol_writes_unique_filled_group() {
    let journal = OrderJournal::default_without_audit();
    let intent = arbitrage_intent("hedge-1-long", "c1", "hyperliquid", "BTC-USDC");
    submit_intent(&journal, &intent);
    mark_filled(&journal, &intent, "x1");

    let event = journal
        .record_funding_by_venue_symbol_reported(
            "hyperliquid",
            "BTC",
            &funding_input(),
            OrderUpdateSource::PrivateWs,
            11,
        )
        .expect("funding ledger event");

    assert_eq!(
        event.event_type,
        shared_types::ExecutionLedgerEventType::FundingPayment
    );
    assert_eq!(event.order.identity.internal_order_id, "hedge-1-long");
    assert!(matches!(
        event.payload,
        ExecutionLedgerPayload::FundingPayment(shared_types::FundingPaymentLedgerRecord {
            amount: -0.12,
            ..
        })
    ));
}

#[test]
fn record_funding_by_venue_symbol_skips_ambiguous_groups() {
    let journal = OrderJournal::default_without_audit();
    for (order_id, client_id, exchange_id) in
        [("hedge-1-long", "c1", "x1"), ("hedge-2-long", "c2", "x2")]
    {
        let intent = arbitrage_intent(order_id, client_id, "hyperliquid", "BTC");
        submit_intent(&journal, &intent);
        mark_filled(&journal, &intent, exchange_id);
    }

    let event = journal.record_funding_by_venue_symbol_reported(
        "hyperliquid",
        "BTC",
        &funding_input(),
        OrderUpdateSource::PrivateWs,
        11,
    );

    assert!(matches!(
        event,
        Err(FundingPaymentIngestSkipReason::AmbiguousOrderGroup)
    ));
    assert!(!journal.ledger_events().iter().any(|event| {
        event.event_type == shared_types::ExecutionLedgerEventType::FundingPayment
    }));
}

#[test]
fn record_funding_by_venue_symbol_reports_missing_fill_anchor() {
    let journal = OrderJournal::default_without_audit();
    let intent = arbitrage_intent("hedge-1-long", "c1", "hyperliquid", "BTC");
    submit_intent(&journal, &intent);

    let event = journal.record_funding_by_venue_symbol_reported(
        "hyperliquid",
        "BTC",
        &funding_input(),
        OrderUpdateSource::PrivateWs,
        11,
    );

    assert!(matches!(
        event,
        Err(FundingPaymentIngestSkipReason::NoFilledAnchor)
    ));
}

#[test]
fn record_funding_by_venue_symbol_reports_no_matching_order() {
    let journal = OrderJournal::default_without_audit();

    let event = journal.record_funding_by_venue_symbol_reported(
        "hyperliquid",
        "BTC",
        &funding_input(),
        OrderUpdateSource::PrivateWs,
        11,
    );

    assert!(matches!(
        event,
        Err(FundingPaymentIngestSkipReason::NoMatchingOrder)
    ));
}

#[test]
fn record_funding_by_venue_symbol_reports_duplicate_event() {
    let journal = OrderJournal::default_without_audit();
    let intent = arbitrage_intent("hedge-1-long", "c1", "hyperliquid", "BTC-USDC");
    submit_intent(&journal, &intent);
    mark_filled(&journal, &intent, "x1");
    let input = funding_input();

    journal
        .record_funding_by_venue_symbol_reported(
            "hyperliquid",
            "BTC",
            &input,
            OrderUpdateSource::PrivateWs,
            11,
        )
        .expect("first funding ledger event");
    let duplicate = journal.record_funding_by_venue_symbol_reported(
        "hyperliquid",
        "BTC",
        &input,
        OrderUpdateSource::PrivateWs,
        12,
    );

    assert!(matches!(
        duplicate,
        Err(FundingPaymentIngestSkipReason::DuplicateOrAlreadyRecorded)
    ));
}

pub(super) fn arbitrage_intent(
    id: &str,
    client_order_id: &str,
    exchange: &str,
    symbol: &str,
) -> OrderIntent {
    OrderIntent {
        id: id.into(),
        source: OrderSource::ArbitragePreview,
        strategy: Some(shared_types::StrategyKind::PerpCross),
        mode: ExecutionMode::DryRun,
        exchange: exchange.to_owned(),
        symbol: symbol.to_owned(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 1.0,
        price: Some(10.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: shared_types::TimeInForce::Ioc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: client_order_id.into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

pub(super) fn submit_intent(journal: &OrderJournal, intent: &OrderIntent) {
    journal.insert_created(intent.clone(), 1);
    journal
        .mark_risk_checked(&intent.id, RiskDecision::allow(10.0), 2)
        .expect("risk checked");
    journal.mark_submitted(&intent.id, 3).expect("submitted");
}

pub(super) fn mark_filled(
    journal: &OrderJournal,
    intent: &OrderIntent,
    exchange_order_id: &str,
) {
    journal
        .apply_ack(&OrderAck {
            internal_order_id: intent.id.clone(),
            exchange_order_id: Some(exchange_order_id.into()),
            client_order_id: intent.client_order_id.clone(),
            identity_update: Default::default(),
            state: LiveOrderState::Filled,
            accepted_at_ms: 4,
            message: None,
            filled_quantity: Some(1.0),
            filled_price: Some(100.0),
            filled_fee: Some(0.01),
        })
        .expect("filled order");
}

fn funding_input() -> FundingLedgerInput {
    FundingLedgerInput {
        venue_event_id: "hl-funding:BTC:10".into(),
        amount: -0.12,
        currency: "USDC".into(),
        funding_time_ms: 10,
    }
}
