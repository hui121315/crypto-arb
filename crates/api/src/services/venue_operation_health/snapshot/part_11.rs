fn nav_storage_status(health: &NavStorageHealth, now_ms: i64) -> VenueOperationStatus {
    if !health.enabled {
        return VenueOperationStatus::Warn;
    }
    if nav_storage_recent_unrecovered_error(health, now_ms) {
        return VenueOperationStatus::Blocked;
    }
    if health
        .latest_sample_status
        .as_deref()
        .is_some_and(|status| status == crate::lifecycle::nav_persist::NAV_SAMPLE_STATUS_UNKNOWN)
    {
        return VenueOperationStatus::Warn;
    }
    if health.error_total() > 0 {
        return VenueOperationStatus::Warn;
    }
    VenueOperationStatus::Ok
}

fn execution_ledger_storage_status(
    snapshot: &ExecutionLedgerStorageSnapshot,
) -> VenueOperationStatus {
    if !snapshot.configured {
        return VenueOperationStatus::Warn;
    }
    if snapshot.replay_failures > 0 || snapshot.append_failures > 0 {
        return VenueOperationStatus::Blocked;
    }
    if snapshot.query_failures > 0 {
        return VenueOperationStatus::Warn;
    }
    VenueOperationStatus::Ok
}

fn execution_ledger_storage_message(snapshot: &ExecutionLedgerStorageSnapshot) -> String {
    let path = snapshot.path.as_deref().unwrap_or("未配置");
    if !snapshot.configured {
        return "ExecutionLedger JSONL 未配置，复盘/PNL 只能读取进程内事实".to_owned();
    }
    format!(
        "ExecutionLedger JSONL path={path}，events={}，replayed={}，append_ok={}，append_failed={}，replay_failed={}，query_ok={}，query_failed={}",
        snapshot.event_count,
        snapshot.replayed_events,
        snapshot.append_successes,
        snapshot.append_failures,
        snapshot.replay_failures,
        snapshot.query_successes,
        snapshot.query_failures
    )
}

fn execution_ledger_storage_problem(
    snapshot: &ExecutionLedgerStorageSnapshot,
    status: VenueOperationStatus,
    message: &str,
) -> Option<ApiProblem> {
    if status == VenueOperationStatus::Ok {
        return None;
    }
    let code = if !snapshot.configured {
        codes::EXECUTION_LEDGER_STORAGE_UNAVAILABLE
    } else if snapshot.replay_failures > 0 || snapshot.append_failures > 0 {
        codes::EXECUTION_LEDGER_STORAGE_IO_FAILED
    } else {
        codes::EXECUTION_LEDGER_QUERY_FAILED
    };
    let mut problem =
        ApiProblem::new(code, message.to_owned()).with_source(SOURCE_EXECUTION_LEDGER_STORAGE);
    problem.details = Some(serde_json::json!({
        "path": snapshot.path.as_deref(),
        "configured": snapshot.configured,
        "eventCount": snapshot.event_count,
        "replayedEvents": snapshot.replayed_events,
        "replayFailures": snapshot.replay_failures,
        "appendSuccesses": snapshot.append_successes,
        "appendFailures": snapshot.append_failures,
        "lastAppendAtMs": snapshot.last_append_at_ms,
        "querySuccesses": snapshot.query_successes,
        "queryFailures": snapshot.query_failures,
        "lastQueryAtMs": snapshot.last_query_at_ms,
    }));
    Some(problem)
}

fn execution_ledger_storage_evidence(
    snapshot: &ExecutionLedgerStorageSnapshot,
) -> VenueOperationEvidence {
    let mut request_context = vec![
        format!("configured={}", snapshot.configured),
        format!("event_count={}", snapshot.event_count),
        format!("replayed_events={}", snapshot.replayed_events),
        format!("replay_failures={}", snapshot.replay_failures),
        format!("append_successes={}", snapshot.append_successes),
        format!("append_failures={}", snapshot.append_failures),
        format!("query_successes={}", snapshot.query_successes),
        format!("query_failures={}", snapshot.query_failures),
    ];
    if let Some(path) = snapshot.path.as_deref() {
        request_context.push(format!("path={path}"));
    }
    VenueOperationEvidence {
        method: "jsonl".to_owned(),
        path: "execution_ledger_events".to_owned(),
        checked_at: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        doc_version: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        schema_hash: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        fixture_id: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        parser_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_builder_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        auth_kind: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_id: None,
        request_context,
        doc_urls: Vec::new(),
        use_cases: vec![
            "execution_replay".to_owned(),
            "review_fact_source".to_owned(),
            "portfolio_pnl_fact_source".to_owned(),
        ],
        data_kinds: vec!["order_state".to_owned(), "fill_event".to_owned()],
        rate_scopes: Vec::new(),
        weight: 0,
    }
}

fn execution_ledger_storage_io_total(snapshot: &ExecutionLedgerStorageSnapshot) -> u64 {
    snapshot
        .append_successes
        .saturating_add(snapshot.append_failures)
        .saturating_add(snapshot.replayed_events)
        .saturating_add(snapshot.replay_failures)
        .saturating_add(snapshot.query_successes)
        .saturating_add(snapshot.query_failures) as u64
}

fn execution_ledger_storage_observed_at(snapshot: &ExecutionLedgerStorageSnapshot) -> Option<i64> {
    [snapshot.last_append_at_ms, snapshot.last_query_at_ms]
        .into_iter()
        .flatten()
        .max()
}

fn order_snapshot_storage_status(snapshot: &OrderSnapshotStorageSnapshot) -> VenueOperationStatus {
    if !snapshot.configured {
        return VenueOperationStatus::Warn;
    }
    if snapshot.replay_failures > 0 || snapshot.append_failures > 0 {
        return VenueOperationStatus::Blocked;
    }
    VenueOperationStatus::Ok
}

fn order_snapshot_storage_message(snapshot: &OrderSnapshotStorageSnapshot) -> String {
    let path = snapshot.path.as_deref().unwrap_or("未配置");
    if !snapshot.configured {
        return "OrderSnapshot JSONL 未配置，订单热投影只能读取进程内状态".to_owned();
    }
    format!(
        "OrderSnapshot JSONL path={path}，records={}，replayed={}，append_ok={}，append_failed={}，replay_failed={}",
        snapshot.record_count,
        snapshot.replayed_records,
        snapshot.append_successes,
        snapshot.append_failures,
        snapshot.replay_failures
    )
}

fn order_snapshot_storage_problem(
    snapshot: &OrderSnapshotStorageSnapshot,
    status: VenueOperationStatus,
    message: &str,
) -> Option<ApiProblem> {
    if status == VenueOperationStatus::Ok {
        return None;
    }
    let code = if !snapshot.configured {
        codes::ORDER_SNAPSHOT_STORAGE_UNAVAILABLE
    } else {
        codes::ORDER_SNAPSHOT_STORAGE_IO_FAILED
    };
    let mut problem =
        ApiProblem::new(code, message.to_owned()).with_source(SOURCE_ORDER_SNAPSHOT_STORAGE);
    problem.details = Some(serde_json::json!({
        "path": snapshot.path.as_deref(),
        "configured": snapshot.configured,
        "recordCount": snapshot.record_count,
        "replayedRecords": snapshot.replayed_records,
        "replayFailures": snapshot.replay_failures,
        "appendSuccesses": snapshot.append_successes,
        "appendFailures": snapshot.append_failures,
        "lastAppendAtMs": snapshot.last_append_at_ms,
    }));
    Some(problem)
}

fn order_snapshot_storage_evidence(
    snapshot: &OrderSnapshotStorageSnapshot,
) -> VenueOperationEvidence {
    let mut request_context = vec![
        format!("configured={}", snapshot.configured),
        format!("record_count={}", snapshot.record_count),
        format!("replayed_records={}", snapshot.replayed_records),
        format!("replay_failures={}", snapshot.replay_failures),
        format!("append_successes={}", snapshot.append_successes),
        format!("append_failures={}", snapshot.append_failures),
    ];
    if let Some(path) = snapshot.path.as_deref() {
        request_context.push(format!("path={path}"));
    }
    VenueOperationEvidence {
        method: "jsonl".to_owned(),
        path: "order_snapshots".to_owned(),
        checked_at: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        doc_version: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        schema_hash: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        fixture_id: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        parser_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_builder_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        auth_kind: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_id: None,
        request_context,
        doc_urls: Vec::new(),
        use_cases: vec![
            "open_order_replay".to_owned(),
            "execution_ui_projection".to_owned(),
            "order_reconciliation_projection".to_owned(),
        ],
        data_kinds: vec!["order_record".to_owned(), "order_projection".to_owned()],
        rate_scopes: Vec::new(),
        weight: 0,
    }
}

fn order_snapshot_storage_io_total(snapshot: &OrderSnapshotStorageSnapshot) -> u64 {
    snapshot
        .append_successes
        .saturating_add(snapshot.append_failures)
        .saturating_add(snapshot.replayed_records)
        .saturating_add(snapshot.replay_failures) as u64
}

fn trading_sql_migration_status(health: &SqlLedgerMigrationHealth) -> VenueOperationStatus {
    if !health.configured {
        return VenueOperationStatus::Warn;
    }
    if health.last_error.is_some() || !health.applied {
        return VenueOperationStatus::Blocked;
    }
    VenueOperationStatus::Ok
}
