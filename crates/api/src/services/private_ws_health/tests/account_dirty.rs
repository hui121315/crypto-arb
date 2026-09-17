use super::super::*;
use crate::trading_service::private_ws_events::{
    PrivateAccountDirty, PrivateAccountScope, PrivateWsApplyOutcome,
};

#[test]
fn account_dirty_stays_healthy_during_bounded_refetch_grace() {
    let store = PrivateWsHealthStore::default();
    let dirty = PrivateAccountDirty::new(
        "bybit",
        PrivateAccountScope::Balances,
        "wallet_delta_requires_rest_refresh",
    );

    store.record_events("bybit", &[PrivateWsEvent::AccountDirty(dirty.clone())]);
    assert!(store.snapshot(common::time::now_ms()).is_empty());
    store.record_apply_outcome(
        "bybit",
        &PrivateWsApplyOutcome {
            account_cache_dirty: Some(dirty),
            ..PrivateWsApplyOutcome::default()
        },
    );
    let row = store
        .snapshot(common::time::now_ms())
        .into_iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ACCOUNT_STREAM)
        .expect("account stream row");

    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.requested, Some(1));
    assert_eq!(row.rows, Some(0));
    assert!(row.error.is_none());
    assert!(row.message.contains("正在后台同步账户快照"));
    assert_eq!(
        row.account_dirty.as_ref().map(|dirty| dirty.scope),
        Some(PrivateAccountScope::Balances)
    );

    store.record_apply_outcome(
        "bybit",
        &PrivateWsApplyOutcome {
            account_cache_updated: true,
            ..PrivateWsApplyOutcome::default()
        },
    );
    let recovered = store
        .snapshot(common::time::now_ms())
        .into_iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ACCOUNT_STREAM)
        .expect("recovered account stream row");
    assert_eq!(recovered.status, VenueOperationStatus::Ok);
    assert!(recovered.account_dirty.is_none());
    assert_eq!(recovered.warn_count, 0);
    assert!(recovered.last_problem.is_none());
}

#[test]
fn account_dirty_becomes_warn_only_after_refetch_grace_expires() {
    let store = PrivateWsHealthStore::default();
    let dirty = PrivateAccountDirty::new(
        "binance",
        PrivateAccountScope::All,
        "account_update_requires_rest_refresh",
    );
    store.record_apply_outcome(
        "binance",
        &PrivateWsApplyOutcome {
            account_cache_dirty: Some(dirty),
            ..PrivateWsApplyOutcome::default()
        },
    );

    let row = store
        .snapshot(common::time::now_ms().saturating_add(ACCOUNT_REFETCH_GRACE_MS + 1))
        .into_iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ACCOUNT_STREAM)
        .expect("account stream row");

    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert_eq!(row.retry_after_ms, Some(2_000));
    assert!(row.error.is_some());
}

#[test]
fn all_scope_dirty_clears_after_both_account_caches_refresh() {
    let store = PrivateWsHealthStore::default();
    let dirty = PrivateAccountDirty::new(
        "binance",
        PrivateAccountScope::All,
        "account_update_requires_rest_refresh",
    );
    store.record_apply_outcome(
        "binance",
        &PrivateWsApplyOutcome {
            account_cache_dirty: Some(dirty),
            ..PrivateWsApplyOutcome::default()
        },
    );

    store.record_account_cache_refreshed("binance", PrivateAccountScope::Balances);
    let partial = store
        .snapshot(common::time::now_ms())
        .into_iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ACCOUNT_STREAM)
        .expect("partial account refresh row");
    assert_eq!(partial.status, VenueOperationStatus::Ok);
    assert_eq!(
        partial.account_dirty.map(|dirty| dirty.scope),
        Some(PrivateAccountScope::Positions)
    );

    store.record_account_cache_refreshed("binance", PrivateAccountScope::Positions);
    let complete = store
        .snapshot(common::time::now_ms())
        .into_iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ACCOUNT_STREAM)
        .expect("complete account refresh row");
    assert_eq!(complete.status, VenueOperationStatus::Ok);
    assert!(complete.account_dirty.is_none());
    assert!(complete.error.is_none());
    assert!(complete.message.contains("已通过后台快照同步"));
}
