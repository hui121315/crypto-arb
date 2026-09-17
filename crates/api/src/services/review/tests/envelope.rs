use super::super::*;
use super::*;
use serde_json::json;

#[test]
fn executed_requires_fill_ledger_snapshot() {
    let long = order("hedge-1-long", OrderSide::Buy, 100.0);
    let short = order("hedge-1-short", OrderSide::Sell, 101.0);
    let orders = vec![long.clone(), short.clone()];
    let no_ledger = executed(&orders, &[], 1);

    assert!(no_ledger.is_empty());

    let ledger = vec![
        fill_event(&long, HedgeLegRole::Long),
        fill_event(&short, HedgeLegRole::Short),
    ];
    let trades = executed(&orders, &ledger, 1);

    assert_eq!(trades.len(), 1);
    assert_eq!(trades[0].id, "hedge-1");
    assert_eq!(trades[0].gross_pnl_usd, 1.0);
    assert_eq!(trades[0].fee_usd, 0.02);
    assert_eq!(trades[0].net_pnl_usd, 0.98);
    assert_eq!(trades[0].closed_at_ms, None);
}

#[test]
fn executed_envelope_reports_partial_evidence() {
    let long = order("hedge-1-long", OrderSide::Buy, 100.0);
    let short = order("hedge-1-short", OrderSide::Sell, 101.0);
    let orders = vec![long.clone(), short.clone()];
    let ledger = vec![
        fill_event(&long, HedgeLegRole::Long),
        fill_event(&short, HedgeLegRole::Short),
    ];

    let envelope = executed_envelope(&orders, &ledger, 1, &ReviewPageQuery::default());

    assert_eq!(envelope.row_count, 1);
    assert_eq!(
        envelope.ledger_status,
        Some(ReviewLedgerStatus::PartialEvidence)
    );
    assert_eq!(envelope.status, ListStatus::Degraded);
    assert!(envelope.missing_fields.contains(&ReviewPnlField::Funding));
    assert!(envelope.missing_fields.contains(&ReviewPnlField::Gross));
    assert!(!envelope.missing_fields.contains(&ReviewPnlField::Slippage));
    assert!(envelope.rows[0]
        .estimated_fields
        .contains(&ReviewPnlField::Slippage));
    assert_eq!(
        review_ledger_problem_detail(&envelope, "ledgerStatus"),
        Some(json!("partial_evidence"))
    );
    assert_eq!(
        review_ledger_problem_detail(&envelope, "windowDays"),
        Some(json!(1))
    );
    assert_eq!(
        review_ledger_problem_detail(&envelope, "ledgerEventCount"),
        Some(json!(2))
    );
    assert_eq!(
        review_ledger_problem_detail(&envelope, "sampleCount"),
        Some(json!(1))
    );
    assert_eq!(
        review_ledger_problem_detail(&envelope, "excludedIncompleteCount"),
        Some(json!(0))
    );
    assert_review_storage_health(
        &envelope,
        "storage:review_execution_ledger",
        envelope.row_count,
    );
}

#[test]
fn executed_envelope_reports_empty_ledger() {
    let envelope = executed_envelope(&[], &[], 1, &ReviewPageQuery::default());

    assert!(envelope.rows.is_empty());
    assert_eq!(
        envelope.ledger_status,
        Some(ReviewLedgerStatus::NoLedgerEvents)
    );
    assert_eq!(envelope.status, ListStatus::Fresh);
    assert!(envelope.problems.is_empty());
    assert_eq!(
        review_ledger_problem_detail(&envelope, "ledgerStatus"),
        None
    );
}

#[test]
fn executed_envelope_returns_first_page_only() {
    let orders = Vec::new();
    let ledger = Vec::new();
    let envelope = executed_envelope(&orders, &ledger, 1, &ReviewPageQuery::new(Some(1), None));

    assert_eq!(envelope.page.limit, 1);
    assert_eq!(envelope.page.total_rows, 0);
    assert_eq!(envelope.page.returned_count, 0);
}

#[test]
fn executed_envelope_pages_materialized_rows_with_exact_total() {
    let now_ms = common::time::now_ms();
    let long_a = order_at("hedge-a-long", OrderSide::Buy, 100.0, now_ms - 1_000);
    let short_a = order_at("hedge-a-short", OrderSide::Sell, 101.0, now_ms - 1_000);
    let long_b = order_at("hedge-b-long", OrderSide::Buy, 90.0, now_ms - 2_000);
    let short_b = order_at("hedge-b-short", OrderSide::Sell, 91.0, now_ms - 2_000);
    let orders = vec![
        long_a.clone(),
        short_a.clone(),
        long_b.clone(),
        short_b.clone(),
    ];
    let ledger = vec![
        fill_event(&long_a, HedgeLegRole::Long),
        fill_event(&short_a, HedgeLegRole::Short),
        fill_event(&long_b, HedgeLegRole::Long),
        fill_event(&short_b, HedgeLegRole::Short),
    ];

    let first = executed_envelope(&orders, &ledger, 1, &ReviewPageQuery::new(Some(1), None));
    let envelope = executed_envelope(
        &orders,
        &ledger,
        1,
        &ReviewPageQuery::new(Some(1), first.page.next_cursor),
    );

    assert_eq!(envelope.rows.len(), 1);
    assert_eq!(envelope.rows[0].id, "hedge-b");
    let details = &envelope.rows[0].evidence.ledger_events;
    assert_eq!(details.len(), 2);
    assert!(details.iter().any(|event| {
        event.event_id == "fill-hedge-b-long"
            && event.source == OrderUpdateSource::PrivateWs
            && event.order.identity.internal_order_id == "hedge-b-long"
            && matches!(
                &event.payload,
                shared_types::ReviewLedgerPayloadEvidence::Fill {
                    fee: Some(fee),
                    ..
                } if fee.amount == 0.01
            )
    }));
    assert_eq!(envelope.page.total_rows, 2);
    assert_eq!(envelope.page.returned_count, 1);
    assert_eq!(envelope.page.next_cursor, None);
    assert_eq!(
        envelope.ledger_status,
        Some(ReviewLedgerStatus::PartialEvidence)
    );
    assert!(envelope.missing_fields.contains(&ReviewPnlField::Funding));
    assert!(!envelope.missing_fields.contains(&ReviewPnlField::Slippage));
    assert_eq!(
        review_ledger_problem_detail(&envelope, "ledgerStatus"),
        Some(json!("partial_evidence"))
    );
    assert_eq!(
        review_ledger_problem_detail(&envelope, "rowCount"),
        Some(json!(2))
    );
    assert_eq!(
        review_ledger_problem_detail(&envelope, "sampleCount"),
        Some(json!(2))
    );
}

#[test]
fn strategy_performance_explains_rows_when_net_evidence_is_incomplete() {
    let long_a = order("hedge-a-long", OrderSide::Buy, 100.0);
    let short_a = order("hedge-a-short", OrderSide::Sell, 101.0);
    let long_b = order("hedge-b-long", OrderSide::Buy, 200.0);
    let short_b = order("hedge-b-short", OrderSide::Sell, 210.0);
    let orders = vec![long_a.clone(), short_a.clone(), long_b, short_b];
    let ledger = vec![
        fill_event(&long_a, HedgeLegRole::Long),
        fill_event(&short_a, HedgeLegRole::Short),
    ];

    let envelope = strategy_performance_envelope_at(&orders, &ledger, &[], common::time::now_ms());

    assert_eq!(envelope.rows.len(), 1);
    assert_eq!(envelope.rows[0].trades_30d, 0);
    assert_eq!(envelope.rows[0].total_trades_30d, 1);
    assert_eq!(envelope.rows[0].skipped_trades_30d, 1);
    assert_eq!(
        envelope.rows[0].sample_status,
        StrategyPerformanceSampleStatus::NoCompleteSample
    );
    assert_eq!(envelope.status, ListStatus::Degraded);
    assert_eq!(
        envelope.ledger_status,
        Some(ReviewLedgerStatus::PartialEvidence)
    );
    assert!(envelope.missing_fields.contains(&ReviewPnlField::Net));
    assert!(envelope
        .problems
        .iter()
        .any(|problem| problem.code == codes::REVIEW_LEDGER_INCOMPLETE));
    assert_eq!(
        review_ledger_problem_detail(&envelope, "windowDays"),
        Some(json!(30))
    );
    assert_eq!(
        review_ledger_problem_detail(&envelope, "ledgerEventCount"),
        Some(json!(2))
    );
    assert_eq!(
        review_ledger_problem_detail(&envelope, "rowCount"),
        Some(json!(1))
    );
    assert_eq!(
        review_ledger_problem_detail(&envelope, "sampleCount"),
        Some(json!(0))
    );
    assert_eq!(
        review_ledger_problem_detail(&envelope, "excludedIncompleteCount"),
        Some(json!(1))
    );
    assert_review_storage_health(&envelope, "storage:review_execution_ledger", 1);
}

#[test]
fn runtime_snapshot_projects_execution_and_strategy_from_one_generation() {
    let now_ms = common::time::now_ms();
    let long = order_at("runtime-hedge-long", OrderSide::Buy, 100.0, now_ms - 1_000);
    let short = order_at(
        "runtime-hedge-short",
        OrderSide::Sell,
        101.0,
        now_ms - 1_000,
    );
    let ledger = vec![
        fill_event(&long, HedgeLegRole::Long),
        fill_event(&short, HedgeLegRole::Short),
    ];

    let snapshot = runtime_snapshot_from_parts(&[long, short], &ledger, &[], now_ms);

    assert_eq!(snapshot.generated_at_ms, now_ms);
    assert_eq!(snapshot.executed.generated_at_ms, now_ms);
    assert_eq!(snapshot.strategy_performance.generated_at_ms, now_ms);
    assert_eq!(snapshot.executed.rows.len(), 1);
    assert_eq!(snapshot.executed.rows[0].id, "runtime-hedge");
    assert_eq!(snapshot.strategy_performance.rows.len(), 1);
    assert_eq!(
        snapshot.strategy_performance.rows[0].total_trades_30d,
        snapshot.executed.page.total_rows as u32
    );
}
