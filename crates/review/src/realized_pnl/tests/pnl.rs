use super::*;
mod helpers;

use helpers::*;
use shared_types::{
    CloseLeg, CloseLegStatus, CloseRun, CloseRunCostReconciliation, CloseRunScope, CloseRunStatus,
    ExecutionLedgerEvent, PositionPairEvidence, PositionPairEvidenceSource, PositionSide,
};

#[test]
fn realizes_price_and_fee_from_fill_ledger() {
    let orders = hedge_orders();
    let ledger = vec![
        fill_event(&orders[0], 100.0, 0.10, 1_000, "fill-long"),
        fill_event(&orders[1], 102.0, 0.11, 2_000, "fill-short"),
    ];

    let rows = realized_pnl_by_group(&orders, &ledger, 0, 10_000);
    let row = rows.get("hedge-1").expect("realized group");

    assert_eq!(row.price_pnl_usd, 2.0);
    assert_close(row.fee_usd, 0.21);
    assert_close(row.net_pnl_usd, 1.79);
    assert_eq!(row.realized_at_ms, 2_000);
    assert_eq!(row.evidence.fill_event_ids, ["fill-long", "fill-short"]);
    assert_eq!(row.evidence.fee_event_ids, ["fill-long", "fill-short"]);
}

#[test]
fn realizes_funding_payment_from_same_group() {
    let orders = hedge_orders();
    let ledger = vec![
        fill_event(&orders[0], 100.0, 0.10, 1_000, "fill-long"),
        funding_event(&orders[0], 0.25, 1_500, "funding-long"),
        fill_event(&orders[1], 102.0, 0.11, 2_000, "fill-short"),
    ];

    let rows = realized_pnl_by_group(&orders, &ledger, 0, 10_000);
    let row = rows.get("hedge-1").expect("realized group");
    let trades = apply_realized_pnl(crate::executed_from_orders(&orders, 10_000, 1), &rows);

    assert_close(row.price_pnl_usd, 2.0);
    assert_close(row.fee_usd, 0.21);
    assert_close(row.funding_usd, 0.25);
    assert_close(row.net_pnl_usd, 2.04);
    assert_eq!(row.evidence.funding_event_ids, ["funding-long"]);
    assert!(!trades[0].missing_fields.contains(&ReviewPnlField::Funding));
    assert!(trades[0]
        .estimated_fields
        .contains(&ReviewPnlField::Slippage));
    assert!(trades[0].missing_fields.contains(&ReviewPnlField::Net));
}

#[test]
fn applies_complete_close_run_cost_to_net_pnl() {
    let orders = hedge_orders();
    let ledger = vec![
        linked_event(
            fill_event(&orders[0], 100.0, 0.10, 1_000, "fill-long"),
            "run-1",
            "ticket-1",
        ),
        linked_event(
            funding_event(&orders[0], 0.0, 1_500, "funding-long"),
            "run-1",
            "ticket-1",
        ),
        linked_event(
            slippage_event(&orders[0], 0.25, 1_600, "slippage-long"),
            "run-1",
            "ticket-1",
        ),
        linked_event(
            fill_event(&orders[1], 102.0, 0.11, 2_000, "fill-short"),
            "run-1",
            "ticket-1",
        ),
        linked_event(
            slippage_event(&orders[1], 0.50, 2_100, "slippage-short"),
            "run-1",
            "ticket-1",
        ),
    ];
    let close_runs = vec![successful_pair_close_run(
        "close-1",
        "run-1",
        "ticket-1",
        99.0,
        103.0,
        CloseRunCostReconciliation {
            close_fee_usd: Some(0.2),
            close_slippage_usd: Some(1.0),
            funding_usd: Some(-0.1),
            manual_handling_usd: Some(2.0),
            total_actual_cost_usd: Some(3.1),
            evidence_event_ids: vec![
                "close-fee-1".to_owned(),
                "close-slippage-1".to_owned(),
                "close-funding-1".to_owned(),
                "manual-1".to_owned(),
            ],
            close_fee_event_ids: vec!["close-fee-1".to_owned()],
            close_slippage_event_ids: vec!["close-slippage-1".to_owned()],
            funding_event_ids: vec!["close-funding-1".to_owned()],
            manual_handling_event_ids: vec!["manual-1".to_owned()],
            ..CloseRunCostReconciliation::default()
        },
    )];

    let rows = realized_pnl_by_group_with_close_runs(&orders, &ledger, &close_runs, 0, 10_000);
    let trades = apply_realized_pnl(crate::executed_from_orders(&orders, 10_000, 1), &rows);

    assert_close(rows["hedge-1"].price_pnl_usd, -2.0);
    assert_close(rows["hedge-1"].fee_usd, 0.41);
    assert_close(rows["hedge-1"].funding_usd, -0.1);
    assert_close(rows["hedge-1"].slippage_usd, 1.75);
    assert_close(rows["hedge-1"].net_pnl_usd, -4.51);
    assert_eq!(rows["hedge-1"].closed_at_ms, Some(4_000));
    assert_eq!(rows["hedge-1"].evidence.close_run_evidence.len(), 1);
    assert!(!trades[0].missing_fields.contains(&ReviewPnlField::Net));
    assert!(trades[0].estimated_fields.contains(&ReviewPnlField::Gross));
    assert!(trades[0].estimated_fields.contains(&ReviewPnlField::Net));

    let perf = crate::compute_performance(&trades, StrategyKind::PerpCross);
    assert_close(perf.estimated_net_pnl_30d_usd, -4.51);
    assert_eq!(perf.actual_trades_30d, 0);
    for slippage in [-5.0, 0.0, 50.0] {
        let mut changed = close_runs.clone();
        changed[0]
            .cost_reconciliation
            .as_mut()
            .unwrap()
            .close_slippage_usd = Some(slippage);
        let changed = realized_pnl_by_group_with_close_runs(&orders, &ledger, &changed, 0, 10_000);
        assert_close(changed["hedge-1"].net_pnl_usd, -4.51);
        assert_close(changed["hedge-1"].slippage_usd, 0.75 + slippage);
    }
    let mut mixed = close_runs.clone();
    mixed[0].cost_events.push(shared_types::CloseRunCostLedgerEvent {
        event_id: "close-slippage-1".into(),
        component: shared_types::CloseRunCostComponent::Slippage,
        amount_usd: 1.0,
        source: OrderUpdateSource::PrivateWs,
        quality: ExecutionLedgerQuality::Actual,
        occurred_at_ms: 4_000,
        captured_at_ms: 4_000,
    });
    let rows = realized_pnl_by_group_with_close_runs(&orders, &ledger, &mixed, 0, 10_000);
    assert_close(rows["hedge-1"].net_pnl_usd, -4.51);
    assert_close(rows["hedge-1"].slippage_usd, 1.75);
}

#[test]
fn closed_cash_pnl_requires_cash_evidence_not_slippage_attribution() {
    let mut orders = hedge_orders();
    for order in &mut orders {
        order.intent.mode = ExecutionMode::Live;
        order.intent.price = None;
    }
    let ledger = vec![
        linked_event(
            fill_event(&orders[0], 100.0, 0.10, 1_000, "fill-long"),
            "run-1",
            "ticket-1",
        ),
        linked_event(
            fill_event(&orders[1], 102.0, 0.11, 2_000, "fill-short"),
            "run-1",
            "ticket-1",
        ),
    ];
    let mut close = successful_pair_close_run(
        "close-1",
        "run-1",
        "ticket-1",
        99.0,
        103.0,
        CloseRunCostReconciliation {
            close_fee_usd: Some(0.2),
            close_fee_event_ids: vec!["close-fee".into()],
            funding_usd: Some(0.0),
            funding_event_ids: vec!["funding-zero".into()],
            missing_fields: vec!["close_slippage".into()],
            ..CloseRunCostReconciliation::default()
        },
    );
    for leg in &mut close.legs {
        leg.order.as_mut().unwrap().intent.mode = ExecutionMode::Live;
    }
    let project = |run: &CloseRun| {
        let rows = realized_pnl_by_group_with_close_runs(
            &orders,
            &ledger,
            std::slice::from_ref(run),
            0,
            10_000,
        );
        apply_realized_pnl(crate::executed_from_orders(&orders, 10_000, 1), &rows)
    };
    let trades = project(&close);
    assert_close(trades[0].net_pnl_usd, -2.41);
    assert!(trades[0].missing_fields.contains(&ReviewPnlField::Slippage));
    assert!(trades[0].actual_fields.contains(&ReviewPnlField::Net));
    let perf = crate::compute_performance(&trades, StrategyKind::PerpCross);
    assert_eq!(perf.actual_trades_30d, 1);
    assert_eq!(perf.losing_trades_30d, 1);
    assert_close(perf.actual_net_pnl_30d_usd, -2.41);

    for missing in [
        "fee",
        "fee_id",
        "funding",
        "manual",
        "legacy_total",
        "finality",
    ] {
        let mut incomplete = close.clone();
        let cost = incomplete.cost_reconciliation.as_mut().unwrap();
        match missing {
            "fee" => {
                cost.close_fee_usd = None;
            }
            "fee_id" => {
                cost.close_fee_event_ids.clear();
            }
            "funding" => {
                cost.funding_usd = None;
                cost.funding_event_ids.clear();
            }
            "manual" => {
                cost.manual_handling_usd = Some(2.0);
            }
            "legacy_total" => {
                *cost = CloseRunCostReconciliation {
                    total_actual_cost_usd: Some(1.2),
                    evidence_event_ids: vec!["opaque-total".into()],
                    ..CloseRunCostReconciliation::default()
                };
            }
            "finality" => {
                incomplete.status = CloseRunStatus::Submitted;
            }
            _ => unreachable!(),
        }
        let trades = project(&incomplete);
        assert!(
            trades[0].missing_fields.contains(&ReviewPnlField::Net),
            "{missing}"
        );
        assert_eq!(
            crate::compute_performance(&trades, StrategyKind::PerpCross).actual_trades_30d,
            0,
            "{missing}"
        );
        if missing == "legacy_total" {
            assert_close(trades[0].net_pnl_usd, -2.21);
        }
    }
}

#[test]
fn accepts_close_run_zero_funding_evidence_without_opening_payment() {
    let orders = hedge_orders();
    let ledger = vec![
        linked_event(
            fill_event(&orders[0], 100.0, 0.10, 1_000, "fill-long"),
            "run-1",
            "ticket-1",
        ),
        linked_event(
            slippage_event(&orders[0], 0.0, 1_100, "slippage-long"),
            "run-1",
            "ticket-1",
        ),
        linked_event(
            fill_event(&orders[1], 102.0, 0.11, 2_000, "fill-short"),
            "run-1",
            "ticket-1",
        ),
        linked_event(
            slippage_event(&orders[1], 0.0, 2_100, "slippage-short"),
            "run-1",
            "ticket-1",
        ),
    ];
    let close_runs = vec![close_run_with_cost(
        "close-1",
        "run-1",
        "ticket-1",
        CloseRunCostReconciliation {
            close_fee_usd: Some(0.2),
            close_slippage_usd: Some(0.0),
            funding_usd: Some(0.0),
            total_actual_cost_usd: Some(0.2),
            close_fee_event_ids: vec!["close-fee-1".to_owned()],
            close_slippage_event_ids: vec!["close-slippage-1".to_owned()],
            funding_event_ids: vec!["paper-funding-model:close-1".to_owned()],
            evidence_event_ids: vec![
                "close-fee-1".to_owned(),
                "close-slippage-1".to_owned(),
                "paper-funding-model:close-1".to_owned(),
            ],
            ..CloseRunCostReconciliation::default()
        },
    )];

    let rows = realized_pnl_by_group_with_close_runs(&orders, &ledger, &close_runs, 0, 10_000);
    let trades = apply_realized_pnl(crate::executed_from_orders(&orders, 10_000, 1), &rows);

    assert!(trades[0].actual_fields.contains(&ReviewPnlField::Funding));
    assert!(trades[0].missing_fields.contains(&ReviewPnlField::Gross));
    assert!(trades[0].missing_fields.contains(&ReviewPnlField::Net));
}

#[test]
fn skips_close_run_cost_component_when_event_id_is_already_in_ledger() {
    let orders = hedge_orders();
    let ledger = vec![
        linked_event(
            fill_event(&orders[0], 100.0, 0.10, 1_000, "fill-long"),
            "run-1",
            "ticket-1",
        ),
        linked_event(
            funding_event(&orders[0], 0.25, 1_500, "funding-long"),
            "run-1",
            "ticket-1",
        ),
        linked_event(
            fill_event(&orders[1], 102.0, 0.11, 2_000, "fill-short"),
            "run-1",
            "ticket-1",
        ),
    ];
    let close_runs = vec![close_run_with_cost(
        "close-1",
        "run-1",
        "ticket-1",
        CloseRunCostReconciliation {
            close_fee_usd: Some(0.2),
            funding_usd: Some(0.25),
            total_actual_cost_usd: Some(0.45),
            evidence_event_ids: vec!["close-fee-1".to_owned(), "funding-long".to_owned()],
            close_fee_event_ids: vec!["close-fee-1".to_owned()],
            funding_event_ids: vec!["funding-long".to_owned()],
            ..CloseRunCostReconciliation::default()
        },
    )];

    let baseline = realized_pnl_by_group(&orders, &ledger, 0, 10_000);
    let rows = realized_pnl_by_group_with_close_runs(&orders, &ledger, &close_runs, 0, 10_000);

    assert_close(
        rows["hedge-1"].net_pnl_usd,
        baseline["hedge-1"].net_pnl_usd - 0.2,
    );
}

#[test]
fn realizes_slippage_from_intent_price_and_fill_ledger() {
    let orders = vec![
        order_with_price("hedge-1-long", OrderSide::Buy, Some(100.0)),
        order_with_price("hedge-1-short", OrderSide::Sell, Some(105.0)),
    ];
    let ledger = vec![
        fill_event(&orders[0], 101.0, 0.10, 1_000, "fill-long"),
        funding_event(&orders[0], 0.0, 1_500, "funding-long"),
        fill_event(&orders[1], 104.0, 0.11, 2_000, "fill-short"),
    ];

    let rows = realized_pnl_by_group(&orders, &ledger, 0, 10_000);
    let row = rows.get("hedge-1").expect("realized group");
    let trades = apply_realized_pnl(crate::executed_from_orders(&orders, 10_000, 1), &rows);

    assert_close(row.price_pnl_usd, 3.0);
    assert_close(row.slippage_usd, 2.0);
    assert!(row.evidence.slippage_event_ids.is_empty());
    assert_close(trades[0].slippage_usd, 2.0);
    assert!(trades[0]
        .estimated_fields
        .contains(&ReviewPnlField::Slippage));
    assert!(trades[0].missing_fields.contains(&ReviewPnlField::Net));
}

#[test]
fn prefers_durable_slippage_event_over_fill_derived_fallback() {
    let orders = vec![
        order_with_price("hedge-1-long", OrderSide::Buy, Some(100.0)),
        order_with_price("hedge-1-short", OrderSide::Sell, Some(105.0)),
    ];
    let ledger = vec![
        slippage_event(&orders[0], 0.25, 1_000, "slippage-fill-long"),
        fill_event(&orders[0], 101.0, 0.10, 1_000, "fill-long"),
        funding_event(&orders[0], 0.0, 1_500, "funding-long"),
        slippage_event(&orders[1], 0.50, 2_000, "slippage-fill-short"),
        fill_event(&orders[1], 104.0, 0.11, 2_000, "fill-short"),
    ];

    let rows = realized_pnl_by_group(&orders, &ledger, 0, 10_000);
    let row = rows.get("hedge-1").expect("realized group");
    let trades = apply_realized_pnl(crate::executed_from_orders(&orders, 10_000, 1), &rows);

    assert_close(row.slippage_usd, 0.75);
    assert_eq!(
        row.evidence.slippage_event_ids,
        ["slippage-fill-long", "slippage-fill-short"]
    );
    assert_close(trades[0].slippage_usd, 0.75);
    assert!(!trades[0].missing_fields.contains(&ReviewPnlField::Slippage));
    assert!(trades[0].actual_fields.contains(&ReviewPnlField::Slippage));
}
