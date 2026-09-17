use super::*;
use fixtures::*;
use shared_types::OrderSide;

mod fixtures;

#[test]
fn today_pnl_uses_ledger_fill_snapshots_only() {
    let rows = vec![
        accepted_order("hedge-1-long", OrderSide::Buy, 86_400_000 + 1_000),
        accepted_order("hedge-1-short", OrderSide::Sell, 86_400_000 + 1_500),
        accepted_order("hedge-2-long", OrderSide::Buy, 86_400_000 + 2_000),
    ];
    let ledger = vec![
        fill_event(&rows[0], 100.0, 86_400_000 + 3_000),
        fill_event(&rows[1], 102.0, 86_400_000 + 3_500),
    ];

    let pnl = today_from_ledger(&rows, &ledger, &[], 86_400_000 + 10_000);

    assert_eq!(pnl.realized_pnl_usd, 2.0);
    assert_eq!(pnl.funding_usd, 0.0);
    assert_eq!(pnl.fee_rebate_usd, 0.0);
    assert_eq!(
        pnl.evidence.quality,
        shared_types::ExecutionLedgerQuality::Missing
    );
    assert!(pnl
        .evidence
        .missing_fields
        .contains(&shared_types::ReviewPnlField::Fee));
    assert!(pnl
        .evidence
        .missing_fields
        .contains(&shared_types::ReviewPnlField::Funding));
    assert!(pnl
        .evidence
        .estimated_fields
        .contains(&shared_types::ReviewPnlField::Slippage));
}

#[test]
fn daily_history_records_pnl_on_completion_day() {
    let rows = vec![
        accepted_order("hedge-1-long", OrderSide::Buy, 1_000),
        accepted_order("hedge-1-short", OrderSide::Sell, DAY_MS + 2_000),
    ];
    let ledger = vec![
        fill_event(&rows[0], 100.0, 1_000),
        fill_event(&rows[1], 80.0, DAY_MS + 2_000),
    ];

    let history = daily_history_from_ledger(&rows, &ledger, &[], 3 * DAY_MS + 1, 3);

    assert_eq!(history, vec![(0, 0.0), (DAY_MS, -20.0), (2 * DAY_MS, 0.0)]);
}

#[test]
fn snapshot_does_not_claim_open_hedges_as_realized_pnl() {
    let rows = vec![
        accepted_order("hedge-1-long", OrderSide::Buy, DAY_MS + 1_000),
        accepted_order("hedge-1-short", OrderSide::Sell, DAY_MS + 2_000),
        accepted_order("hedge-2-long", OrderSide::Buy, 2 * DAY_MS + 1_000),
        accepted_order("hedge-2-short", OrderSide::Sell, 2 * DAY_MS + 2_000),
    ];
    let ledger = vec![
        fill_event(&rows[0], 100.0, DAY_MS + 1_000),
        fill_event(&rows[1], 101.0, DAY_MS + 2_000),
        fill_event(&rows[2], 80.0, 2 * DAY_MS + 1_000),
        fill_event(&rows[3], 82.0, 2 * DAY_MS + 2_000),
    ];

    let snapshot = snapshot_from_ledger(&rows, &ledger, &[], 2 * DAY_MS + 10_000);

    assert_eq!(snapshot.today.realized_pnl_usd, 0.0);
    assert_eq!(
        snapshot.today.evidence.quality,
        shared_types::ExecutionLedgerQuality::Actual
    );
    assert!(snapshot.today.evidence.problem.is_none());
    assert_eq!(
        snapshot.history[HISTORY_LOOKBACK_DAYS as usize - 1],
        (DAY_MS, 0.0)
    );
}

#[test]
fn snapshot_attributes_realized_pnl_to_terminal_close_day() {
    let rows = vec![
        accepted_order("hedge-1-long", OrderSide::Buy, 1_000),
        accepted_order("hedge-1-short", OrderSide::Sell, 2_000),
    ];
    let ledger = vec![
        linked_event(fill_event(&rows[0], 100.0, 1_000), "run-1", "ticket-1"),
        linked_event(fill_event(&rows[1], 102.0, 2_000), "run-1", "ticket-1"),
    ];
    let close_runs = [close_run_with_cost("close-1", "run-1", "ticket-1", 0.0)];

    let snapshot = snapshot_from_ledger(&rows, &ledger, &close_runs, DAY_MS + 10_000);

    assert_eq!(snapshot.today.realized_pnl_usd, 2.0);
    assert_eq!(snapshot.today.evidence.realized_group_count, 1);
    assert_eq!(snapshot.today.evidence.close_run_count, 1);
    assert!(snapshot.history.iter().all(|(_, pnl)| *pnl == 0.0));
}

#[test]
fn sql_realized_window_order_snapshots_drive_pnl() {
    let rows = vec![
        accepted_order("sql-hedge-long", OrderSide::Buy, DAY_MS + 1_000),
        accepted_order("sql-hedge-short", OrderSide::Sell, DAY_MS + 2_000),
    ];
    let window = trading::SqlRealizedWindow {
        events: vec![
            fill_event(&rows[0], 100.0, DAY_MS + 1_000),
            fill_event(&rows[1], 103.0, DAY_MS + 2_000),
        ],
        order_snapshots: rows,
        close_runs: Vec::new(),
    };

    let realized = realized_ledger_from_sql_window(window);
    let pnl = today_from_ledger(
        &realized.orders,
        &realized.ledger,
        &realized.close_runs,
        DAY_MS + 10_000,
    );

    assert_eq!(realized.orders.len(), 2);
    assert_eq!(realized.source, PNL_SOURCE_SQL);
    assert_eq!(pnl.realized_pnl_usd, 3.0);
}

#[test]
fn sql_realized_window_close_run_costs_reduce_today_and_history_pnl() {
    let rows = vec![
        accepted_order("sql-hedge-long", OrderSide::Buy, DAY_MS + 1_000),
        accepted_order("sql-hedge-short", OrderSide::Sell, DAY_MS + 2_000),
    ];
    let window = trading::SqlRealizedWindow {
        events: vec![
            linked_event(
                fill_event(&rows[0], 100.0, DAY_MS + 1_000),
                "run-1",
                "ticket-1",
            ),
            linked_event(
                fill_event(&rows[1], 103.0, DAY_MS + 2_000),
                "run-1",
                "ticket-1",
            ),
        ],
        order_snapshots: rows,
        close_runs: vec![close_run_with_cost("close-1", "run-1", "ticket-1", 0.4)],
    };

    let realized = realized_ledger_from_sql_window(window);
    let today = today_from_ledger(
        &realized.orders,
        &realized.ledger,
        &realized.close_runs,
        DAY_MS + 10_000,
    );
    let history = daily_history_from_ledger(
        &realized.orders,
        &realized.ledger,
        &realized.close_runs,
        2 * DAY_MS + 1,
        2,
    );

    assert!((today.realized_pnl_usd - 2.6).abs() < 1e-9);
    assert_eq!(today.evidence.close_run_count, 1);
    assert_eq!(today.evidence.unwind_run_count, 1);
    assert_eq!(history, vec![(0, 0.0), (DAY_MS, 2.6)]);
}

#[test]
fn empty_realized_window_is_an_actual_zero_with_source() {
    let pnl = today_from_ledger(&[], &[], &[], DAY_MS + 10_000);

    assert_eq!(pnl.realized_pnl_usd, 0.0);
    assert_eq!(
        pnl.evidence.quality,
        shared_types::ExecutionLedgerQuality::Actual
    );
    assert_eq!(pnl.evidence.actual_fields.len(), PNL_FIELDS.len());
    assert_eq!(pnl.evidence.source, PNL_SOURCE_LEDGER);
    assert!(pnl.evidence.problem.is_none());
}

#[test]
fn today_pnl_subtracts_realized_fee_cost() {
    let rows = vec![
        accepted_order("hedge-1-long", OrderSide::Buy, DAY_MS + 1_000),
        accepted_order("hedge-1-short", OrderSide::Sell, DAY_MS + 1_500),
    ];
    let ledger = vec![
        fill_event_with_fee(&rows[0], 100.0, 0.10, DAY_MS + 2_000, "fill-long"),
        fill_event_with_fee(&rows[1], 102.0, 0.15, DAY_MS + 2_500, "fill-short"),
    ];

    let pnl = today_from_ledger(&rows, &ledger, &[], DAY_MS + 10_000);

    assert!((pnl.realized_pnl_usd - 1.75).abs() < 1e-9);
    assert_eq!(pnl.funding_usd, 0.0);
    assert!((pnl.fee_rebate_usd + 0.25).abs() < 1e-9);
}

#[test]
fn today_pnl_includes_realized_funding_payment() {
    let rows = vec![
        accepted_order("hedge-1-long", OrderSide::Buy, DAY_MS + 1_000),
        accepted_order("hedge-1-short", OrderSide::Sell, DAY_MS + 1_500),
    ];
    let ledger = vec![
        fill_event(&rows[0], 100.0, DAY_MS + 2_000),
        funding_event(&rows[0], 0.25, DAY_MS + 2_250, "funding-long"),
        fill_event(&rows[1], 102.0, DAY_MS + 2_500),
    ];

    let pnl = today_from_ledger(&rows, &ledger, &[], DAY_MS + 10_000);

    assert!((pnl.realized_pnl_usd - 2.25).abs() < 1e-9);
    assert_eq!(pnl.funding_usd, 0.25);
    assert_eq!(pnl.fee_rebate_usd, 0.0);
}

#[test]
fn ignores_incomplete_or_fillless_groups() {
    let rows = vec![
        accepted_order("hedge-1-long", OrderSide::Buy, 1_000),
        accepted_order("hedge-1-short", OrderSide::Sell, 2_000),
        accepted_order("hedge-2-long", OrderSide::Buy, 3_000),
    ];
    let ledger = vec![fill_event(&rows[0], 100.0, 1_000)];

    assert!(realized_by_day(&rows, &ledger, &[], 0, DAY_MS).is_empty());
}

#[test]
fn deduplicates_cumulative_fill_snapshots_by_order() {
    let rows = vec![
        accepted_order("hedge-1-long", OrderSide::Buy, 1_000),
        accepted_order("hedge-1-short", OrderSide::Sell, 2_000),
    ];
    let ledger = vec![
        fill_event(&rows[0], 100.0, 1_000),
        fill_event_with_id(&rows[0], 100.0, 1_500, "fill-hedge-1-long-rest"),
        fill_event(&rows[1], 102.0, 2_000),
    ];

    let values = realized_by_day(&rows, &ledger, &[], 0, DAY_MS);

    assert_eq!(values.get(&0), Some(&2.0));
}
