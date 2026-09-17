#![allow(clippy::expect_used)]

use super::*;
use shared_types::{ExecutionMode, OrderIntent, OrderSide, OrderSource, OrderType};

#[test]
fn success_records_global_row_when_no_open_orders() {
    let store = ReconciliationHealthStore::default();
    let cycle = ReconciliationHealthCycle::default();

    store.record_success(&cycle, &[], 0, &[]);
    let row = store
        .snapshot(common::time::now_ms())
        .into_iter()
        .next()
        .expect("global row");

    assert_eq!(row.venue, GLOBAL_RECONCILIATION_VENUE);
    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.requested, Some(0));
}

#[test]
fn next_cycle_replaces_stale_venue_rows() {
    let store = ReconciliationHealthStore::default();
    let cycle =
        ReconciliationHealthCycle::from_orders(&[order("binance", LiveOrderState::Accepted)]);
    store.record_failure(&cycle, "network down");

    store.record_success(&ReconciliationHealthCycle::default(), &[], 0, &[]);
    let rows = store.snapshot(common::time::now_ms());

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].venue, GLOBAL_RECONCILIATION_VENUE);
    assert_eq!(rows[0].status, VenueOperationStatus::Ok);
}

#[test]
fn diff_without_refresh_failure_records_warn() {
    let store = ReconciliationHealthStore::default();
    let cycle =
        ReconciliationHealthCycle::from_orders(&[order("binance", LiveOrderState::Accepted)]);

    store.record_success(&cycle, &[], 1, &[]);
    let row = store
        .snapshot(common::time::now_ms())
        .into_iter()
        .next()
        .expect("row");

    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert_eq!(row.requested, Some(1));
    assert_eq!(row.rows, Some(1));
}

#[test]
fn refresh_failure_records_blocked_error() {
    let store = ReconciliationHealthStore::default();
    let cycle =
        ReconciliationHealthCycle::from_orders(&[order("binance", LiveOrderState::Accepted)]);

    store.record_success(
        &cycle,
        &[],
        1,
        &[ReconcileRefreshFailure {
            internal_order_id: "o1".to_owned(),
            venue: "binance".to_owned(),
            error: "timeout".to_owned(),
        }],
    );
    let row = store
        .snapshot(common::time::now_ms())
        .into_iter()
        .next()
        .expect("row");

    assert_eq!(row.status, VenueOperationStatus::Blocked);
    assert!(row.error.as_deref().is_some_and(|err| err.contains("o1")));
}

#[test]
fn failure_records_each_open_order_venue() {
    let store = ReconciliationHealthStore::default();
    let cycle = ReconciliationHealthCycle::from_orders(&[
        order("binance", LiveOrderState::Accepted),
        order("okx", LiveOrderState::PartiallyFilled),
    ]);

    store.record_failure(&cycle, "upstream closed");
    let rows = store.snapshot(common::time::now_ms());

    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .all(|row| row.status == VenueOperationStatus::Blocked));
    assert!(rows.iter().all(|row| row
        .error
        .as_deref()
        .is_some_and(|err| err.contains("upstream"))));
}

#[test]
fn stale_ok_sample_downgrades_to_warn() {
    let store = ReconciliationHealthStore::default();
    let cycle = ReconciliationHealthCycle::default();
    store.record_success(&cycle, &[], 0, &[]);
    let observed = store
        .snapshot(common::time::now_ms())
        .into_iter()
        .next()
        .expect("row")
        .observed_at_ms;

    let row = store
        .snapshot(observed + RECONCILIATION_STALE_MS + 1)
        .into_iter()
        .next()
        .expect("row");

    assert_eq!(row.status, VenueOperationStatus::Warn);
}

fn order(exchange: &str, state: LiveOrderState) -> OrderRecord {
    OrderRecord {
        intent: OrderIntent {
            id: format!("{exchange}-1"),
            source: OrderSource::Manual,
            strategy: None,
            mode: ExecutionMode::Live,
            exchange: exchange.to_owned(),
            symbol: "BTC".to_owned(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            quantity: 1.0,
            price: Some(1.0),
            slippage_tolerance_bps: None,
            reduce_only: false,
            time_in_force: shared_types::TimeInForce::Gtc,
            post_only: false,
            margin_mode: shared_types::MarginMode::Cross,
            leverage: 1.0,
            client_order_id: format!("{exchange}-client"),
            client_order_id_policy: None,
            created_at_ms: 1,
        },
        state,
        risk: None,
        identity: Default::default(),
        last_update_source: Default::default(),
        exchange_order_id: Some(format!("{exchange}-exchange")),
        message: None,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
        updated_at_ms: 2,
    }
}
