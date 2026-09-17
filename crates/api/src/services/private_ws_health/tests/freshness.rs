use super::*;

#[test]
fn connected_session_stays_ok_until_an_explicit_disconnect() {
    let store = PrivateWsHealthStore::default();
    store.record_connected("gate");
    let observed = store
        .snapshot(common::time::now_ms())
        .into_iter()
        .next()
        .expect("row")
        .observed_at_ms;

    let row = store
        .snapshot(observed + TEST_FRESHNESS_WINDOW_MS + 1)
        .into_iter()
        .next()
        .expect("row");

    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert!(row.freshness_ms.unwrap_or_default() > TEST_FRESHNESS_WINDOW_MS);

    store.record_disconnected("gate", "read timeout");
    let disconnected = store
        .snapshot(common::time::now_ms())
        .into_iter()
        .next()
        .expect("row");
    assert_eq!(disconnected.status, VenueOperationStatus::Warn);
    assert!(disconnected.message.contains("read timeout"));
}

#[test]
fn event_driven_stream_evidence_does_not_expire_while_session_owns_liveness() {
    let store = PrivateWsHealthStore::default();
    store.record_events(
        "bitget",
        &[PrivateWsEvent::Positions(
            crate::trading_service::private_ws_events::PrivatePositionsSnapshot {
                venue: "bitget".to_owned(),
                rows: Vec::new(),
            },
        )],
    );
    let observed = store
        .snapshot(common::time::now_ms())
        .into_iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ACCOUNT_STREAM)
        .expect("account stream row")
        .observed_at_ms;

    let row = store
        .snapshot(observed + TEST_FRESHNESS_WINDOW_MS + 1)
        .into_iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ACCOUNT_STREAM)
        .expect("account stream row");

    assert_eq!(row.status, VenueOperationStatus::Ok);
}

#[test]
fn account_cache_ownership_requires_a_connected_subscribed_clean_stream() {
    let store = PrivateWsHealthStore::default();
    store.record_connected("hyperliquid");
    assert!(!store.account_session_owns_cache("hyperliquid"));

    store.record_subscribe_sent("hyperliquid", 13, 13);
    assert!(!store.account_session_owns_cache("hyperliquid"));

    store.record_events(
        "hyperliquid",
        &[PrivateWsEvent::Positions(
            crate::trading_service::private_ws_events::PrivatePositionsSnapshot {
                venue: "hyperliquid".to_owned(),
                rows: Vec::new(),
            },
        )],
    );
    assert!(store.account_session_owns_cache("hyperliquid"));

    store.record_parse_error("hyperliquid", "invalid account payload");
    assert!(!store.account_session_owns_cache("hyperliquid"));
    store.record_text_received("hyperliquid");
    assert!(!store.account_session_owns_cache("hyperliquid"));
    store.record_events(
        "hyperliquid",
        &[PrivateWsEvent::Positions(
            crate::trading_service::private_ws_events::PrivatePositionsSnapshot {
                venue: "hyperliquid".to_owned(),
                rows: Vec::new(),
            },
        )],
    );
    assert!(store.account_session_owns_cache("hyperliquid"));
    store.record_disconnected("hyperliquid", "heartbeat timeout");
    assert!(!store.account_session_owns_cache("hyperliquid"));
}

#[test]
fn dirty_account_stream_cannot_extend_cache_until_recovery_completes() {
    let store = PrivateWsHealthStore::default();
    store.record_connected("hyperliquid");
    store.record_subscribe_sent("hyperliquid", 13, 13);
    store.record_events(
        "hyperliquid",
        &[PrivateWsEvent::Positions(
            crate::trading_service::private_ws_events::PrivatePositionsSnapshot {
                venue: "hyperliquid".to_owned(),
                rows: Vec::new(),
            },
        )],
    );
    let dirty = crate::trading_service::private_ws_events::PrivateAccountDirty::new(
        "hyperliquid",
        crate::trading_service::private_ws_events::PrivateAccountScope::Positions,
        "position_changed",
    );
    store.record_apply_outcome(
        "hyperliquid",
        &PrivateWsApplyOutcome {
            account_cache_dirty: Some(dirty),
            ..PrivateWsApplyOutcome::default()
        },
    );

    assert!(!store.account_session_owns_cache("hyperliquid"));
    store.record_account_cache_refreshed("hyperliquid", PrivateAccountScope::Positions);
    assert!(store.account_session_owns_cache("hyperliquid"));
}

#[test]
fn order_cache_ownership_requires_an_authoritative_snapshot_after_errors() {
    let store = PrivateWsHealthStore::default();
    store.record_connected("hyperliquid");
    store.record_subscribe_sent("hyperliquid", 13, 13);
    assert!(!store.order_session_owns_cache("hyperliquid"));

    let snapshot = || {
        PrivateWsEvent::OpenOrders(
            crate::trading_service::private_ws_events::PrivateOpenOrdersSnapshot {
                venue: "hyperliquid".to_owned(),
                rows: Vec::new(),
            },
        )
    };
    store.record_events("hyperliquid", &[snapshot()]);
    assert!(store.order_session_owns_cache("hyperliquid"));

    store.record_parse_error("hyperliquid", "invalid order payload");
    assert!(!store.order_session_owns_cache("hyperliquid"));
    store.record_text_received("hyperliquid");
    assert!(!store.order_session_owns_cache("hyperliquid"));
    store.record_events("hyperliquid", &[snapshot()]);
    assert!(store.order_session_owns_cache("hyperliquid"));

    store.record_disconnected("hyperliquid", "heartbeat timeout");
    assert!(!store.order_session_owns_cache("hyperliquid"));
}
