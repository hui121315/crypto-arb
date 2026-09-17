#[test]
fn private_ws_account_dirty_projects_typed_refetch_problem_and_evidence() {
    let dirty = crate::trading_service::private_ws_events::PrivateAccountDirty::new(
        "binance",
        crate::trading_service::private_ws_events::PrivateAccountScope::Positions,
        "position_delta_requires_rest_refresh",
    );
    let snapshot = PrivateWsRuntimeHealth {
        venue: "binance".to_owned(),
        operation: OP_PRIVATE_WS_ACCOUNT_STREAM,
        status: VenueOperationStatus::Warn,
        message: "venue=binance; scope=positions; reason=position_delta_requires_rest_refresh"
            .to_owned(),
        request_id: Some("req-pr-ar-dirty".to_owned()),
        requested: Some(1),
        rows: Some(0),
        freshness_ms: Some(0),
        retry_after_ms: None,
        error: Some("bounded REST refetch pending".to_owned()),
        observed_at_ms: 10,
        ok_count: 0,
        warn_count: 1,
        blocked_count: 0,
        last_problem: Some("position cache dirty".to_owned()),
        last_problem_at_ms: Some(10),
        account_dirty: Some(dirty),
    };

    let rows = private_ws_runtime_rows(&[credential(true, true)], vec![snapshot], &[], 20);
    let row = rows
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ACCOUNT_STREAM)
        .expect("private ws account stream row");
    let problem = row.problem.as_ref().expect("typed dirty problem");
    let details = problem.details.as_ref().expect("dirty problem details");
    let account_dirty = details
        .get("accountDirty")
        .and_then(serde_json::Value::as_object)
        .expect("account dirty details");
    let context = &row.evidence.as_ref().expect("private ws evidence").request_context;

    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert_eq!(
        account_dirty.get("venue").and_then(serde_json::Value::as_str),
        Some("binance")
    );
    assert_eq!(
        account_dirty.get("scope").and_then(serde_json::Value::as_str),
        Some("positions")
    );
    assert_eq!(
        account_dirty.get("refetch").and_then(serde_json::Value::as_str),
        Some("bounded_rest_on_next_read")
    );
    assert!(context
        .iter()
        .any(|item| item == "account_dirty_venue=binance"));
    assert!(context
        .iter()
        .any(|item| item == "account_dirty_scope=positions"));
    assert!(context
        .iter()
        .any(|item| item == "account_dirty_refetch=bounded_rest_on_next_read"));
}

#[test]
fn completed_account_refetch_clears_current_dirty_warning_only_after_required_scopes() {
    let dirty = crate::trading_service::private_ws_events::PrivateAccountDirty::new(
        "binance",
        crate::trading_service::private_ws_events::PrivateAccountScope::All,
        "account_update_requires_rest_refresh",
    );
    let mut snapshots = vec![PrivateWsRuntimeHealth {
        venue: "binance".to_owned(),
        operation: OP_PRIVATE_WS_ACCOUNT_STREAM,
        status: VenueOperationStatus::Warn,
        message: "bounded REST refetch pending".to_owned(),
        request_id: None,
        requested: Some(1),
        rows: Some(0),
        freshness_ms: Some(5),
        retry_after_ms: None,
        error: Some("bounded REST refetch pending".to_owned()),
        observed_at_ms: 10,
        ok_count: 0,
        warn_count: 1,
        blocked_count: 0,
        last_problem: Some("account cache dirty".to_owned()),
        last_problem_at_ms: Some(10),
        account_dirty: Some(dirty),
    }];
    let balances = vec![fresh_cache("binance", 4, 12)];

    resolve_completed_account_refetches(&mut snapshots, &balances, &[], 20);
    assert_eq!(snapshots[0].status, VenueOperationStatus::Warn);

    let positions = vec![fresh_cache("binance", 2, 15)];
    resolve_completed_account_refetches(&mut snapshots, &balances, &positions, 20);
    let recovered = &snapshots[0];
    assert_eq!(recovered.status, VenueOperationStatus::Ok);
    assert_eq!(recovered.rows, Some(6));
    assert_eq!(recovered.freshness_ms, Some(5));
    assert!(recovered.error.is_none());
    assert!(recovered.account_dirty.is_none());
    assert!(recovered.message.contains("有界 REST 补拉同步"));
    assert_eq!(recovered.warn_count, 1);
    assert_eq!(recovered.last_problem.as_deref(), Some("account cache dirty"));
}

fn fresh_cache(venue: &str, rows: u64, observed_at_ms: i64) -> AccountCacheSnapshot {
    AccountCacheSnapshot {
        venue: venue.to_owned(),
        rows,
        freshness_ms: 0,
        observed_at_ms,
        quality: AccountCacheQuality::Fresh,
    }
}
