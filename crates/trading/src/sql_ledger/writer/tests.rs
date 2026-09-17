use super::lifecycle::{ShutdownStart, WriterControl};
use super::protocol::{
    classify_event_write, classify_postgres_failure, retry_decision, EventWriteError,
    PostgresFailure, RetryDecision,
};
use super::*;

#[test]
fn fresh_insert_is_committed_ack() {
    let row = event_row();
    assert!(matches!(
        classify_event_write(true, &row, &row),
        Ok(SqlLedgerPersistAck::Committed)
    ));
}

#[test]
fn matching_identity_and_payload_hash_is_idempotent_ack() {
    let row = event_row();
    assert!(matches!(
        classify_event_write(false, &row, &row),
        Ok(SqlLedgerPersistAck::AlreadyPersisted)
    ));
}

#[test]
fn changed_payload_hash_or_identity_is_integrity_conflict() {
    let row = event_row();
    let mut changed_payload = row.clone();
    changed_payload.payload = serde_json::json!({"eventId": "evt-1", "changed": true});
    let mut changed_hash = row.clone();
    changed_hash.payload_hash = "fnv1a64:0000000000000000".to_owned();
    let mut changed_identity = row.clone();
    changed_identity.exchange = "different".to_owned();
    for requested in [&changed_payload, &changed_hash, &changed_identity] {
        assert!(matches!(
            classify_event_write(false, &row, requested),
            Err(EventWriteError::IntegrityConflict { .. })
        ));
    }
}

#[test]
fn retry_classification_is_transient_and_bounded() {
    assert_eq!(
        classify_postgres_failure(false, Some("40001")),
        PostgresFailure::Retryable
    );
    assert_eq!(
        classify_postgres_failure(false, Some("40P01")),
        PostgresFailure::Retryable
    );
    assert_eq!(
        classify_postgres_failure(false, Some("23505")),
        PostgresFailure::Terminal
    );
    assert_eq!(
        retry_decision(1, PostgresFailure::Retryable),
        RetryDecision::Retry
    );
    assert_eq!(
        retry_decision(2, PostgresFailure::Retryable),
        RetryDecision::Retry
    );
    assert_eq!(
        retry_decision(3, PostgresFailure::Retryable),
        RetryDecision::Stop
    );
}

#[test]
fn closed_and_class_08_connections_fail_without_same_client_retry() {
    assert_eq!(
        classify_postgres_failure(true, Some("40001")),
        PostgresFailure::Terminal
    );
    for sqlstate in ["08000", "08003", "08006", "08P01", "57P01"] {
        assert_eq!(
            classify_postgres_failure(false, Some(sqlstate)),
            PostgresFailure::Terminal,
            "unexpected retry for {sqlstate}"
        );
    }
}

#[tokio::test]
async fn durable_enqueue_waits_for_momentary_capacity() {
    let (sender, mut receiver) = mpsc::channel(1);
    let control = Arc::new(WriterControl::new());
    assert!(sender.try_send(drain_write()).is_ok());
    let queued_control = Arc::clone(&control);
    let queued_sender = sender.clone();
    let queued =
        tokio::spawn(async move { queued_control.send(&queued_sender, drain_write()).await });

    tokio::task::yield_now().await;
    assert!(!queued.is_finished());
    assert!(matches!(
        receiver.recv().await,
        Some(SqlLedgerWrite::Drain(_))
    ));
    assert_eq!(queued.await.expect("enqueue task"), Ok(()));
    assert!(matches!(
        receiver.recv().await,
        Some(SqlLedgerWrite::Drain(_))
    ));
}

#[test]
fn drain_reopens_but_stopped_writer_rejects_stale_clones() {
    let (sender, _receiver) = mpsc::channel(1);
    let control = WriterControl::new();
    let drain = control.begin_drain().expect("begin drain");
    assert!(matches!(
        control.try_send(&sender, drain_write()),
        Err(SqlLedgerWriteError::QueueClosed)
    ));
    assert_eq!(control.begin_shutdown(), ShutdownStart::WaitForDrain);
    drop(drain);
    assert!(control.try_send(&sender, drain_write()).is_ok());
    assert_eq!(control.begin_shutdown(), ShutdownStart::Lead);
    assert_eq!(control.begin_shutdown(), ShutdownStart::WaitForStop);
    control.mark_stopped();
    assert_eq!(control.begin_shutdown(), ShutdownStart::Done);
    assert!(matches!(
        control.try_send(&sender, drain_write()),
        Err(SqlLedgerWriteError::QueueClosed)
    ));
}

#[tokio::test]
async fn cancelled_drain_and_unqueued_shutdown_restore_running_state() {
    let (sender, mut receiver) = mpsc::channel(1);
    let control = WriterControl::new();

    let drain = async {
        let _guard = control.begin_drain().expect("begin drain");
        std::future::pending::<()>().await;
    };
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(1), drain)
            .await
            .is_err()
    );
    assert!(control.try_send(&sender, drain_write()).is_ok());
    assert!(receiver.recv().await.is_some());

    assert_eq!(control.begin_shutdown(), ShutdownStart::Lead);
    let shutdown = async {
        let _guard = control.shutdown_leader_guard();
        std::future::pending::<()>().await;
    };
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(1), shutdown)
            .await
            .is_err()
    );
    assert!(control.try_send(&sender, drain_write()).is_ok());
}

#[test]
fn enqueued_shutdown_cancellation_keeps_writer_stopping() {
    let control = WriterControl::new();
    assert_eq!(control.begin_shutdown(), ShutdownStart::Lead);
    let mut leader = control.shutdown_leader_guard();
    leader.command_enqueued();
    drop(leader);

    assert_eq!(control.begin_shutdown(), ShutdownStart::WaitForStop);
}

#[test]
fn grouped_failure_counter_increments_for_every_event() {
    let stats = SqlLedgerWriteStats::default();
    let result = Err(EventWriteError::Encoding("bad group".to_owned()));

    super::protocol::record_event_group_result(&stats, 3, &result);

    assert_eq!(
        stats
            .event_append_failures
            .load(std::sync::atomic::Ordering::Acquire),
        3
    );
}

fn drain_write() -> SqlLedgerWrite {
    let (ack, _) = oneshot::channel();
    SqlLedgerWrite::Drain(ack)
}

fn event_row() -> super::super::SqlEventRow {
    super::super::SqlEventRow {
        event_id: "evt-1".to_owned(),
        internal_order_id: "ord-1".to_owned(),
        client_order_id: "client-1".to_owned(),
        exchange_order_id: Some("exchange-1".to_owned()),
        public_client_order_id: "client-1".to_owned(),
        venue_client_order_id: None,
        run_id: Some("run-1".to_owned()),
        ticket_id: Some("ticket-1".to_owned()),
        leg_role: Some("long".to_owned()),
        exchange: "mock".to_owned(),
        symbol: "BTC".to_owned(),
        side: "buy".to_owned(),
        order_ref: serde_json::json!({"internalOrderId": "ord-1"}),
        event_type: "order_state".to_owned(),
        source: "private_ws".to_owned(),
        state: Some("accepted".to_owned()),
        lifecycle_event: None,
        payload: serde_json::json!({"eventId": "evt-1"}),
        payload_text: None,
        payload_hash: "fnv1a64:1234567890abcdef".to_owned(),
        schema_version: super::super::SQL_LEDGER_SCHEMA_VERSION as i32,
        occurred_at_ms: 9,
        captured_at_ms: 10,
    }
}
