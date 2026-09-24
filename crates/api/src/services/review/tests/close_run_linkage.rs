use super::super::*;
use super::*;
use shared_types::{
    CloseLeg, CloseLegStatus, CloseRun, CloseRunCostReconciliation, CloseRunScope, CloseRunStatus,
    PositionPairEvidence, PositionPairEvidenceSource, PositionSide,
};

#[test]
fn executed_envelope_attaches_matching_close_run_evidence() {
    let long = order("hedge-1-long", OrderSide::Buy, 100.0);
    let short = order("hedge-1-short", OrderSide::Sell, 101.0);
    let orders = vec![long.clone(), short.clone()];
    let ledger = vec![
        linked_fill_event(&long, HedgeLegRole::Long, "run-1", "ticket-1"),
        linked_fill_event(&short, HedgeLegRole::Short, "run-1", "ticket-1"),
    ];
    let close_runs = vec![
        review_close_run("close-1", "run-1", "ticket-1"),
        review_close_run("close-other", "run-other", "ticket-other"),
    ];

    let baseline = executed_envelope(&orders, &ledger, 1, &ReviewPageQuery::default());
    let envelope = executed_envelope_with_close_runs(
        &orders,
        &ledger,
        &close_runs,
        1,
        &ReviewPageQuery::default(),
    );

    assert_eq!(envelope.rows.len(), 1);
    assert_eq!(
        envelope.rows[0].gross_pnl_usd,
        baseline.rows[0].gross_pnl_usd
    );
    assert_eq!(envelope.rows[0].fee_usd, baseline.rows[0].fee_usd);
    assert_eq!(
        envelope.rows[0].funding_usd,
        baseline.rows[0].funding_usd + 0.25
    );
    assert_eq!(envelope.rows[0].slippage_usd, baseline.rows[0].slippage_usd);
    assert_eq!(
        envelope.rows[0].net_pnl_usd,
        baseline.rows[0].net_pnl_usd - 0.75
    );
    assert_close_run_evidence(&envelope.rows[0]);
}

#[test]
fn scoped_review_filters_before_paging_and_requires_same_identity_tuple() {
    use shared_types::review::ReviewScope;
    let mut orders = Vec::new();
    let mut ledger = Vec::new();
    for group in ["a-newer", "z-requested"] {
        for (side, role, suffix) in [
            (OrderSide::Buy, HedgeLegRole::Long, "long"),
            (OrderSide::Sell, HedgeLegRole::Short, "short"),
        ] {
            let row = order(&format!("{group}-{suffix}"), side, 100.0);
            ledger.push(linked_fill_event(
                &row,
                role,
                group,
                &format!("ticket-{group}"),
            ));
            orders.push(row);
        }
    }
    let closes = vec![review_close_run(
        "close-target",
        "z-requested",
        "ticket-z-requested",
    )];
    let now = common::time::now_ms();
    let materialized = super::super::executed_projection::materialize_executed_at(
        &orders, &ledger, &closes, 365, now,
    );
    let scope = ReviewScope {
        run_id: Some("z-requested".into()),
        ticket_id: Some("ticket-z-requested".into()),
        opportunity_id: Some("opp-1".into()),
        close_run_id: None,
    };
    let query = |scope: &ReviewScope| {
        super::super::executed_projection::scoped_executed_envelope_at(
            &materialized,
            scope,
            None,
            &ReviewPageQuery::new(Some(1), None),
            super::super::executed_projection::ExecutedEnvelopeContext {
                ledger: &ledger,
                days: 365,
                offset: 0,
                limit: 1,
                now_ms: now,
                from_ms: now - 365 * DAY_MS,
                page_status: ListStatus::Fresh,
                page_problems: Vec::new(),
            },
        )
    };
    let result = query(&scope);
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].id, "z-requested");
    assert_eq!(result.page.total_rows, 1);
    assert_eq!(
        result.rows[0],
        materialized
            .trades
            .iter()
            .find(|row| row.id == "z-requested")
            .unwrap()
            .clone()
    );
    let mut wrong = scope.clone();
    wrong.ticket_id = Some("ticket-a-newer".into());
    assert!(query(&wrong).rows.is_empty());
    wrong = scope.clone();
    wrong.opportunity_id = Some("wrong-opportunity".into());
    assert!(query(&wrong).rows.is_empty());
    wrong = scope.clone();
    wrong.close_run_id = Some("unrelated-close".into());
    assert!(query(&wrong).rows.is_empty());
    let close = ReviewScope {
        close_run_id: Some("close-target".into()),
        ..Default::default()
    };
    assert_eq!(query(&close).rows[0].id, "z-requested");
    assert_ne!(query(&close).page.snapshot_id, result.page.snapshot_id);
    assert!(!ReviewScope::default().is_valid());
    assert!(!ReviewScope {
        run_id: Some(String::new()),
        ..Default::default()
    }
    .is_valid());
}

fn assert_close_run_evidence(row: &ExecutedTrade) {
    let close_run_evidence = &row.evidence.close_run_evidence;
    assert_eq!(close_run_evidence.len(), 1);
    assert_eq!(close_run_evidence[0].close_run_id, "close-1");
    assert_eq!(close_run_evidence[0].run_id, "run-1");
    assert_eq!(close_run_evidence[0].ticket_id, "ticket-1");
    assert_eq!(close_run_evidence[0].status, CloseRunStatus::Compensated);
    assert_eq!(close_run_evidence[0].matched_notional_usd, 100.0);
    let reconciliation = close_run_evidence[0].cost_reconciliation.as_ref();
    assert!(reconciliation.is_some(), "close run cost reconciliation");
    if let Some(reconciliation) = reconciliation {
        assert_eq!(
            reconciliation.evidence_event_ids,
            vec!["funding-close-1".to_owned(), "manual-close-1".to_owned()]
        );
        assert_eq!(reconciliation.total_actual_cost_usd, Some(1.25));
    }
}

fn linked_fill_event(
    order: &OrderRecord,
    role: HedgeLegRole,
    run_id: &str,
    ticket_id: &str,
) -> ExecutionLedgerEvent {
    let mut event = fill_event(order, role);
    event.order.run_id = Some(run_id.to_owned());
    event.order.ticket_id = Some(ticket_id.to_owned());
    event
}

fn review_close_run(id: &str, run_id: &str, ticket_id: &str) -> CloseRun {
    CloseRun {
        id: id.to_owned(),
        scope: CloseRunScope::Pair,
        status: CloseRunStatus::Compensated,
        action_run_id: None,
        request_id: None,
        idempotency_key: None,
        snapshot_version: "snapshot-1".to_owned(),
        expected_leg_count: 1,
        reason: None,
        legs: vec![CloseLeg {
            venue: "mock".to_owned(),
            symbol: "BTC".to_owned(),
            side: PositionSide::Long,
            status: CloseLegStatus::Filled,
            quantity: 1.0,
            mark_price: 100.0,
            notional_usd: 100.0,
            order: None,
            finality_source: Some(OrderUpdateSource::PrivateWs),
            confirmed_filled_at_ms: Some(1_000),
            problem: None,
            pair_evidence: Some(PositionPairEvidence {
                source: PositionPairEvidenceSource::ExecutionRun,
                run_id: run_id.to_owned(),
                ticket_id: ticket_id.to_owned(),
                opportunity_id: "opp-1".to_owned(),
                venue: "mock".to_owned(),
                symbol: "BTC".to_owned(),
                side: PositionSide::Long,
                partner_venue: "okx".to_owned(),
                partner_symbol: "BTC".to_owned(),
                partner_side: PositionSide::Short,
                leg_filled_quantity: 1.0,
                partner_filled_quantity: 1.0,
                matched_notional_usd: 100.0,
                updated_at_ms: 1_000,
            }),
            cost_events: Vec::new(),
        }],
        submitted_order_count: 1,
        failed_leg_count: 0,
        naked_exposure_usd: 0.0,
        message: "compensated".to_owned(),
        problem: None,
        finality_problem: None,
        finality_checked_at_ms: Some(1_000),
        unwind_plan: None,
        cost_events: Vec::new(),
        cost_reconciliation: Some(CloseRunCostReconciliation {
            funding_usd: Some(0.25),
            manual_handling_usd: Some(1.0),
            total_actual_cost_usd: Some(1.25),
            evidence_event_ids: vec![format!("funding-{id}"), format!("manual-{id}")],
            funding_event_ids: vec![format!("funding-{id}")],
            manual_handling_event_ids: vec![format!("manual-{id}")],
            ..CloseRunCostReconciliation::default()
        }),
        started_at_ms: 1_000,
        updated_at_ms: 1_000,
    }
}
