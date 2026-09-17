use super::*;

#[test]
fn snapshot_and_balance_queries_select_newest_then_apply_ascending() {
    assert_latest_replay_query(
        SQL_ORDER_SNAPSHOTS_REPLAY_QUERY,
        "ORDER BY updated_at_ms DESC, internal_order_id DESC LIMIT $1",
        "ORDER BY updated_at_ms ASC, internal_order_id ASC",
        "latest_order_snapshots",
    );
    assert_latest_replay_query(
        SQL_BALANCE_EVENTS_REPLAY_QUERY,
        "ORDER BY observed_at_ms DESC, event_id DESC LIMIT $1",
        "ORDER BY observed_at_ms ASC, event_id ASC",
        "latest_balance_events",
    );
}

#[test]
fn replay_window_discards_only_the_oldest_limit_plus_one_sentinel() {
    let limit = SQL_LEDGER_REPLAY_LIMIT as usize;
    let exact = (0..limit).collect::<Vec<_>>();
    let over_boundary = (0..=limit).collect::<Vec<_>>();

    assert_eq!(retained_replay_rows(exact), (0..limit).collect::<Vec<_>>());
    assert_eq!(
        retained_replay_rows(over_boundary),
        (1..=limit).collect::<Vec<_>>()
    );
}

#[test]
fn run_finality_replay_rejects_tampered_source_links() {
    let run = execution_run();
    let event = SqlRunFinalityLedgerEvent::from_execution_run(
        &run,
        OrderUpdateSource::PrivateWs,
        Some("fill-event-1"),
        Some("long-order"),
        123,
    )
    .expect("run finality event");
    let row = replay_event_from_ledger_event(event);
    let mut tampered_event_source = row.clone();
    tampered_event_source.source_event_id = Some("fill-event-2".to_owned());
    let mut tampered_order_source = row;
    tampered_order_source.source_order_event_id = Some("short-order".to_owned());
    let mut replay = SqlLedgerReplay::with_limit(10);

    for event in [tampered_event_source, tampered_order_source] {
        push_replay_run_finality_result(&mut replay, validate_run_finality_replay_event(event));
    }

    assert!(replay.run_finality_events.is_empty());
    assert_eq!(replay.health.run_finality_decode_failures, 2);
}

#[test]
fn run_finality_replay_accepts_legacy_event_ids() {
    let run = execution_run();
    let event = SqlRunFinalityLedgerEvent::from_execution_run(
        &run,
        OrderUpdateSource::PrivateWs,
        None,
        None,
        123,
    )
    .expect("run finality event");
    let mut row = replay_event_from_ledger_event(event);
    set_legacy_event_id(&mut row);

    assert!(validate_run_finality_replay_event(row).is_ok());
}

#[test]
fn run_finality_replay_rejects_links_added_to_legacy_rows() {
    let run = execution_run();
    let event = SqlRunFinalityLedgerEvent::from_execution_run(
        &run,
        OrderUpdateSource::PrivateWs,
        None,
        None,
        123,
    )
    .expect("run finality event");
    let mut row = replay_event_from_ledger_event(event);
    set_legacy_event_id(&mut row);
    let mut tampered_event_source = row.clone();
    tampered_event_source.source_event_id = Some("fill-event-1".to_owned());
    let mut tampered_order_source = row;
    tampered_order_source.source_order_event_id = Some("long-order".to_owned());
    let mut replay = SqlLedgerReplay::with_limit(10);

    for event in [tampered_event_source, tampered_order_source] {
        push_replay_run_finality_result(&mut replay, validate_run_finality_replay_event(event));
    }

    assert!(replay.run_finality_events.is_empty());
    assert_eq!(replay.health.run_finality_decode_failures, 2);
}

fn set_legacy_event_id(row: &mut SqlRunFinalityReplayEvent) {
    row.event_id = legacy_run_finality_event_id(&RunFinalityEventIdInput {
        run_kind: &row.run_kind,
        run_id: &row.run_id,
        state: &row.state,
        source: &row.source,
        source_event_id: row.source_event_id.as_deref(),
        source_order_event_id: row.source_order_event_id.as_deref(),
        occurred_at_ms: row.occurred_at_ms,
        payload_hash: &row.payload_hash,
    });
}

fn assert_latest_replay_query(sql: &str, newest: &str, ascending: &str, alias: &str) {
    assert!(sql.contains(newest));
    assert!(sql.contains(ascending));
    assert!(sql.contains(alias));
}

fn retained_replay_rows<T>(rows: Vec<T>) -> Vec<T> {
    let skip = newest_replay_rows_to_skip(rows.len());
    rows.into_iter().skip(skip).collect()
}

fn replay_event_from_ledger_event(event: SqlRunFinalityLedgerEvent) -> SqlRunFinalityReplayEvent {
    SqlRunFinalityReplayEvent {
        event_id: event.event_id,
        run_kind: event.run_kind,
        run_id: event.run_id,
        source_event_id: event.source_event_id,
        source_order_event_id: event.source_order_event_id,
        source: event.source,
        state: event.state,
        payload: event.payload,
        payload_hash: event.payload_hash,
        schema_version: SQL_LEDGER_SCHEMA_VERSION as i32,
        occurred_at_ms: event.occurred_at_ms,
        captured_at_ms: event.captured_at_ms,
    }
}
