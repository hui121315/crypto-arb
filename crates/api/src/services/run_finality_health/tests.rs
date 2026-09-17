use super::*;

#[test]
fn record_outcome_projects_per_venue_counts() {
    let store = RunFinalityHealthStore::default();
    let mut outcome = RunFinalityOutcome::default();
    outcome.venue_outcomes.insert(
        "binance".to_owned(),
        RunFinalityVenueOutcome {
            scanned_order_count: 2,
            refreshed_order_count: 1,
            remote_missing_count: 1,
            skipped_terminal_count: 0,
            refresh_failure_count: 0,
            publish_failure_count: 0,
            sample_problem: Some(RunFinalitySampleProblem {
                raw_order_id: "exchange-1".to_owned(),
                internal_order_id: "internal-1".to_owned(),
                venue: "binance".to_owned(),
                source: "ExecutionRun".to_owned(),
                order_state: shared_types::LiveOrderState::Accepted,
                code: shared_types::problem::codes::HEDGE_ORDER_FINALITY_FAILED.to_owned(),
                message: "订单终态回查未找到远端订单".to_owned(),
                status: Some(409),
                checked_at_ms: 42,
                error: None,
            }),
        },
    );

    store.record_outcome(&outcome);
    let rows = store.snapshot(common::time::now_ms());
    assert_eq!(rows.len(), 1);
    let row = &rows[0];

    assert_eq!(row.venue, "binance");
    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert_eq!(row.requested, Some(2));
    assert_eq!(row.rows, Some(1));
    assert_eq!(row.remote_missing_count, 1);
    assert!(row
        .error
        .as_deref()
        .is_some_and(|error| error.contains("远端缺失 1")));
    assert!(row
        .sample_problem
        .as_ref()
        .is_some_and(|sample| sample.raw_order_id == "exchange-1"));
    assert!(row.message.contains("样本订单 exchange-1"));
}

#[test]
fn failure_counts_block_finality_row() {
    let store = RunFinalityHealthStore::default();
    let mut outcome = RunFinalityOutcome::default();
    outcome.venue_outcomes.insert(
        "okx".to_owned(),
        RunFinalityVenueOutcome {
            scanned_order_count: 1,
            refresh_failure_count: 1,
            ..RunFinalityVenueOutcome::default()
        },
    );

    store.record_outcome(&outcome);
    let rows = store.snapshot(common::time::now_ms());
    assert_eq!(rows.len(), 1);
    let row = &rows[0];

    assert_eq!(row.status, VenueOperationStatus::Blocked);
    assert_eq!(row.rows, Some(0));
    assert!(row
        .error
        .as_deref()
        .is_some_and(|error| error.contains("刷新失败 1")));
}

#[test]
fn next_cycle_replaces_stale_venues_with_global_empty_row() {
    let store = RunFinalityHealthStore::default();
    let mut outcome = RunFinalityOutcome::default();
    outcome.venue_outcomes.insert(
        "okx".to_owned(),
        RunFinalityVenueOutcome {
            scanned_order_count: 1,
            refreshed_order_count: 1,
            ..RunFinalityVenueOutcome::default()
        },
    );
    store.record_outcome(&outcome);

    store.record_outcome(&RunFinalityOutcome::default());
    let rows = store.snapshot(common::time::now_ms());

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].venue, GLOBAL_RUN_FINALITY_VENUE);
    assert_eq!(rows[0].status, VenueOperationStatus::Ok);
    assert_eq!(rows[0].requested, Some(0));
}

#[test]
fn stale_ok_sample_downgrades_to_warn() {
    let store = RunFinalityHealthStore::default();
    store.record_outcome(&RunFinalityOutcome::default());
    let current_rows = store.snapshot(common::time::now_ms());
    assert_eq!(current_rows.len(), 1);
    let observed = current_rows[0].observed_at_ms;

    let stale_rows = store.snapshot(observed + RUN_FINALITY_STALE_MS + 1);
    assert_eq!(stale_rows.len(), 1);
    let row = &stale_rows[0];

    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert!(row
        .error
        .as_deref()
        .is_some_and(|error| error.contains("变旧")));
}

#[test]
fn credential_update_invalidation_removes_exact_and_family_finality_rows() {
    let store = RunFinalityHealthStore::default();
    let mut outcome = RunFinalityOutcome::default();
    outcome.venue_outcomes.insert(
        "okx".to_owned(),
        RunFinalityVenueOutcome {
            scanned_order_count: 1,
            refreshed_order_count: 1,
            ..RunFinalityVenueOutcome::default()
        },
    );
    outcome.venue_outcomes.insert(
        "binance".to_owned(),
        RunFinalityVenueOutcome {
            scanned_order_count: 1,
            refreshed_order_count: 1,
            ..RunFinalityVenueOutcome::default()
        },
    );
    outcome.venue_outcomes.insert(
        "hyperliquid:xyz".to_owned(),
        RunFinalityVenueOutcome {
            scanned_order_count: 1,
            refreshed_order_count: 1,
            ..RunFinalityVenueOutcome::default()
        },
    );
    store.record_outcome(&outcome);

    store.invalidate_credentials_update(" OKX ");
    let after_okx = venue_keys(store.snapshot(common::time::now_ms()));
    assert!(!after_okx.iter().any(|venue| venue == "okx"));
    assert!(after_okx.iter().any(|venue| venue == "binance"));
    assert!(after_okx.iter().any(|venue| venue == "hyperliquid:xyz"));

    store.invalidate_credentials_update("hyperliquid");
    let after_hyperliquid = venue_keys(store.snapshot(common::time::now_ms()));
    assert_eq!(after_hyperliquid, vec!["binance"]);
}

fn venue_keys(mut rows: Vec<RunFinalityRuntimeHealth>) -> Vec<String> {
    rows.sort_unstable_by(|left, right| left.venue.cmp(&right.venue));
    rows.into_iter()
        .map(|row| normalized_venue_name(&row.venue))
        .collect()
}
