use super::*;
use shared_types::{CloseRunCostComponent, CloseRunCostLedgerEvent};

fn opening(orders: &[OrderRecord], run: &str, ticket: &str) -> Vec<ExecutionLedgerEvent> {
    orders
        .iter()
        .enumerate()
        .map(|(index, order)| {
            linked_event(
                fill_event(
                    order,
                    100.0 + index as f64 * 2.0,
                    0.1,
                    1_000 + index as i64 * 100,
                    &format!("fill-{}", order.intent.id),
                ),
                run,
                ticket,
            )
        })
        .collect()
}

fn close(id: &str, run: &str, ticket: &str, quantity: f64, fee: f64) -> CloseRun {
    let mut close = successful_pair_close_run(
        id,
        run,
        ticket,
        99.0,
        103.0,
        CloseRunCostReconciliation::default(),
    );
    for (index, leg) in close.legs.iter_mut().enumerate() {
        leg.quantity = quantity;
        leg.pair_evidence.as_mut().unwrap().partner_venue = "mock".into();
        let order = leg.order.as_mut().unwrap();
        order.intent.id = format!("{id}-{index}");
        order.intent.client_order_id = format!("{id}-{index}-client");
        order.identity = VenueOrderIdentity::from_intent(&order.intent);
        order.intent.quantity = quantity;
        order.filled_quantity = Some(quantity);
        order.filled_fee = Some(fee);
        leg.cost_events = vec![cost(
            &format!("fee-{id}-{index}"),
            CloseRunCostComponent::Fee,
            fee,
        )];
    }
    close.cost_reconciliation = Some(CloseRunCostReconciliation {
        close_fee_usd: Some(fee * 2.0),
        close_fee_event_ids: close
            .legs
            .iter()
            .map(|leg| leg.cost_events[0].event_id.clone())
            .collect(),
        ..CloseRunCostReconciliation::default()
    });
    close
}

fn cost(id: &str, component: CloseRunCostComponent, amount_usd: f64) -> CloseRunCostLedgerEvent {
    CloseRunCostLedgerEvent {
        event_id: id.into(),
        component,
        amount_usd,
        source: OrderUpdateSource::PrivateWs,
        quality: ExecutionLedgerQuality::Actual,
        occurred_at_ms: 4_000,
        captured_at_ms: 4_000,
    }
}

#[test]
fn realized_window_uses_completed_close_time_not_opening_time() {
    let orders = hedge_orders();
    let ledger = opening(&orders, "run-1", "ticket-1");
    let run = close("close", "run-1", "ticket-1", 1.0, 0.1);
    let rows =
        realized_pnl_by_group_with_close_runs(&orders, &ledger, &[run.clone()], 4_000, 5_000);
    assert_eq!(
        rows.len(),
        1,
        "opening fills before the window must not hide today's close"
    );
    assert_eq!(rows["hedge-1"].realized_at_ms, 4_000);
    assert_close(rows["hedge-1"].net_pnl_usd, -2.4);
    assert!(
        realized_pnl_by_group_with_close_runs(&orders, &ledger, &[run.clone()], 4_001, 5_000)
            .is_empty()
    );
    assert!(
        realized_pnl_by_group_with_close_runs(&orders, &ledger, &[run.clone()], 2_000, 4_000)
            .is_empty()
    );
    assert!(realized_pnl_by_group_with_close_runs(&orders, &ledger, &[], 4_000, 5_000).is_empty());
    assert!(
        realized_pnl_by_group_with_close_runs(&orders, &ledger, &[run], 5_000, 4_000).is_empty()
    );
}

#[test]
fn partial_closes_accumulate_only_when_the_original_quantities_are_fully_closed() {
    let orders = hedge_orders();
    let ledger = opening(&orders, "run-1", "ticket-1");
    let first = close("first", "run-1", "ticket-1", 0.4, 0.04);
    let row = realized_pnl_by_group_with_close_runs(&orders, &ledger, &[first.clone()], 0, 10_000);
    assert_eq!(row["hedge-1"].closed_at_ms, None);
    assert!(realized_pnl_field_quality(&row["hedge-1"])
        .missing
        .contains(&ReviewPnlField::Gross));
    let second = close("second", "run-1", "ticket-1", 0.6, 0.06);
    let rows = realized_pnl_by_group_with_close_runs(
        &orders,
        &ledger,
        &[first.clone(), second.clone(), first, second],
        0,
        10_000,
    );
    assert_eq!(rows["hedge-1"].closed_at_ms, Some(4_000));
    assert_close(rows["hedge-1"].price_pnl_usd, -2.0);
    assert_close(rows["hedge-1"].fee_usd, 0.4);
    assert_close(rows["hedge-1"].net_pnl_usd, -2.4);
}

fn partially_cancelled_close() -> CloseRun {
    let mut run = close("partial", "run-1", "ticket-1", 1.0, 0.04);
    run.status = CloseRunStatus::UnwindRequired;
    run.finality_problem = Some(shared_types::ApiProblem::new("CLOSE_RUN_FAILED", "partial close"));
    for leg in &mut run.legs {
        leg.status = CloseLegStatus::Cancelled;
        let time = leg.confirmed_filled_at_ms.take().unwrap();
        let order = leg.order.as_mut().unwrap();
        order.state = LiveOrderState::Cancelled;
        order.filled_quantity = Some(0.4);
        order.updated_at_ms = 8_000;
        leg.ledger_fills = Some(shared_types::CloseFillLedger {
            totals: shared_types::ExecutionLedgerFillTotals {
                quantity: 0.4, notional: 0.4 * order.filled_price.unwrap(), fee: Some(0.04),
            },
            event_ids: vec![format!("fill-{}", order.intent.id)],
            last_fill_at_ms: Some(time),
        });
    }
    run
}

#[test]
fn cancelled_partial_fills_and_retries_realize_once_at_the_last_fill_time() {
    let orders = hedge_orders();
    let ledger = opening(&orders, "run-1", "ticket-1");
    let first = partially_cancelled_close();
    let second = close("retry", "run-1", "ticket-1", 0.6, 0.06);
    let rows = realized_pnl_by_group_with_close_runs(&orders, &ledger,
        &[first.clone(), second.clone(), first, second], 0, 10_000);
    let row = &rows["hedge-1"];
    assert_eq!(row.closed_at_ms, Some(4_000), "later cancellation time is not a new fill");
    assert_close(row.price_pnl_usd, -2.0);
    assert_close(row.fee_usd, 0.4);
    assert_close(row.net_pnl_usd, -2.4);
    assert!(realized_pnl_field_quality(row).estimated.contains(&ReviewPnlField::Gross));
}

#[test]
fn partial_close_needs_remote_terminal_state_complete_fills_and_matching_identity() {
    let orders = hedge_orders();
    let ledger = opening(&orders, "run-1", "ticket-1");
    let second = close("retry", "run-1", "ticket-1", 0.6, 0.06);
    for fault in ["active", "cancel_requested", "ack", "missing_time", "missing_fill", "invalid_notional", "wrong_venue", "overfill", "missing_quantity"] {
        let mut first = partially_cancelled_close();
        let leg = &mut first.legs[0];
        let order = leg.order.as_mut().unwrap();
        match fault {
            "active" => { leg.status = CloseLegStatus::PartiallyFilled; order.state = LiveOrderState::PartiallyFilled; }
            "cancel_requested" => { leg.status = CloseLegStatus::CancelRequested; order.state = LiveOrderState::CancelRequested; }
            "ack" => { leg.finality_source = Some(OrderUpdateSource::AdapterAck); order.last_update_source = OrderUpdateSource::AdapterAck; }
            "missing_time" => leg.ledger_fills.as_mut().unwrap().last_fill_at_ms = None,
            "missing_fill" => leg.ledger_fills.as_mut().unwrap().totals.quantity = 0.2,
            "invalid_notional" => leg.ledger_fills.as_mut().unwrap().totals.notional = f64::NAN,
            "wrong_venue" => order.intent.exchange = "other".into(),
            "overfill" => order.filled_quantity = Some(2.0),
            "missing_quantity" => order.filled_quantity = None,
            _ => unreachable!(),
        }
        let rows = realized_pnl_by_group_with_close_runs(&orders, &ledger, &[first, second.clone()], 0, 10_000);
        assert_eq!(rows["hedge-1"].closed_at_ms, None, "{fault}");
    }
}

#[test]
fn close_success_does_not_override_quantity_identity_mode_or_time_mismatches() {
    let orders = hedge_orders();
    let ledger = opening(&orders, "run-1", "ticket-1");
    for fault in [
        "partial_fill",
        "overfill",
        "wrong_market",
        "wrong_pair",
        "wrong_mode",
        "not_reduce",
        "early",
        "future",
        "ack",
    ] {
        let mut run = close("close", "run-1", "ticket-1", 1.0, 0.1);
        let leg = &mut run.legs[0];
        let order = leg.order.as_mut().unwrap();
        match fault {
            "partial_fill" => order.filled_quantity = Some(0.4),
            "overfill" => order.filled_quantity = Some(1.5),
            "wrong_market" => {
                leg.symbol = "ETH".into();
                order.intent.symbol = "ETH".into();
            }
            "wrong_pair" => leg.pair_evidence.as_mut().unwrap().partner_symbol = "ETH".into(),
            "wrong_mode" => order.intent.mode = ExecutionMode::Live,
            "not_reduce" => order.intent.reduce_only = false,
            "early" => leg.confirmed_filled_at_ms = Some(100),
            "future" => leg.confirmed_filled_at_ms = Some(11_000),
            "ack" => {
                order.intent.mode = ExecutionMode::Live;
                order.last_update_source = OrderUpdateSource::AdapterAck;
                leg.finality_source = Some(OrderUpdateSource::AdapterAck);
            }
            _ => unreachable!(),
        }
        let rows = realized_pnl_by_group_with_close_runs(&orders, &ledger, &[run], 0, 10_000);
        assert_eq!(rows["hedge-1"].closed_at_ms, None, "{fault}");
    }
}

#[test]
fn separate_single_leg_closes_can_finish_the_same_pair() {
    let orders = hedge_orders();
    let ledger = opening(&orders, "run-1", "ticket-1");
    let mut long = close("long", "run-1", "ticket-1", 1.0, 0.1);
    let mut short = close("short", "run-1", "ticket-1", 1.0, 0.1);
    long.legs.truncate(1);
    short.legs.remove(0);
    for run in [&mut long, &mut short] {
        run.scope = CloseRunScope::Single;
        run.expected_leg_count = 1;
        run.submitted_order_count = 1;
        run.cost_reconciliation.as_mut().unwrap().close_fee_usd = Some(0.1);
        run.cost_reconciliation
            .as_mut()
            .unwrap()
            .close_fee_event_ids = vec![run.legs[0].cost_events[0].event_id.clone()];
    }
    let rows = realized_pnl_by_group_with_close_runs(&orders, &ledger, &[long, short], 0, 10_000);
    assert_eq!(rows["hedge-1"].closed_at_ms, Some(4_000));
    assert_close(rows["hedge-1"].price_pnl_usd, -2.0);
    assert_close(rows["hedge-1"].fee_usd, 0.4);
}

#[test]
fn proven_zero_fill_rejections_do_not_hide_a_later_successful_close() {
    let orders = hedge_orders();
    let ledger = opening(&orders, "run-1", "ticket-1");
    let mut rejected = close("rejected", "run-1", "ticket-1", 1.0, 0.0);
    rejected.status = CloseRunStatus::Failed;
    for leg in &mut rejected.legs {
        leg.status = CloseLegStatus::Rejected;
        leg.confirmed_filled_at_ms = None;
        let order = leg.order.as_mut().unwrap();
        order.state = LiveOrderState::Rejected;
        order.filled_quantity = Some(0.0);
    }
    let completed = close("complete", "run-1", "ticket-1", 1.0, 0.1);
    let rows = realized_pnl_by_group_with_close_runs(
        &orders,
        &ledger,
        &[rejected.clone(), completed.clone()],
        0,
        10_000,
    );
    assert_eq!(rows["hedge-1"].closed_at_ms, Some(4_000));
    for leg in &mut rejected.legs {
        leg.order.as_mut().unwrap().filled_quantity = None;
    }
    let rows =
        realized_pnl_by_group_with_close_runs(&orders, &ledger, &[rejected, completed], 0, 10_000);
    assert_eq!(rows["hedge-1"].closed_at_ms, None);
}

#[test]
fn position_labels_can_match_native_open_symbols_without_matching_another_quote_market() {
    let mut orders = hedge_orders();
    orders[0].intent.symbol = "BTCUSDT".into();
    orders[1].intent.symbol = "BTC-USDT-SWAP".into();
    let ledger = opening(&orders, "run-1", "ticket-1");
    let run = close("close", "run-1", "ticket-1", 1.0, 0.1);
    let project = |run: &CloseRun| {
        realized_pnl_by_group_with_close_symbol_key(
            &orders,
            &ledger,
            std::slice::from_ref(run),
            0,
            10_000,
            |symbol| match symbol {
                "BTCUSDT" | "BTC-USDT-SWAP" => "BTC".into(),
                other => other.to_owned(),
            },
        )
    };
    assert_eq!(project(&run)["hedge-1"].closed_at_ms, Some(4_000));
    let mut wrong_quote = run.clone();
    wrong_quote.legs[0].symbol = "BTCUSDC".into();
    wrong_quote.legs[0].order.as_mut().unwrap().intent.symbol = "BTCUSDC".into();
    assert_eq!(project(&wrong_quote)["hedge-1"].closed_at_ms, None);
}

#[test]
fn close_all_attributes_each_pairs_own_fills_and_order_fee_once() {
    let mut orders = hedge_orders();
    orders.extend([
        order("hedge-2-long", OrderSide::Buy),
        order("hedge-2-short", OrderSide::Sell),
    ]);
    let mut ledger = opening(&orders[..2], "run-1", "ticket-1");
    ledger.extend(opening(&orders[2..], "run-2", "ticket-2"));
    let mut run = close("first", "run-1", "ticket-1", 1.0, 0.2);
    let other = close("second", "run-2", "ticket-2", 1.0, 0.5);
    // Two cumulative fee updates describe the same order, not two cash charges.
    run.legs[0]
        .cost_events
        .insert(0, cost("earlier-fee", CloseRunCostComponent::Fee, 0.1));
    run.legs.extend(other.legs);
    run.scope = CloseRunScope::All;
    run.expected_leg_count = 4;
    run.submitted_order_count = 4;
    run.cost_reconciliation = Some(CloseRunCostReconciliation {
        close_fee_usd: Some(1.4),
        close_fee_event_ids: run
            .legs
            .iter()
            .flat_map(|leg| leg.cost_events.iter().map(|event| event.event_id.clone()))
            .collect(),
        ..CloseRunCostReconciliation::default()
    });
    let rows = realized_pnl_by_group_with_close_runs(&orders, &ledger, &[run.clone()], 0, 10_000);
    assert_close(rows["hedge-1"].fee_usd, 0.6);
    assert_close(rows["hedge-2"].fee_usd, 1.2);
    for (key, fee) in [("hedge-1", 0.4), ("hedge-2", 1.0)] {
        assert_eq!(rows[key].closed_at_ms, Some(4_000));
        assert_close(rows[key].price_pnl_usd, -2.0);
        assert_close(
            rows[key].evidence.close_run_evidence[0]
                .cost_reconciliation
                .as_ref()
                .unwrap()
                .close_fee_usd
                .unwrap(),
            fee,
        );
    }
    run.cost_reconciliation.as_mut().unwrap().funding_usd = Some(0.3);
    run.cost_reconciliation.as_mut().unwrap().funding_event_ids = vec!["all-funding".into()];
    let rows = realized_pnl_by_group_with_close_runs(&orders, &ledger, &[run], 0, 10_000);
    for row in rows.values() {
        assert_close(row.funding_usd, 0.0);
        assert!(realized_pnl_field_quality(row)
            .missing
            .contains(&ReviewPnlField::Net));
        assert!(row.evidence.close_run_evidence[0]
            .cost_reconciliation
            .as_ref()
            .unwrap()
            .missing_fields
            .iter()
            .any(|field| field == "unallocated_close_cost"));
    }
}
