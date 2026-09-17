#![allow(clippy::expect_used)]

use super::*;
use crate::trading_service::private_ws_events::{PrivateFillDelta, PrivateWsApplyOutcome};

mod account_dirty;
mod freshness;

#[test]
fn subscribe_failure_is_blocked() {
    let store = PrivateWsHealthStore::default();

    store.record_subscribe_failed("okx", 3, 1, "closed");
    let row = store
        .snapshot(common::time::now_ms())
        .into_iter()
        .find(|row| row.operation == OP_PRIVATE_WS_SUBSCRIBE)
        .expect("subscribe row");

    assert_eq!(row.status, VenueOperationStatus::Blocked);
    assert_eq!(row.requested, Some(3));
    assert_eq!(row.rows, Some(1));
    assert!(row
        .error
        .as_deref()
        .is_some_and(|err| err.contains("closed")));
}

#[tokio::test]
async fn subscribe_failure_captures_scoped_request_id() {
    let store = PrivateWsHealthStore::default();

    common::request_id::scope("req-private-ws-1".to_owned(), async {
        store.record_subscribe_failed("okx", 3, 1, "closed");
    })
    .await;
    let row = store
        .snapshot(common::time::now_ms())
        .into_iter()
        .find(|row| row.operation == OP_PRIVATE_WS_SUBSCRIBE)
        .expect("subscribe row");

    assert_eq!(row.request_id.as_deref(), Some("req-private-ws-1"));
}

#[test]
fn subscribe_build_failure_preserves_payload_error() {
    let store = PrivateWsHealthStore::default();

    store.record_subscribe_build_failed("hyperliquid", 6, 2, "invalid user address");
    let row = store
        .snapshot(common::time::now_ms())
        .into_iter()
        .find(|row| row.operation == OP_PRIVATE_WS_SUBSCRIBE)
        .expect("subscribe row");

    assert_eq!(row.status, VenueOperationStatus::Blocked);
    assert_eq!(row.requested, Some(6));
    assert_eq!(row.rows, Some(2));
    assert_eq!(row.error.as_deref(), Some("invalid user address"));
}

#[test]
fn subscribe_sent_records_streams_waiting_for_events() {
    let store = PrivateWsHealthStore::default();

    store.record_subscribe_sent("okx", 2, 2);
    let rows = store.snapshot(common::time::now_ms());
    let order_stream = rows
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ORDER_STREAM)
        .expect("order stream row");
    let account_stream = rows
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ACCOUNT_STREAM)
        .expect("account stream row");

    assert_eq!(order_stream.status, VenueOperationStatus::Unknown);
    assert_eq!(account_stream.status, VenueOperationStatus::Unknown);
    assert_eq!(order_stream.rows, Some(0));
    assert_eq!(account_stream.rows, Some(0));
    assert!(order_stream.message.contains("等待订单流事件样本"));
    assert!(account_stream.message.contains("等待账户流事件样本"));
}

#[test]
fn gate_subscription_requires_every_server_ack() {
    let store = PrivateWsHealthStore::default();

    store.record_subscribe_attempt("gate", 4);
    store.record_subscribe_sent_pending_ack("gate", 4, 4);
    for channel in ["futures.orders", "futures.positions", "futures.balances"] {
        store.record_subscribe_ack("gate", 4, channel, None);
    }
    let partial = store
        .snapshot(common::time::now_ms())
        .into_iter()
        .find(|row| row.operation == OP_PRIVATE_WS_SUBSCRIBE)
        .expect("subscribe row");
    assert_eq!(partial.status, VenueOperationStatus::Unknown);
    assert_eq!(partial.requested, Some(4));
    assert_eq!(partial.rows, Some(3));

    store.record_subscribe_ack("gate", 4, "futures.usertrades", Some("gate-trace-4"));
    let rows = store.snapshot(common::time::now_ms());
    let subscribed = rows
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_SUBSCRIBE)
        .expect("subscribe row");
    assert_eq!(subscribed.status, VenueOperationStatus::Ok);
    assert_eq!(subscribed.rows, Some(4));
    assert_eq!(subscribed.request_id.as_deref(), Some("gate-trace-4"));
    assert!(rows.iter().any(|row| {
        row.operation == OP_PRIVATE_WS_ORDER_STREAM
            && row.status == VenueOperationStatus::Unknown
            && row.rows == Some(0)
    }));
    assert!(rows.iter().any(|row| {
        row.operation == OP_PRIVATE_WS_ACCOUNT_STREAM
            && row.status == VenueOperationStatus::Unknown
            && row.rows == Some(0)
    }));
}

#[test]
fn gate_auth_rejection_keeps_error_and_retry_context() {
    let store = PrivateWsHealthStore::default();

    store.record_subscribe_attempt("gate", 4);
    store.record_subscribe_ack("gate", 4, "futures.positions", None);
    store.record_subscribe_rejected(
        "gate",
        4,
        "futures.orders",
        "code=4; message=authentication fail",
        Some("gate-auth-trace"),
    );
    store.record_auth_failed("gate", "code=4; message=authentication fail");

    let rows = store.snapshot(common::time::now_ms());
    for operation in [OP_PRIVATE_WS_SESSION, OP_PRIVATE_WS_SUBSCRIBE] {
        let row = rows
            .iter()
            .find(|row| row.operation == operation)
            .expect("gate health row");
        assert_eq!(row.status, VenueOperationStatus::Blocked);
        assert_eq!(row.retry_after_ms, Some(PRIVATE_WS_RETRY_AFTER_MS));
        assert_eq!(
            row.error.as_deref(),
            Some("code=4; message=authentication fail")
        );
    }
    let subscribe = rows
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_SUBSCRIBE)
        .expect("subscribe row");
    assert_eq!(subscribe.requested, Some(4));
    assert_eq!(subscribe.rows, Some(1));
    assert_eq!(subscribe.request_id.as_deref(), Some("gate-auth-trace"));
}

#[test]
fn order_event_overwrites_waiting_order_stream() {
    let store = PrivateWsHealthStore::default();

    store.record_subscribe_sent("binance", 1, 1);
    store.record_events(
        "binance",
        &[PrivateWsEvent::Fill(PrivateFillDelta {
            venue: "binance".to_owned(),
            exchange_order_id: "ex-1".to_owned(),
            client_order_id: None,
            symbol: Some("BTCUSDT".to_owned()),
            side: None,
            venue_event_id: "fill-1".to_owned(),
            quantity: 1.0,
            price: 100.0,
            fee_amount: None,
            fee_currency: None,
            occurred_at_ms: 10,
        })],
    );
    let row = store
        .snapshot(common::time::now_ms())
        .into_iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ORDER_STREAM)
        .expect("order stream row");

    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.rows, Some(1));
}

#[test]
fn balance_ledger_update_records_account_stream() {
    let store = PrivateWsHealthStore::default();

    store.record_apply_outcome(
        "okx",
        &PrivateWsApplyOutcome {
            balance_ledger_updated: true,
            ..PrivateWsApplyOutcome::default()
        },
    );
    let row = store
        .snapshot(common::time::now_ms())
        .into_iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ACCOUNT_STREAM)
        .expect("account stream row");

    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.rows, Some(1));
}

#[test]
fn ledger_apply_success_is_recorded_only_after_durable_ack() {
    let store = PrivateWsHealthStore::default();
    let outcome = PrivateWsApplyOutcome {
        ledger_updated: true,
        ..PrivateWsApplyOutcome::default()
    };

    assert!(store.snapshot(common::time::now_ms()).is_empty());
    store.record_apply_outcome("okx", &outcome);

    let row = store
        .snapshot(common::time::now_ms())
        .into_iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ORDER_STREAM)
        .expect("order stream row");
    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.ok_count, 1);
    assert_eq!(row.blocked_count, 0);
}

#[test]
fn ledger_durability_failure_is_blocked_and_not_counted_as_success() {
    let store = PrivateWsHealthStore::default();
    let outcome = PrivateWsApplyOutcome {
        ledger_updated: true,
        ..PrivateWsApplyOutcome::default()
    };

    store.record_apply_durability_failure("okx", &outcome, "commit ACK timed out");

    let row = store
        .snapshot(common::time::now_ms())
        .into_iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ORDER_STREAM)
        .expect("order stream row");
    assert_eq!(row.status, VenueOperationStatus::Blocked);
    assert_eq!(row.ok_count, 0);
    assert_eq!(row.blocked_count, 1);
    assert_eq!(row.error.as_deref(), Some("commit ACK timed out"));
    assert!(row.message.contains("账本持久化失败"));
}

#[test]
fn channel_counters_accumulate_and_keep_last_problem_after_recovery() {
    let store = PrivateWsHealthStore::default();
    store.record_connected("bitget");
    store.record_disconnected("bitget", "read timeout");
    store.record_connected("bitget");

    let row = store
        .snapshot(common::time::now_ms())
        .into_iter()
        .find(|row| row.operation == OP_PRIVATE_WS_SESSION)
        .expect("session row");

    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.ok_count, 2);
    assert_eq!(row.warn_count, 1);
    assert_eq!(row.blocked_count, 0);
    assert_eq!(
        row.last_problem.as_deref(),
        Some("私有 WS 断开：read timeout")
    );
    assert!(row.last_problem_at_ms.is_some());
}
