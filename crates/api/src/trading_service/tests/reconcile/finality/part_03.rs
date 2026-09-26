#[tokio::test]
async fn reconcile_records_single_order_refresh_failure() {
    let service = service_with_reconcile_adapter_error(Vec::new(), "refresh timeout");
    seed_accepted_order(&service, "refresh-fails", "x1", 0.01);

    let outcome = must_ok(
        service.reconcile_and_refresh_missing_orders().await,
        "refresh failure reconcile",
    );

    assert_eq!(outcome.diffs.len(), 1);
    assert!(outcome.refreshed.is_empty());
    assert_eq!(outcome.refresh_failures.len(), 1);
    assert_eq!(
        outcome.refresh_failures[0].internal_order_id,
        "refresh-fails"
    );
    assert_eq!(outcome.refresh_failures[0].venue, "mock");
    assert!(outcome.refresh_failures[0]
        .error
        .contains("refresh timeout"));
}

#[tokio::test]
async fn reconcile_refreshes_unknown_before_global_open_orders_failure() {
    let remote = order_info("venue-order-unknown", OrderStatus::Filled, 0.01);
    let (service, adapter) = service_with_open_orders_error(Some(remote), "open orders timeout");
    seed_unknown_order(&service, "unknown-before-open-orders");

    let error = must_err(
        service.reconcile_and_refresh_missing_orders().await,
        "global open-orders failure remains visible",
    );
    let record = must_some(
        service.get_order("unknown-before-open-orders"),
        "unknown order should remain in the journal",
    );

    assert!(matches!(error, ExchangeError::Network(message) if message == "open orders timeout"));
    assert_eq!(record.state, LiveOrderState::Filled);
    assert_eq!(
        record.exchange_order_id.as_deref(),
        Some("venue-order-unknown")
    );
    assert_eq!(
        adapter.exchange_order_query_ids(),
        vec!["client-unknown-before-open-orders"]
    );
}

#[tokio::test]
async fn reconcile_fails_old_bitget_submit_confirmed_missing_by_client_oid() {
    let service = service_with_reconcile_adapter(Vec::new(), None);
    *service.adapter_name.write() = LIVE_ROUTER_ADAPTER_ID;
    seed_unknown_order_on(
        &service,
        "bitget-missing-old",
        "bitget",
        common::time::now_ms() - 60_001,
    );

    let outcome = must_ok(
        service.reconcile_and_refresh_missing_orders().await,
        "old Bitget missing submit reconciliation",
    );
    let record = must_some(
        service.get_order("bitget-missing-old"),
        "old Bitget missing submit record",
    );

    assert_eq!(record.state, LiveOrderState::Failed);
    assert_eq!(record.last_update_source, OrderUpdateSource::Reconcile);
    assert!(record
        .message
        .as_deref()
        .is_some_and(|message| message.contains("submission was not retried")));
    assert_eq!(outcome.refreshed.len(), 1);
}

#[tokio::test]
async fn reconcile_keeps_recent_bitget_missing_submit_unknown() {
    let service = service_with_reconcile_adapter(Vec::new(), None);
    *service.adapter_name.write() = LIVE_ROUTER_ADAPTER_ID;
    seed_unknown_order_on(
        &service,
        "bitget-missing-recent",
        "bitget",
        common::time::now_ms(),
    );

    let outcome = must_ok(
        service.reconcile_and_refresh_missing_orders().await,
        "recent Bitget missing submit reconciliation",
    );
    let record = must_some(
        service.get_order("bitget-missing-recent"),
        "recent Bitget missing submit record",
    );

    assert_eq!(record.state, LiveOrderState::Unknown);
    assert!(outcome.refreshed.is_empty());
}

#[tokio::test]
async fn reconcile_fails_old_kucoin_submit_confirmed_missing_by_client_oid() {
    let service = service_with_reconcile_adapter(Vec::new(), None);
    *service.adapter_name.write() = LIVE_ROUTER_ADAPTER_ID;
    seed_unknown_order_on(
        &service,
        "kucoin-missing-old",
        "kucoin",
        common::time::now_ms() - 60_001,
    );

    let outcome = must_ok(
        service.reconcile_and_refresh_missing_orders().await,
        "old KuCoin missing submit reconciliation",
    );
    let record = must_some(
        service.get_order("kucoin-missing-old"),
        "old KuCoin missing submit record",
    );

    assert_eq!(record.state, LiveOrderState::Failed);
    assert_eq!(record.last_update_source, OrderUpdateSource::Reconcile);
    assert!(record
        .message
        .as_deref()
        .is_some_and(|message| message.contains("kucoin order was not found by clientOid")));
    assert_eq!(outcome.refreshed.len(), 1);
}

#[tokio::test]
async fn reconcile_fails_old_okx_submit_confirmed_missing_by_client_order_id() {
    let service = service_with_reconcile_adapter(Vec::new(), None);
    *service.adapter_name.write() = LIVE_ROUTER_ADAPTER_ID;
    seed_unknown_order_on(
        &service,
        "okx-missing-old",
        "okx",
        common::time::now_ms() - 60_001,
    );

    let outcome = must_ok(
        service.reconcile_and_refresh_missing_orders().await,
        "old OKX missing submit reconciliation",
    );
    let record = must_some(
        service.get_order("okx-missing-old"),
        "old OKX missing submit record",
    );

    assert_eq!(record.state, LiveOrderState::Failed);
    assert_eq!(record.last_update_source, OrderUpdateSource::Reconcile);
    assert!(record
        .message
        .as_deref()
        .is_some_and(|message| message.contains("okx order was not found by clOrdId")));
    assert_eq!(outcome.refreshed.len(), 1);
}

#[tokio::test]
async fn reconcile_keeps_recent_okx_missing_submit_unknown() {
    let service = service_with_reconcile_adapter(Vec::new(), None);
    *service.adapter_name.write() = LIVE_ROUTER_ADAPTER_ID;
    seed_unknown_order_on(
        &service,
        "okx-missing-recent",
        "okx",
        common::time::now_ms(),
    );

    let outcome = must_ok(
        service.reconcile_and_refresh_missing_orders().await,
        "recent OKX missing submit reconciliation",
    );
    let record = must_some(
        service.get_order("okx-missing-recent"),
        "recent OKX missing submit record",
    );

    assert_eq!(record.state, LiveOrderState::Unknown);
    assert!(outcome.refreshed.is_empty());
}

#[tokio::test]
async fn reconcile_fails_old_persisted_kucoin_submitted_record_confirmed_missing() {
    let service = service_with_reconcile_adapter(Vec::new(), None);
    *service.adapter_name.write() = LIVE_ROUTER_ADAPTER_ID;
    let mut intent = limit_intent("kucoin-submitted-missing-old");
    intent.exchange = "kucoin".to_owned();
    intent.mode = ExecutionMode::Live;
    intent.created_at_ms = common::time::now_ms() - 60_001;
    seed_account_order(&service, intent.clone(), intent.created_at_ms);
    service.journal.mark_risk_checked(
        &intent.id,
        RiskDecision::allow(intent.quantity * 50_000.0),
        intent.created_at_ms + 1,
    );
    service
        .journal
        .mark_submitted(&intent.id, intent.created_at_ms + 2);

    let outcome = must_ok(
        service.reconcile_and_refresh_missing_orders().await,
        "old persisted KuCoin submitted reconciliation",
    );
    let record = must_some(
        service.get_order(&intent.id),
        "old persisted KuCoin submitted record",
    );

    assert_eq!(record.state, LiveOrderState::Failed);
    assert_eq!(record.last_update_source, OrderUpdateSource::Reconcile);
    assert_eq!(outcome.refreshed.len(), 1);
}

#[tokio::test]
async fn reconcile_never_uses_mock_missing_result_as_live_order_evidence() {
    let service = service_with_reconcile_adapter(Vec::new(), None);
    seed_unknown_order_on(
        &service,
        "kucoin-missing-through-mock",
        "kucoin",
        common::time::now_ms() - 60_001,
    );

    let outcome = must_ok(
        service.reconcile_and_refresh_missing_orders().await,
        "mock route missing result remains observe-only",
    );
    let record = must_some(
        service.get_order("kucoin-missing-through-mock"),
        "mock route must retain live order ambiguity",
    );

    assert_eq!(record.state, LiveOrderState::Unknown);
    assert!(outcome.refreshed.is_empty());
}
