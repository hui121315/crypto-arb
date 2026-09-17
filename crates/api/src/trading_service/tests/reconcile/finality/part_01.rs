use super::support::*;
use super::*;

#[tokio::test]
async fn reconcile_refreshes_state_mismatch_by_single_order_lookup() {
    let remote = order_info("x1", OrderStatus::PartiallyFilled, 0.01);
    let service = service_with_reconcile_adapter(vec![remote.clone()], Some(remote));
    seed_accepted_order(&service, "state-mismatch", "x1", 0.01);

    let outcome = must_ok(
        service.reconcile_and_refresh_missing_orders().await,
        "state mismatch reconcile",
    );

    assert_eq!(outcome.diffs.len(), 1);
    assert_eq!(
        outcome.diffs[0].kind,
        trading::ReconcileDiffKind::StateMismatch
    );
    assert_eq!(outcome.refreshed.len(), 1);
    assert!(outcome.refresh_failures.is_empty());
    assert_eq!(outcome.refreshed[0].state, LiveOrderState::PartiallyFilled);
    let local = must_some(
        service.get_order("state-mismatch"),
        "state mismatch order should remain local",
    );
    assert_eq!(local.state, LiveOrderState::PartiallyFilled);
    assert_eq!(local.filled_quantity, Some(0.005));
}

#[tokio::test]
async fn refresh_order_state_uses_original_order_exchange() {
    let remote = order_info("x1", OrderStatus::Filled, 0.01);
    let (service, adapter) = service_with_reconcile_adapter_handle(Vec::new(), Some(remote));
    seed_accepted_order(&service, "venue-scoped", "x1", 0.01);

    let updated = must_ok(
        service.refresh_order_state("venue-scoped").await,
        "venue-scoped refresh",
    );

    assert_eq!(
        must_some(updated, "order should refresh").state,
        LiveOrderState::Filled
    );
    assert_eq!(adapter.exchange_order_queries(), vec!["mock".to_owned()]);
}

#[tokio::test]
async fn refresh_order_state_prefers_numeric_exchange_order_id() {
    let remote = order_info("x1", OrderStatus::Filled, 0.01);
    let (service, adapter) = service_with_reconcile_adapter_handle(Vec::new(), Some(remote));
    seed_accepted_order_with_venue_client_id(&service, "exchange-id", "777", "t-venue-c1", 0.01);

    let updated = must_ok(
        service.refresh_order_state("exchange-id").await,
        "exchange id refresh",
    );

    assert_eq!(
        must_some(updated, "order should refresh").state,
        LiveOrderState::Filled
    );
    assert_eq!(adapter.exchange_order_id_queries(), vec!["777".to_owned()]);
    assert!(adapter.exchange_order_query_ids().is_empty());
}

#[tokio::test]
async fn refresh_order_state_falls_back_to_venue_client_id_without_exchange_id() {
    let remote = order_info("x1", OrderStatus::Filled, 0.01);
    let (service, adapter) = service_with_reconcile_adapter_handle(Vec::new(), Some(remote));
    seed_accepted_order_with_venue_client_id(&service, "venue-client", "", "t-venue-c1", 0.01);

    let updated = must_ok(
        service.refresh_order_state("venue-client").await,
        "venue client refresh",
    );

    assert_eq!(
        must_some(updated, "order should refresh").state,
        LiveOrderState::Filled
    );
    assert_eq!(
        adapter.exchange_order_query_ids(),
        vec!["t-venue-c1".to_owned()]
    );
    assert!(adapter.exchange_order_id_queries().is_empty());
}

#[test]
fn get_order_by_exchange_order_id_uses_journal_index() {
    let service = TradingService::new_mock();
    seed_accepted_order(&service, "exchange-index", "x-index", 0.01);

    let record = must_some(
        service.get_order_by_exchange_order_id("x-index"),
        "exchange order id should locate local record",
    );

    assert_eq!(record.intent.id, "exchange-index");
}

#[tokio::test]
async fn live_submit_records_order_proof_place_ack() {
    let (service, _adapter) = service_with_reconcile_adapter_handle(Vec::new(), None);
    service.update_risk_config(|config| config.live_trading_enabled = true);
    let mut intent = limit_intent("proof-submit");
    intent.mode = ExecutionMode::Live;

    let record = must_ok(service.submit(intent).await, "live proof submit");
    let rows = service
        .live_order_proof_health
        .snapshot(common::time::now_ms());
    let proof = must_some(
        rows.into_iter().find(|row| row.venue == "mock"),
        "live submit proof row",
    );

    assert_eq!(record.state, LiveOrderState::Accepted);
    assert_eq!(proof.status, shared_types::VenueOperationStatus::Warn);
    assert_eq!(proof.place_ack_count, 1);
    assert_eq!(proof.cancel_requested_count, 0);
    assert_eq!(proof.cancel_finality_count, 0);
    assert_eq!(proof.rows, Some(1));
    assert_eq!(
        proof
            .place_proof
            .as_ref()
            .map(|sample| sample.source.as_str()),
        Some("adapter_ack")
    );
}

#[tokio::test]
async fn cancel_refreshes_cancel_requested_to_cancelled_when_order_query_confirms() {
    let remote = order_info("x1", OrderStatus::Canceled, 0.01);
    let (service, adapter) = service_with_reconcile_adapter_handle(Vec::new(), Some(remote));
    seed_accepted_order(&service, "cancel-finality", "x1", 0.01);

    let record = must_ok(service.cancel("cancel-finality").await, "cancel finality");

    assert_eq!(record.state, LiveOrderState::Cancelled);
    assert_eq!(adapter.exchange_order_queries(), vec!["mock".to_owned()]);
    assert_eq!(service.open_order_count(), 0);
}

#[tokio::test]
async fn live_cancel_order_query_finality_records_order_proof() {
    let remote = order_info("x1", OrderStatus::Canceled, 0.01);
    let (service, _adapter) = service_with_reconcile_adapter_handle(Vec::new(), Some(remote));
    service.update_risk_config(|config| config.live_trading_enabled = true);
    let mut intent = limit_intent("proof-cancel");
    intent.mode = ExecutionMode::Live;
    must_ok(
        service.submit(intent).await,
        "live proof submit before cancel",
    );

    let record = must_ok(service.cancel("proof-cancel").await, "live proof cancel");
    let rows = service
        .live_order_proof_health
        .snapshot(common::time::now_ms());
    let proof = must_some(
        rows.into_iter().find(|row| row.venue == "mock"),
        "live cancel proof row",
    );

    assert_eq!(record.state, LiveOrderState::Cancelled);
    assert_eq!(proof.status, shared_types::VenueOperationStatus::Ok);
    assert_eq!(proof.place_ack_count, 1);
    assert_eq!(proof.cancel_requested_count, 1);
    assert_eq!(proof.cancel_finality_count, 1);
    assert_eq!(
        proof
            .cancel_finality
            .as_ref()
            .map(|sample| sample.source.as_str()),
        Some("order_query")
    );
}

#[tokio::test]
async fn cancel_keeps_cancel_requested_when_order_query_returns_none() {
    let (service, adapter) = service_with_reconcile_adapter_handle(Vec::new(), None);
    seed_accepted_order(&service, "cancel-pending", "x1", 0.01);

    let record = must_ok(service.cancel("cancel-pending").await, "cancel pending");

    assert_eq!(record.state, LiveOrderState::CancelRequested);
    assert_eq!(adapter.exchange_order_queries(), vec!["mock".to_owned()]);
    assert_eq!(service.open_order_count(), 1);
}

#[tokio::test]
async fn cancel_keeps_cancel_requested_when_finality_probe_errors() {
    let service = service_with_reconcile_adapter_error(Vec::new(), "order query timeout");
    seed_accepted_order(&service, "cancel-probe-error", "x1", 0.01);

    let record = must_ok(
        service.cancel("cancel-probe-error").await,
        "cancel probe error",
    );

    assert_eq!(record.state, LiveOrderState::CancelRequested);
    assert_eq!(service.open_order_count(), 1);
}

#[test]
fn live_remote_cancel_blocks_when_configured_audit_sink_unhealthy() {
    let record = cancel_guard_record(ExecutionMode::Live, LiveOrderState::Accepted);
    let snapshot = audit_health_snapshot(true, false, Some("open_failed: denied"));

    let error = must_err(
        ensure_remote_cancel_audit_snapshot(&record, &snapshot),
        "live cancel audit guard",
    );

    assert!(
        matches!(&error, trading::TradingError::AuditLogUnavailable { .. }),
        "expected audit error, got {error:?}"
    );
    let reason = match error {
        trading::TradingError::AuditLogUnavailable { reason } => reason,
        _ => String::new(),
    };
    assert!(reason.contains("not open"), "reason: {reason}");
    assert!(reason.contains("open_failed"), "reason: {reason}");
}

#[test]
fn testnet_remote_cancel_ignores_unhealthy_audit_sink() {
    let record = cancel_guard_record(ExecutionMode::Testnet, LiveOrderState::Accepted);
    let snapshot = audit_health_snapshot(true, false, Some("open_failed: denied"));

    must_ok(
        ensure_remote_cancel_audit_snapshot(&record, &snapshot),
        "testnet cancel audit guard",
    );
}

#[test]
fn terminal_live_cancel_does_not_require_audit_sink() {
    let record = cancel_guard_record(ExecutionMode::Live, LiveOrderState::Filled);
    let snapshot = audit_health_snapshot(true, false, Some("open_failed: denied"));

    must_ok(
        ensure_remote_cancel_audit_snapshot(&record, &snapshot),
        "terminal live cancel audit guard",
    );
}
