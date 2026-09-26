use super::super::*;
use super::*;
use trading::{SqlLedgerInit, SqlLedgerMigrationHealth, SqlLedgerReplay, SqlLedgerReplayHealth};

#[tokio::test]
async fn configured_history_failure_does_not_publish_memory_as_complete() {
    let now = common::time::now_ms();
    let (orders, events, close) = historical_review_fixture(now);
    let mut init = sql_replay_init(orders, events);
    init.migration_health.configured = true;
    let service = TradingService::new_mock_with_storage_paths_and_sql(None, None, init);
    assert!(!service.list_execution_ledger_events().is_empty());
    let closes = [close];
    let direct =
        executed_envelope_from_trading(&service, &closes, 30, &ReviewPageQuery::default()).await;
    let scope = shared_types::review::ReviewScope {
        run_id: Some("run-hot".into()),
        ticket_id: None,
        opportunity_id: None,
        close_run_id: None,
    };
    let scoped = scoped_executed_from_trading(
        &service,
        &closes,
        30,
        &ReviewPageQuery::default(),
        &scope,
        None,
    )
    .await;
    let runtime = runtime_snapshot_from_trading(&service, &closes).await;
    for envelope in [direct, scoped, runtime.executed] {
        assert!(
            envelope.rows.is_empty(),
            "failed database reads cannot report cached rows as complete"
        );
        assert_eq!(envelope.status, ListStatus::Degraded);
        assert!(envelope.missing_fields.contains(&ReviewPnlField::Net));
        assert_eq!(envelope.problems[0].code, codes::REVIEW_HISTORY_READ_FAILED);
        assert!(!envelope.problems[0].message.contains("postgres"));
    }
    assert!(runtime.strategy_performance.rows.is_empty());
    assert_eq!(
        runtime.strategy_performance.problems[0].code,
        codes::REVIEW_HISTORY_READ_FAILED
    );
}

pub(crate) fn historical_review_fixture(
    now: i64,
) -> (Vec<OrderRecord>, Vec<ExecutionLedgerEvent>, CloseRun) {
    let mut orders = Vec::new();
    let mut events = Vec::new();
    for (group, run, ticket, time) in [
        ("hot-hedge", "run-hot", "ticket-hot", now - 40 * DAY_MS),
        ("recent-hedge", "run-recent", "ticket-recent", now - 30_000),
        (
            "unrelated-hedge",
            "run-other",
            "ticket-other",
            now - 50 * DAY_MS,
        ),
    ] {
        for (suffix, side, role, price) in [
            ("long", OrderSide::Buy, HedgeLegRole::Long, 100.0),
            ("short", OrderSide::Sell, HedgeLegRole::Short, 101.0),
        ] {
            let order = order_at(&format!("{group}-{suffix}"), side, price, time);
            events.push(linked_fill_event(&order, role, run, ticket));
            orders.push(order);
        }
    }
    let mut close = completed_hot_close_run(now - 1_000);
    for leg in &mut close.legs {
        leg.order.as_mut().unwrap().intent.mode = ExecutionMode::DryRun;
    }
    (orders, events, close)
}

pub(crate) fn partial_close_review_fixture(
    now: i64,
) -> (Vec<OrderRecord>, Vec<ExecutionLedgerEvent>, Vec<CloseRun>) {
    let (orders, events, template) = historical_review_fixture(now);
    let mut runs = Vec::new();
    for (id, quantity, time, cancelled) in [
        ("partial-hot", 0.4, now - 4_000, true),
        ("retry-hot", 0.6, now - 1_000, false),
    ] {
        let mut run = template.clone();
        run.id = id.into();
        run.started_at_ms = time - 10;
        run.updated_at_ms = now - 500;
        if cancelled {
            run.status = shared_types::CloseRunStatus::UnwindRequired;
            run.finality_problem = Some(ApiProblem::new(codes::CLOSE_RUN_FAILED, "部分成交后撤单"));
        }
        for (index, leg) in run.legs.iter_mut().enumerate() {
            leg.quantity = if cancelled { 1.0 } else { quantity };
            leg.confirmed_filled_at_ms = (!cancelled).then_some(time);
            let order = leg.order.as_mut().unwrap();
            order.intent.id = format!("{id}-{index}");
            order.intent.client_order_id = format!("{id}-{index}-client");
            order.intent.quantity = leg.quantity;
            order.intent.created_at_ms = time - 10;
            order.identity = shared_types::VenueOrderIdentity::from_intent(&order.intent);
            order.filled_quantity = Some(quantity);
            order.filled_fee = Some(0.01 * quantity);
            order.updated_at_ms = now - 500;
            if cancelled {
                order.state = LiveOrderState::Cancelled;
                leg.status = shared_types::CloseLegStatus::Cancelled;
            }
            leg.ledger_fills = Some(shared_types::CloseFillLedger {
                totals: shared_types::ExecutionLedgerFillTotals {
                    quantity, notional: quantity * order.filled_price.unwrap(), fee: order.filled_fee,
                },
                event_ids: vec![format!("fill-{id}-{index}")],
                last_fill_at_ms: Some(time),
            });
        }
        run.cost_reconciliation = Some(shared_types::CloseRunCostReconciliation {
            close_fee_usd: Some(0.02 * quantity), close_fee_event_ids: vec![format!("fee-{id}")],
            close_slippage_usd: Some(0.0), close_slippage_event_ids: vec![format!("slippage-{id}")],
            funding_usd: Some(0.0), funding_event_ids: vec![format!("funding-{id}")],
            evidence_event_ids: vec![format!("fee-{id}"), format!("slippage-{id}"), format!("funding-{id}")],
            ..Default::default()
        });
        runs.push(run);
    }
    (orders, events, runs)
}

#[tokio::test]
async fn partial_close_retry_keeps_api_rows_and_strategy_totals_consistent() {
    let now = common::time::now_ms();
    let (orders, events, runs) = partial_close_review_fixture(now);
    let service = TradingService::new_mock_with_storage_paths_and_sql(None, None, sql_replay_init(orders, events));
    let result = runtime_snapshot_from_trading(&service, &runs).await;
    let row = result.executed.rows.iter().find(|row| row.id == "hot-hedge").unwrap();
    assert_eq!(row.closed_at_ms, Some(now - 1_000));
    assert!((row.fee_usd - 0.04).abs() < 1e-9);
    assert!((row.net_pnl_usd + 0.04).abs() < 1e-9);
    assert!(!row.missing_fields.contains(&ReviewPnlField::Net));
    assert_eq!(row.evidence.close_run_evidence.len(), 2);
    let strategy = &result.strategy_performance.rows[0];
    assert_eq!(strategy.trades_30d, 1);
    assert!((strategy.estimated_net_pnl_30d_usd + 0.04).abs() < 1e-9);
}

#[tokio::test]
async fn review_window_recovers_old_openings_and_pages_by_close_time() {
    let now = common::time::now_ms();
    let (orders, events, close) = historical_review_fixture(now);
    let service = TradingService::new_mock_with_storage_paths_and_sql(
        None,
        None,
        sql_replay_init(orders, events),
    );
    let closes = [close];
    let page =
        executed_envelope_from_trading(&service, &closes, 1, &ReviewPageQuery::new(Some(1), None))
            .await;
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].id, "hot-hedge");
    assert_eq!(page.rows[0].closed_at_ms, Some(now - 1_000));
    assert!((page.rows[0].net_pnl_usd + 0.04).abs() < 1e-9);
    let next = executed_envelope_from_trading(
        &service,
        &closes,
        1,
        &ReviewPageQuery::new(Some(1), page.page.next_cursor.clone()),
    )
    .await;
    assert_eq!(next.rows.len(), 1);
    assert_eq!(next.rows[0].id, "recent-hedge");
    let runtime = runtime_snapshot_from_trading(&service, &closes).await;
    assert_eq!(runtime.executed.rows.len(), 2);
    assert_eq!(runtime.executed.rows[0].id, "hot-hedge");
    assert_eq!(runtime.strategy_performance.rows[0].trades_30d, 1);
    assert!((runtime.strategy_performance.rows[0].estimated_net_pnl_30d_usd + 0.04).abs() < 1e-9);
}

#[tokio::test]
async fn executed_envelope_uses_sql_replayed_events_and_order_snapshots() {
    let now_ms = common::time::now_ms();
    let long = order_at("sql-hedge-long", OrderSide::Buy, 100.0, now_ms - 1_000);
    let short = order_at("sql-hedge-short", OrderSide::Sell, 101.0, now_ms - 1_000);
    let events = vec![
        fill_event(&long, HedgeLegRole::Long),
        fill_event(&short, HedgeLegRole::Short),
    ];
    let service = TradingService::new_mock_with_storage_paths_and_sql(
        None,
        None,
        sql_replay_init(vec![long, short], events),
    );

    let envelope =
        executed_envelope_from_trading(&service, &[], 1, &ReviewPageQuery::default()).await;

    assert_eq!(envelope.rows.len(), 1);
    assert_eq!(envelope.rows[0].id, "sql-hedge");
    assert_eq!(envelope.rows[0].evidence.fill_event_ids.len(), 2);
    assert!(envelope
        .storage_health
        .as_ref()
        .is_some_and(|health| health.source == SQL_LEDGER_STORAGE_SOURCE
            && health.rows == Some(2)
            && health.requested == Some(1)));
}

#[tokio::test]
async fn scoped_review_reads_original_ledger_and_checks_opportunity_identity() {
    use serde_json::json;
    use shared_types::review::ReviewScope;
    let now = common::time::now_ms();
    let long = order_at("scoped-long", OrderSide::Buy, 100.0, now - 1_000);
    let short = order_at("scoped-short", OrderSide::Sell, 101.0, now - 1_000);
    let events = vec![
        linked_fill_event(&long, HedgeLegRole::Long, "run-a", "ticket-a"),
        linked_fill_event(&short, HedgeLegRole::Short, "run-a", "ticket-a"),
    ];
    let service = TradingService::new_mock_with_storage_paths_and_sql(
        None,
        None,
        sql_replay_init(vec![long, short], events),
    );
    let leg = |role| json!({"role":role,"exchange":"fixture","symbol":"BTC","orderIds":[],"state":"filled","targetQuantity":1.0,"filledQuantity":1.0,"targetNotionalUsd":100.0});
    let run: shared_types::ExecutionRun = serde_json::from_value(json!({"runId":"run-a","ticketId":"ticket-a","opportunityId":"opp-a", "state":"hedged","longLeg":leg("long"),"shortLeg":leg("short"),"netExposureUsd":0.0,"statusReason":"fixture","createdAtMs":now-1000,"updatedAtMs":now})).unwrap();
    let mut scope = ReviewScope {
        run_id: Some("run-a".into()),
        ticket_id: Some("ticket-a".into()),
        opportunity_id: Some("opp-a".into()),
        close_run_id: None,
    };
    let result = scoped_executed_from_trading(
        &service,
        &[],
        365,
        &ReviewPageQuery::default(),
        &scope,
        Some(&run),
    )
    .await;
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].id, "scoped");
    assert_eq!(result.rows[0].evidence.fill_event_ids.len(), 2);
    assert!(result.storage_health.is_some());
    scope.opportunity_id = Some("wrong".into());
    assert!(scoped_executed_from_trading(
        &service,
        &[],
        365,
        &ReviewPageQuery::default(),
        &scope,
        Some(&run)
    )
    .await
    .rows
    .is_empty());
}

#[tokio::test]
async fn strategy_performance_uses_hot_close_run_when_durable_window_has_none(
) -> Result<(), &'static str> {
    let now_ms = common::time::now_ms();
    let mut long = order_at("hot-hedge-long", OrderSide::Buy, 100.0, now_ms - 2_000);
    let mut short = order_at("hot-hedge-short", OrderSide::Sell, 101.0, now_ms - 2_000);
    long.intent.mode = ExecutionMode::Live;
    short.intent.mode = ExecutionMode::Live;
    let events = vec![
        linked_fill_event(&long, HedgeLegRole::Long, "run-hot", "ticket-hot"),
        linked_fill_event(&short, HedgeLegRole::Short, "run-hot", "ticket-hot"),
    ];
    let service = TradingService::new_mock_with_storage_paths_and_sql(
        None,
        None,
        sql_replay_init(vec![long, short], events),
    );
    let hot_close_run = completed_hot_close_run(now_ms - 1_000);

    let envelope = runtime_snapshot_from_trading(&service, &[hot_close_run])
        .await
        .strategy_performance;
    let row = envelope
        .rows
        .iter()
        .find(|row| row.kind == StrategyKind::SpotPerp)
        .ok_or("spot-perp performance row missing")?;

    assert_eq!(row.total_trades_30d, 1);
    assert_eq!(row.trades_30d, 1);
    assert_eq!(row.skipped_trades_30d, 0);
    assert_eq!(row.actual_trades_30d, 1);
    assert_eq!(row.estimated_trades_30d, 0);
    assert_eq!(
        row.sample_status,
        StrategyPerformanceSampleStatus::PartialEvidence
    );
    assert!((row.net_pnl_30d_usd + 0.04).abs() < 1e-9);
    assert!((row.actual_net_pnl_30d_usd + 0.04).abs() < 1e-9);
    assert!(
        row.estimated_net_pnl_30d_usd.abs() < 1e-9,
        "confirmed live cash PnL must not be counted again as estimated: {}",
        row.estimated_net_pnl_30d_usd
    );
    Ok(())
}

#[test]
fn close_runs_for_review_keeps_newer_snapshot_by_id() {
    let durable =
        close_run_for_review_merge("close-1", 100, shared_types::CloseRunStatus::Submitted);
    let hot = close_run_for_review_merge("close-1", 200, shared_types::CloseRunStatus::Compensated);

    let close_runs = close_runs_for_review(&[hot], &[durable]);

    assert_eq!(close_runs.len(), 1);
    assert_eq!(close_runs[0].id, "close-1");
    assert_eq!(
        close_runs[0].status,
        shared_types::CloseRunStatus::Compensated
    );
    assert_eq!(close_runs[0].updated_at_ms, 200);
}

fn sql_replay_init(
    order_snapshots: Vec<OrderRecord>,
    events: Vec<ExecutionLedgerEvent>,
) -> SqlLedgerInit {
    let health = SqlLedgerReplayHealth {
        query_successes: 2,
        event_rows: events.len(),
        replayed_events: events.len(),
        snapshot_rows: order_snapshots.len(),
        replayed_order_snapshots: order_snapshots.len(),
        replay_limit: 50_000,
        last_query_at_ms: Some(common::time::now_ms()),
        ..SqlLedgerReplayHealth::default()
    };
    SqlLedgerInit {
        migration_health: sql_migration_health(),
        replay: SqlLedgerReplay {
            events,
            order_snapshots,
            balance_events: Vec::new(),
            run_finality_events: Vec::new(),
            health,
        },
        store: None,
    }
}

fn sql_migration_health() -> SqlLedgerMigrationHealth {
    SqlLedgerMigrationHealth {
        // These unit fixtures exercise restored rows without an active SQL store.
        configured: false,
        migration_id: trading::SQL_LEDGER_MIGRATION_ID,
        migration_path: trading::SQL_LEDGER_MIGRATION_PATH,
        migration_checksum: trading::sql_ledger_schema_hash(),
        schema_version: Some(trading::SQL_LEDGER_MIGRATION_VERSION),
        applied: true,
        degraded_reason: None,
        last_success_at_ms: Some(common::time::now_ms()),
        last_error_at_ms: None,
        last_error: None,
        observed_at_ms: common::time::now_ms(),
    }
}

fn close_run_for_review_merge(
    id: &str,
    updated_at_ms: i64,
    status: shared_types::CloseRunStatus,
) -> CloseRun {
    CloseRun {
        id: id.to_owned(),
        scope: shared_types::CloseRunScope::Pair,
        status,
        action_run_id: None,
        request_id: None,
        idempotency_key: None,
        snapshot_version: "snapshot-1".to_owned(),
        expected_leg_count: 0,
        reason: None,
        legs: Vec::new(),
        submitted_order_count: 0,
        failed_leg_count: 0,
        naked_exposure_usd: 0.0,
        message: "merge".to_owned(),
        problem: None,
        finality_problem: None,
        finality_checked_at_ms: Some(updated_at_ms),
        unwind_plan: None,
        cost_reconciliation: None,
        cost_events: Vec::new(),
        started_at_ms: 1,
        updated_at_ms,
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

fn completed_hot_close_run(filled_at_ms: i64) -> CloseRun {
    CloseRun {
        id: "close-hot".to_owned(),
        scope: shared_types::CloseRunScope::Pair,
        status: shared_types::CloseRunStatus::Succeeded,
        action_run_id: None,
        request_id: None,
        idempotency_key: None,
        snapshot_version: "snapshot-hot".to_owned(),
        expected_leg_count: 2,
        reason: Some("test".to_owned()),
        legs: vec![
            hot_close_leg(
                "close-hot-long",
                shared_types::PositionSide::Long,
                OrderSide::Sell,
                100.0,
                filled_at_ms,
            ),
            hot_close_leg(
                "close-hot-short",
                shared_types::PositionSide::Short,
                OrderSide::Buy,
                101.0,
                filled_at_ms,
            ),
        ],
        submitted_order_count: 2,
        failed_leg_count: 0,
        naked_exposure_usd: 0.0,
        message: "closed".to_owned(),
        problem: None,
        finality_problem: None,
        finality_checked_at_ms: Some(filled_at_ms),
        unwind_plan: None,
        cost_reconciliation: Some(shared_types::CloseRunCostReconciliation {
            close_fee_usd: Some(0.02),
            close_slippage_usd: Some(0.0),
            funding_usd: Some(0.0),
            total_actual_cost_usd: Some(0.02),
            evidence_event_ids: vec![
                "close-hot-fee".to_owned(),
                "close-hot-slippage".to_owned(),
                "close-hot-funding".to_owned(),
            ],
            close_fee_event_ids: vec!["close-hot-fee".to_owned()],
            close_slippage_event_ids: vec!["close-hot-slippage".to_owned()],
            funding_event_ids: vec!["close-hot-funding".to_owned()],
            ..shared_types::CloseRunCostReconciliation::default()
        }),
        cost_events: Vec::new(),
        started_at_ms: filled_at_ms,
        updated_at_ms: filled_at_ms,
    }
}

fn hot_close_leg(
    id: &str,
    side: shared_types::PositionSide,
    order_side: OrderSide,
    price: f64,
    filled_at_ms: i64,
) -> shared_types::CloseLeg {
    let mut order = order_at(id, order_side, price, filled_at_ms - 10);
    order.intent.source = OrderSource::Manual;
    order.intent.mode = ExecutionMode::Live;
    order.intent.reduce_only = true;
    order.updated_at_ms = filled_at_ms;
    shared_types::CloseLeg {
        venue: "mock".to_owned(),
        symbol: "BTC".to_owned(),
        side,
        status: shared_types::CloseLegStatus::Filled,
        quantity: 1.0,
        mark_price: price,
        notional_usd: price,
        order: Some(order),
        finality_source: Some(OrderUpdateSource::PrivateWs),
        confirmed_filled_at_ms: Some(filled_at_ms),
        problem: None,
        pair_evidence: Some(shared_types::PositionPairEvidence {
            source: shared_types::PositionPairEvidenceSource::ExecutionRun,
            run_id: "run-hot".to_owned(),
            ticket_id: "ticket-hot".to_owned(),
            opportunity_id: "opp-hot".to_owned(),
            venue: "mock".to_owned(),
            symbol: "BTC".to_owned(),
            side,
            partner_venue: "mock".to_owned(),
            partner_symbol: "BTC".to_owned(),
            partner_side: match side {
                shared_types::PositionSide::Long => shared_types::PositionSide::Short,
                shared_types::PositionSide::Short => shared_types::PositionSide::Long,
            },
            leg_filled_quantity: 1.0,
            partner_filled_quantity: 1.0,
            matched_notional_usd: 100.0,
            updated_at_ms: filled_at_ms,
        }),
        ledger_fills: None,
        cost_events: Vec::new(),
    }
}
