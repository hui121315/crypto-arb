use super::*;

#[test]
fn repeated_fills_and_order_fee_snapshots_do_not_duplicate_cash_amounts() {
    let orders = hedge_orders();
    let fill = incremental_fill_event(&orders[0], 100.0, 0.1, 1_000, "long-fill");
    let short = incremental_fill_event(&orders[1], 102.0, 0.1, 1_000, "short-fill");
    let mut fee = fill.clone();
    fee.event_id = "long-fee-snapshot".into();
    fee.event_type = ExecutionLedgerEventType::FeeSnapshot;
    fee.occurred_at_ms = 2_000;
    fee.payload = ExecutionLedgerPayload::FeeSnapshot(FeeLedgerSnapshot {
        amount: 0.12,
        currency: Some("USDT".into()),
        quality: ExecutionLedgerQuality::Actual,
    });
    let rows = realized_pnl_by_group(
        &orders,
        &[fill.clone(), fill.clone(), short.clone(), fee.clone()],
        0,
        10_000,
    );
    assert_close(rows["hedge-1"].price_pnl_usd, 2.0);
    assert_close(rows["hedge-1"].fee_usd, 0.22);
    assert_eq!(
        rows["hedge-1"].evidence.fee_event_ids,
        ["short-fill", "long-fee-snapshot"]
    );
    assert!(realized_pnl_field_quality(&rows["hedge-1"])
        .actual
        .contains(&ReviewPnlField::Fee));
    fee.occurred_at_ms = 500;
    let rows = realized_pnl_by_group(&orders, &[fill, short, fee], 0, 10_000);
    assert_close(rows["hedge-1"].fee_usd, 0.2);
}

#[test]
fn a_fee_on_one_partial_fill_does_not_prove_fees_for_the_whole_order() {
    let orders = hedge_orders();
    let mut first = incremental_fill_event(&orders[0], 100.0, 0.04, 1_000, "first");
    let mut second = incremental_fill_event(&orders[0], 100.0, 0.0, 1_100, "second");
    if let ExecutionLedgerPayload::FillSnapshot(fill) = &mut first.payload {
        fill.quantity = 0.4;
        fill.quote_value = 40.0;
    }
    if let ExecutionLedgerPayload::FillSnapshot(fill) = &mut second.payload {
        fill.quantity = 0.6;
        fill.quote_value = 60.0;
        fill.fee = None;
    }
    let rows = realized_pnl_by_group(
        &orders,
        &[
            first,
            second,
            fill_event(&orders[1], 102.0, 0.1, 1_200, "short"),
        ],
        0,
        10_000,
    );
    assert_close(rows["hedge-1"].buy_notional_usd, 100.0);
    assert!(realized_pnl_field_quality(&rows["hedge-1"])
        .missing
        .contains(&ReviewPnlField::Fee));
}

#[test]
fn deduplicates_latest_cumulative_fill_snapshot() {
    let orders = hedge_orders();
    let ledger = vec![
        fill_event(&orders[0], 98.0, 0.01, 1_000, "fill-long-a"),
        fill_event(&orders[0], 100.0, 0.10, 2_000, "fill-long-b"),
        fill_event(&orders[1], 102.0, 0.11, 3_000, "fill-short"),
    ];

    let rows = realized_pnl_by_group(&orders, &ledger, 0, 10_000);

    assert_eq!(rows["hedge-1"].price_pnl_usd, 2.0);
    assert_close(rows["hedge-1"].fee_usd, 0.21);
}

#[test]
fn sums_incremental_fill_events_for_same_order() {
    let orders = hedge_orders();
    let ledger = vec![
        incremental_fill_event(&orders[0], 40.0, 0.04, 1_000, "fill-long-a"),
        incremental_fill_event(&orders[0], 60.0, 0.06, 2_000, "fill-long-b"),
        incremental_fill_event(&orders[1], 103.0, 0.10, 3_000, "fill-short"),
    ];

    let rows = realized_pnl_by_group(&orders, &ledger, 0, 10_000);

    assert_eq!(rows["hedge-1"].buy_notional_usd, 100.0);
    assert_eq!(rows["hedge-1"].sell_notional_usd, 103.0);
    assert_eq!(rows["hedge-1"].price_pnl_usd, 3.0);
    assert_close(rows["hedge-1"].fee_usd, 0.20);
    assert_eq!(
        rows["hedge-1"].evidence.fill_event_ids,
        ["fill-long-a", "fill-long-b", "fill-short"]
    );
}

#[test]
fn incremental_fills_override_cumulative_snapshot_for_same_order() {
    let orders = hedge_orders();
    let ledger = vec![
        fill_event(&orders[0], 100.0, 0.50, 1_000, "query-long-cumulative"),
        incremental_fill_event(&orders[0], 40.0, 0.04, 2_000, "fill-long-a"),
        incremental_fill_event(&orders[0], 60.0, 0.06, 3_000, "fill-long-b"),
        fill_event(&orders[1], 103.0, 0.10, 4_000, "query-short-cumulative"),
    ];

    let rows = realized_pnl_by_group(&orders, &ledger, 0, 10_000);

    assert_eq!(rows["hedge-1"].buy_notional_usd, 100.0);
    assert_eq!(rows["hedge-1"].sell_notional_usd, 103.0);
    assert_close(rows["hedge-1"].fee_usd, 0.20);
    assert_eq!(
        rows["hedge-1"].evidence.fill_event_ids,
        ["fill-long-a", "fill-long-b", "query-short-cumulative"]
    );
}

#[test]
fn slippage_follows_the_selected_fill_evidence() {
    let orders = hedge_orders();
    let fallback_fill = fill_event(&orders[0], 101.0, 0.10, 1_000, "adapter-long");
    let authoritative_fill = incremental_fill_event(&orders[0], 101.0, 0.10, 2_000, "private-long");
    let short_fill = incremental_fill_event(&orders[1], 99.0, 0.10, 2_100, "private-short");
    let mut fallback_slippage = slippage_event(&orders[0], 1.0, 1_000, "slippage:adapter-long");
    fallback_slippage.source = OrderUpdateSource::AdapterAck;
    let ledger = vec![
        fallback_fill,
        authoritative_fill,
        short_fill,
        fallback_slippage,
        slippage_event(&orders[0], 1.0, 2_000, "slippage:private-long"),
        slippage_event(&orders[1], 1.0, 2_100, "slippage:private-short"),
    ];

    let rows = realized_pnl_by_group(&orders, &ledger, 0, 10_000);
    let evidence = &rows["hedge-1"].evidence;

    assert_eq!(rows["hedge-1"].slippage_usd, 2.0);
    assert_eq!(
        evidence.slippage_event_ids,
        ["slippage:private-long", "slippage:private-short"]
    );
}
