use super::*;

#[test]
fn records_pnl_on_completion_day_even_when_first_leg_is_older() {
    let orders = hedge_orders();
    let ledger = vec![
        fill_event(&orders[0], 100.0, 0.0, 1_000, "fill-long"),
        fill_event(&orders[1], 80.0, 0.0, DAY_MS + 1_000, "fill-short"),
    ];

    let rows = realized_pnl_by_group(&orders, &ledger, DAY_MS, 2 * DAY_MS);

    assert_eq!(rows["hedge-1"].realized_day_ms, DAY_MS);
    assert_eq!(rows["hedge-1"].price_pnl_usd, -20.0);
}

#[test]
fn ignores_incomplete_groups() {
    let mut orders = hedge_orders();
    orders.truncate(1);
    let ledger = vec![fill_event(&orders[0], 100.0, 0.0, 1_000, "fill-long")];

    let rows = realized_pnl_by_group(&orders, &ledger, 0, 10_000);

    assert!(rows.is_empty());
}

#[test]
fn applies_missing_evidence_to_executed_trade() {
    let orders = hedge_orders();
    let ledger = vec![
        fill_event(&orders[0], 100.0, 0.10, 1_000, "fill-long"),
        fill_event(&orders[1], 102.0, 0.11, 2_000, "fill-short"),
    ];
    let realized = realized_pnl_by_group(&orders, &ledger, 0, 10_000);
    let trades = crate::executed_from_orders(&orders, 10_000, 1);

    let trades = apply_realized_pnl(trades, &realized);

    assert_eq!(trades.len(), 1);
    assert_eq!(
        trades[0].evidence.fill_event_ids,
        ["fill-long", "fill-short"]
    );
    assert_eq!(trades[0].actual_fields, [ReviewPnlField::Fee]);
    assert_eq!(trades[0].estimated_fields, [ReviewPnlField::Slippage]);
    assert_eq!(
        trades[0].missing_fields,
        [
            ReviewPnlField::Gross,
            ReviewPnlField::Funding,
            ReviewPnlField::Net
        ]
    );
}

#[test]
fn realized_pnl_evidence_keeps_lowest_terminal_fill_confidence() {
    let orders = hedge_orders();
    let ledger = vec![
        fill_event_with_confidence(
            &orders[0],
            100.0,
            0.10,
            1_000,
            "fill-long-query",
            shared_types::ExecutionFillConfidence::OrderQuery,
        ),
        incremental_fill_event(&orders[1], 102.0, 0.11, 2_000, "fill-short-ws"),
    ];

    let rows = realized_pnl_by_group(&orders, &ledger, 0, 10_000);
    let evidence = &rows["hedge-1"].evidence;

    assert_eq!(
        evidence.fill_confidence,
        Some(shared_types::ExecutionFillConfidence::OrderQuery)
    );
    assert_eq!(evidence.fill_confidence_score, Some(0.85));
}

#[test]
fn adapter_ack_fill_snapshot_does_not_create_executed_trade() {
    let mut orders = hedge_orders();
    for order in &mut orders {
        order.intent.mode = shared_types::ExecutionMode::Live;
    }
    let ledger = vec![
        fill_event_with_confidence(
            &orders[0],
            100.0,
            0.10,
            1_000,
            "fill-long-ack",
            shared_types::ExecutionFillConfidence::AdapterAck,
        ),
        fill_event(&orders[1], 102.0, 0.11, 2_000, "fill-short"),
    ];

    let rows = realized_pnl_by_group(&orders, &ledger, 0, 10_000);

    assert!(rows.is_empty());
}

#[test]
fn dry_run_adapter_ack_fill_snapshot_creates_paper_trade() {
    let orders = hedge_orders();
    let ledger = vec![
        fill_event_with_confidence(
            &orders[0],
            100.0,
            0.10,
            1_000,
            "fill-long-ack",
            shared_types::ExecutionFillConfidence::AdapterAck,
        ),
        fill_event_with_confidence(
            &orders[1],
            102.0,
            0.11,
            2_000,
            "fill-short-ack",
            shared_types::ExecutionFillConfidence::AdapterAck,
        ),
    ];

    let rows = realized_pnl_by_group(&orders, &ledger, 0, 10_000);

    assert_eq!(rows["hedge-1"].price_pnl_usd, 2.0);
    assert_eq!(
        rows["hedge-1"].evidence.fill_confidence,
        Some(shared_types::ExecutionFillConfidence::AdapterAck)
    );
}

#[test]
fn partial_orders_do_not_create_executed_trade_from_venue_fills() {
    let mut orders = hedge_orders();
    for order in &mut orders {
        order.state = LiveOrderState::PartiallyFilled;
    }
    let ledger = vec![
        fill_event(&orders[0], 100.0, 0.10, 1_000, "fill-long"),
        fill_event(&orders[1], 102.0, 0.11, 2_000, "fill-short"),
    ];

    let rows = realized_pnl_by_group(&orders, &ledger, 0, 10_000);

    assert!(rows.is_empty());
}

#[test]
fn realized_pnl_carries_orderbook_evidence_ids() {
    let orders = hedge_orders();
    let ledger = vec![
        orderbook_event(&orders[0], 900, "book-long"),
        orderbook_event(&orders[1], 950, "book-short"),
        fill_event(&orders[0], 100.0, 0.10, 1_000, "fill-long"),
        fill_event(&orders[1], 102.0, 0.11, 2_000, "fill-short"),
    ];
    let realized = realized_pnl_by_group(&orders, &ledger, 0, 10_000);
    let trades = crate::executed_from_orders(&orders, 10_000, 1);

    let trades = apply_realized_pnl(trades, &realized);

    assert_eq!(
        trades[0].evidence.orderbook_event_ids,
        ["book-long", "book-short"]
    );
    assert!(trades[0]
        .estimated_fields
        .contains(&ReviewPnlField::Slippage));
}

#[test]
fn realized_pnl_carries_per_row_ledger_event_drilldown() {
    let orders = hedge_orders();
    let ledger = vec![
        orderbook_event(&orders[0], 900, "book-long"),
        fill_event(&orders[0], 100.0, 0.10, 1_000, "fill-long"),
        fill_event(&orders[1], 102.0, 0.11, 2_000, "fill-short"),
        funding_event(&orders[0], 0.25, 3_000, "funding-long"),
        slippage_event(&orders[0], 0.25, 1_000, "slippage-long"),
        slippage_event(&orders[1], 0.50, 2_000, "slippage-short"),
    ];
    let realized = realized_pnl_by_group(&orders, &ledger, 0, 10_000);
    let trades = crate::executed_from_orders(&orders, 10_000, 1);

    let trades = apply_realized_pnl(trades, &realized);
    let details = &trades[0].evidence.ledger_events;

    assert_eq!(details.len(), 6);
    assert!(details.iter().any(|event| {
        event.event_id == "fill-long"
            && matches!(
                &event.payload,
                shared_types::ReviewLedgerPayloadEvidence::Fill {
                    average_price,
                    quantity,
                    ..
                } if *average_price == 100.0 && *quantity == 1.0
            )
    }));
    assert!(details.iter().any(|event| {
        event.event_id == "funding-long"
            && matches!(
                &event.payload,
                shared_types::ReviewLedgerPayloadEvidence::Funding { amount, .. }
                    if *amount == 0.25
            )
    }));
    assert!(details.iter().any(|event| {
        event.event_id == "book-long"
            && matches!(
                &event.payload,
                shared_types::ReviewLedgerPayloadEvidence::Orderbook {
                    depth_usd_20bps,
                    ..
                } if *depth_usd_20bps == Some(1500.0)
            )
    }));
}

#[test]
fn keeps_slippage_missing_when_reference_price_is_absent() {
    let orders = vec![
        order_with_price("hedge-1-long", OrderSide::Buy, None),
        order_with_price("hedge-1-short", OrderSide::Sell, Some(105.0)),
    ];
    let ledger = vec![
        fill_event(&orders[0], 101.0, 0.10, 1_000, "fill-long"),
        funding_event(&orders[0], 0.0, 1_500, "funding-long"),
        fill_event(&orders[1], 104.0, 0.11, 2_000, "fill-short"),
    ];

    let rows = realized_pnl_by_group(&orders, &ledger, 0, 10_000);
    let trades = apply_realized_pnl(crate::executed_from_orders(&orders, 10_000, 1), &rows);

    assert!(rows["hedge-1"].evidence.slippage_event_ids.is_empty());
    assert!(trades[0].missing_fields.contains(&ReviewPnlField::Slippage));
    assert!(trades[0].missing_fields.contains(&ReviewPnlField::Net));
}
