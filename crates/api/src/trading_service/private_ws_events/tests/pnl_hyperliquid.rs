use super::*;

#[tokio::test]
async fn hyperliquid_fill_evidence_is_order_linked_and_json_round_trippable() {
    let service = TradingService::new_mock();
    seed_accepted_order(
        &service,
        arbitrage_intent_on(
            "hl-liquidation-fill",
            "hl-liquidation-client",
            OrderSide::Sell,
            "hyperliquid",
            "BTC-USDC",
        ),
        "11223344",
    );
    let fill = private_fill_on(FillFixture {
        venue: "hyperliquid",
        exchange_order_id: "11223344",
        venue_event_id: "hyperliquid-fill:998877",
        quantity: 0.01,
        price: 101_250.5,
        fee_amount: 0.405002,
        occurred_at_ms: 1_784_160_000_123,
    });
    let transport_metadata = OrderTransportMetadata::default().with_venue_fill_evidence(
        shared_types::VenueFillTransportEvidence {
            venue_closed_pnl: "-12.375".to_owned(),
            liquidation: Some(shared_types::VenueLiquidationTransportEvidence {
                liquidated_user: Some("0x2222222222222222222222222222222222222222".to_owned()),
                mark_price: "101200.25".to_owned(),
                method: shared_types::VenueLiquidationMethod::Backstop,
            }),
        },
    );

    let outcome = service
        .apply_private_ws_event(PrivateWsEvent::FillWithEvidence(Box::new(
            PrivateFillWithEvidenceDelta {
                fill,
                transport_metadata,
            },
        )))
        .await;

    assert!(outcome.ledger_updated);
    let fill_event = outcome
        .ledger_events
        .iter()
        .find(|event| event.event_type == ExecutionLedgerEventType::FillEvent)
        .expect("order-linked fill event");
    assert_eq!(
        fill_event.order.identity.internal_order_id,
        "hl-liquidation-fill"
    );
    let evidence = fill_event
        .order
        .identity
        .transport_metadata
        .venue_fill_evidence
        .as_ref()
        .expect("venue fill evidence");
    assert_eq!(evidence.venue_closed_pnl, "-12.375");
    assert_eq!(
        evidence
            .liquidation
            .as_ref()
            .map(|liquidation| liquidation.method),
        Some(shared_types::VenueLiquidationMethod::Backstop)
    );

    let encoded = serde_json::to_string(fill_event).expect("serialize ledger event");
    let replayed: ExecutionLedgerEvent =
        serde_json::from_str(&encoded).expect("replay ledger event");
    assert_eq!(replayed, *fill_event);
}
