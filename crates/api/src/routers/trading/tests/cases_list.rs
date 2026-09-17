#![allow(clippy::panic)]
use super::super::*;
use super::fixtures::*;

#[tokio::test]
async fn list_orders_returns_paged_envelope_and_query_problems() {
    let state = test_state().await;
    submitted_record(&state, "manual-page-a", "client-page-a").await;
    submitted_record(&state, "manual-page-b", "client-page-b").await;
    submitted_record(&state, "manual-page-c", "client-page-c").await;

    let Json(page) = list_orders(
        State(state.clone()),
        Query(OrderListQuery {
            limit: Some(2),
            cursor: None,
            state: None,
            since_ms: None,
        }),
    )
    .await;

    assert_eq!(page.rows.len(), 2);
    assert_eq!(page.page.limit, 2);
    assert_eq!(page.page.total_rows, 3);
    assert_eq!(page.page.returned_count, 2);
    assert!(page.page.has_more);
    assert_eq!(page.page.next_cursor.as_deref(), Some("2"));
    assert_eq!(page.status, ListStatus::Fresh);
    assert_eq!(page.source, ORDER_LIST_SOURCE);

    let Json(degraded) = list_orders(
        State(state),
        Query(OrderListQuery {
            limit: Some(usize::MAX),
            cursor: Some("bad-cursor".to_owned()),
            state: Some("not_a_state".to_owned()),
            since_ms: Some("-1".to_owned()),
        }),
    )
    .await;

    assert_eq!(degraded.page.limit, ORDER_LIST_MAX_LIMIT);
    assert_eq!(degraded.page.start_offset, 0);
    assert_eq!(degraded.status, ListStatus::Degraded);
    assert!(degraded
        .problems
        .iter()
        .any(|problem| problem.code == codes::LIST_LIMIT_CLAMPED));
    assert!(degraded
        .problems
        .iter()
        .any(|problem| problem.code == codes::LIST_CURSOR_INVALID));
    assert!(degraded
        .problems
        .iter()
        .any(|problem| problem.code == codes::LIST_FILTER_INVALID));
}

#[tokio::test]
async fn list_orders_filters_state_and_since_ms_before_pagination() {
    let state = test_state().await;
    state
        .trading_service()
        .update_risk_config(|config| config.live_trading_enabled = true);

    let kept = submitted_live_record(&state, "manual-filter-a", "client-filter-a").await;
    submitted_live_record(&state, "manual-filter-b", "client-filter-b").await;
    let cancelled = cancelled_record(&state, &kept.intent.id).await;

    let Json(page) = list_orders(
        State(state),
        Query(OrderListQuery {
            limit: Some(10),
            cursor: None,
            state: Some("cancelRequested".to_owned()),
            since_ms: Some(cancelled.updated_at_ms.to_string()),
        }),
    )
    .await;

    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.page.total_rows, 1);
    assert_eq!(page.rows[0].intent.id, cancelled.intent.id);
    assert_eq!(page.rows[0].state, LiveOrderState::CancelRequested);
    assert!(page.rows[0].updated_at_ms >= cancelled.updated_at_ms);
    assert_eq!(page.status, ListStatus::Fresh);
}

#[tokio::test]
async fn list_execution_ledger_returns_query_envelope() {
    let state = test_state().await;
    let record = submitted_record(&state, "manual-ledger", "client-ledger").await;
    let internal_order_id = record.intent.id.clone();

    let Json(page) = list_execution_ledger(
        State(state),
        Query(ExecutionLedgerListQuery {
            limit: Some(16),
            internal_order_id: Some(internal_order_id.clone()),
            exchange_order_id: None,
            hedge_group_id: None,
            run_id: None,
            ticket_id: None,
            leg_role: None,
            from_ms: None,
            to_ms: None,
        }),
    )
    .await;

    assert!(!page.rows.is_empty());
    assert_eq!(page.source, EXECUTION_LEDGER_LIST_SOURCE);
    assert_eq!(page.status, ListStatus::Fresh);
    assert_eq!(page.page.total_rows, page.rows.len());
    assert!(
        page.rows
            .iter()
            .all(|event| event.order.identity.internal_order_id.as_str()
                == internal_order_id.as_str())
    );
}

#[tokio::test]
async fn list_execution_runs_filters_exact_context() {
    let state = test_state().await;
    execution_runs::record(&state, execution_run("run-a", "ticket-a", "opp-a", 1));
    execution_runs::record(&state, execution_run("run-b", "ticket-b", "opp-b", 3));
    execution_runs::record(&state, execution_run("run-c", "ticket-c", "opp-a", 2));

    let Json(filtered_page) = list_execution_runs(
        State(state.clone()),
        Query(ExecutionRunListQuery {
            run_id: None,
            ticket_id: None,
            opportunity_id: Some("opp-a".to_owned()),
        }),
    )
    .await;
    let Json(recent_page) =
        list_execution_runs(State(state), Query(ExecutionRunListQuery::default())).await;

    let filtered_ids: Vec<_> = filtered_page
        .rows
        .into_iter()
        .map(|run| run.run_id)
        .collect();
    assert_eq!(filtered_ids, ["run-c", "run-a"]);
    assert_eq!(filtered_page.source, EXECUTION_RUN_LIST_SOURCE);
    assert_eq!(filtered_page.status, ListStatus::Fresh);
    assert_eq!(filtered_page.page.returned_count, 2);
    assert_eq!(filtered_page.page.total_rows, 2);
    assert_eq!(
        recent_page.rows.first().map(|run| run.run_id.as_str()),
        Some("run-b")
    );
    assert_eq!(recent_page.page.limit, EXECUTION_RUN_LIST_DEFAULT_LIMIT);
}

#[tokio::test]
async fn list_balances_returns_evidence_envelope() {
    let state = test_state().await;

    let Json(envelope) = list_balances(State(state)).await;

    assert_eq!(envelope.source, "account_balance_runtime");
    assert_eq!(envelope.row_count, envelope.rows.len());
    assert!(matches!(
        envelope.status,
        ListStatus::Fresh | ListStatus::Degraded
    ));
}

#[tokio::test]
async fn list_positions_returns_evidence_envelope() {
    let state = test_state().await;

    let Json(envelope) = list_positions(State(state)).await;

    assert_eq!(envelope.source, "account_position_runtime");
    assert_eq!(envelope.row_count, envelope.rows.len());
    assert!(matches!(
        envelope.status,
        ListStatus::Fresh | ListStatus::Degraded
    ));
}

#[tokio::test]
async fn account_state_serves_warming_before_cache_ready() {
    let state = test_state().await;

    let Json(snapshot) = account_state_snapshot(State(state)).await;

    assert_eq!(snapshot.source, "account_state_warming");
    assert_eq!(snapshot.status, ListStatus::Degraded);
    assert!(snapshot
        .problems
        .iter()
        .any(|problem| problem.code == codes::ACCOUNT_STATE_SNAPSHOT_WARMING));
}

#[tokio::test]
async fn account_state_reads_cached_portfolio_snapshot() {
    let state = test_state().await;
    let portfolio = crate::services::portfolio::snapshot(&state)
        .await
        .unwrap_or_else(|error| panic!("portfolio snapshot failed: {error}"));
    state.cache_portfolio_snapshot(portfolio);

    let Json(snapshot) = account_state_snapshot(State(state)).await;

    assert_eq!(snapshot.source, "account_state_runtime");
    assert!(matches!(
        snapshot.status,
        ListStatus::Fresh | ListStatus::Degraded
    ));
    assert_eq!(snapshot.balances.row_count, snapshot.balances.rows.len());
    assert_eq!(snapshot.positions.row_count, snapshot.positions.rows.len());
    assert!(snapshot
        .field_quality
        .iter()
        .any(|row| row.field == "equity"
            && row.status == shared_types::AccountFieldQualityStatus::Unknown));
    assert!(snapshot
        .problems
        .iter()
        .any(|problem| problem.code == codes::ACCOUNT_FIELD_UNKNOWN));
}
