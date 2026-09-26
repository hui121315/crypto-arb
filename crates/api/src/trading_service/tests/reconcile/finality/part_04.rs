#[tokio::test]
async fn reconcile_repairs_gate_contract_count_recorded_as_base_quantity() {
    let mut remote = order_info("gate-x1", OrderStatus::Filled, 0.0001);
    remote.exchange = "gate".to_owned();
    remote.filled_quantity = 0.0001;
    remote.filled_price = 50_000.0;
    remote.fees = 0.003;
    let service = service_with_reconcile_adapter(Vec::new(), Some(remote));
    let mut intent = limit_intent("gate-unit-repair");
    intent.exchange = "gate".to_owned();
    intent.mode = ExecutionMode::Live;
    intent.quantity = 0.0001;
    seed_account_order(&service, intent.clone(), 1);
    service
        .journal
        .mark_risk_checked(&intent.id, RiskDecision::allow(5.0), 2);
    service.journal.mark_submitted(&intent.id, 3);
    service.journal.apply_ack(&OrderAck {
        internal_order_id: intent.id.clone(),
        exchange_order_id: Some("gate-x1".to_owned()),
        client_order_id: intent.client_order_id,
        identity_update: Default::default(),
        state: LiveOrderState::Filled,
        accepted_at_ms: 4,
        message: None,
        filled_quantity: Some(1.0),
        filled_price: Some(50_000.0),
        filled_fee: Some(0.003),
    });

    let outcome = must_ok(
        service.reconcile_and_refresh_missing_orders().await,
        "Gate quantity unit repair",
    );
    let repaired = must_some(service.get_order(&intent.id), "repaired Gate order");

    assert_eq!(outcome.refreshed.len(), 1);
    assert_eq!(repaired.filled_quantity, Some(0.0001));
    assert_eq!(repaired.last_update_source, OrderUpdateSource::OrderQuery);
}

#[tokio::test]
async fn reconcile_refreshes_kucoin_terminal_ws_identity_without_raw_contract_quantity() {
    let mut remote = order_info("kucoin-x1", OrderStatus::Filled, 0.1);
    remote.exchange = "kucoin".to_owned();
    remote.filled_quantity = 0.1;
    remote.filled_price = 72.0;
    remote.fees = 0.004;
    let service = service_with_reconcile_adapter(Vec::new(), Some(remote));
    let mut intent = limit_intent("kucoin-unit-refresh");
    intent.exchange = "kucoin".to_owned();
    intent.mode = ExecutionMode::Live;
    intent.quantity = 0.1;
    seed_account_order(&service, intent.clone(), 1);
    service
        .journal
        .mark_risk_checked(&intent.id, RiskDecision::allow(7.2), 2);
    service.journal.mark_submitted(&intent.id, 3);
    service.journal.apply_ack(&OrderAck {
        internal_order_id: intent.id.clone(),
        exchange_order_id: Some("kucoin-x1".to_owned()),
        client_order_id: intent.client_order_id,
        identity_update: Default::default(),
        state: LiveOrderState::Filled,
        accepted_at_ms: 4,
        message: None,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
    });

    let outcome = must_ok(
        service.reconcile_and_refresh_missing_orders().await,
        "KuCoin terminal quantity refresh",
    );
    let refreshed = must_some(service.get_order(&intent.id), "refreshed KuCoin order");

    assert_eq!(outcome.refreshed.len(), 1);
    assert_eq!(refreshed.filled_quantity, Some(0.1));
    assert_eq!(refreshed.last_update_source, OrderUpdateSource::OrderQuery);
}

#[tokio::test]
async fn periodic_reconcile_uses_fresh_private_ws_open_order_snapshot() {
    let (service, _) = service_with_open_orders_error(None, "remote read should stay idle");
    service.open_order_cache.replace(
        "reconcile_test",
        service.account_cache_epoch(),
        Vec::new(),
    );

    let outcome = must_ok(
        service.reconcile_and_refresh_missing_orders().await,
        "periodic cached reconcile",
    );

    assert!(outcome.diffs.is_empty());
    assert!(service.reconcile_open_orders().await.is_err());
}
