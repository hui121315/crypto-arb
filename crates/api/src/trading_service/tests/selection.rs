use super::*;

#[tokio::test]
async fn pinned_close_engine_keeps_submission_and_recovery_on_original_adapter() {
    let (service, original) = super::support::service_with_submit_timeout(None);
    let pinned = service.capture_submission_engine();
    let replacement = Arc::new(super::adapters::ReconcileTestAdapter::new(vec![], None));
    service.engine.set_adapter(replacement.clone());
    let result = service.submit_on_engine(limit_intent("pinned-close"), &pinned).await;
    assert!(matches!(result, Err(TradingError::Exchange(ExchangeError::Timeout { seconds: 10 }))));
    assert_eq!(original.exchange_order_query_ids(), vec!["client-pinned-close"]);
    assert!(replacement.exchange_order_query_ids().is_empty());
    service.update_risk_config(|config| config.max_order_notional = 1.0);
    let blocked = service.submit_on_engine(limit_intent("pinned-risk"), &pinned).await;
    assert!(matches!(blocked, Err(TradingError::RiskBlocked(_))));
    assert_eq!(original.exchange_order_query_ids().len(), 1);
}

#[test]
fn new_mock_defaults_are_safe() {
    let service = TradingService::new_mock();
    assert_eq!(service.adapter_name(), "mock");
    assert_eq!(service.open_order_count(), 0);
    let risk = service.risk_config();
    assert!(!risk.live_trading_enabled);
    assert!(!risk.kill_switch_active);
}

#[tokio::test]
async fn pinned_cancel_keeps_ack_and_terminal_query_on_original_adapter() {
    let remote = super::support::order_info("x1", shared_types::OrderStatus::Canceled, 0.01);
    let (service, original) = super::support::service_with_reconcile_adapter_handle(vec![], Some(remote));
    let mut intent = limit_intent("pinned-cancel");
    intent.mode = ExecutionMode::Testnet;
    must_ok(service.submit(intent).await, "isolated adapter accepts order");
    let pinned = service.capture_submission_engine();
    let replacement = Arc::new(super::adapters::ReconcileTestAdapter::new(vec![], None));
    service.engine.set_adapter(replacement.clone());
    let result = must_ok(service.cancel_on_engine("pinned-cancel", &pinned).await, "cancel original order");
    assert_eq!(result.state, LiveOrderState::Cancelled);
    assert_eq!(original.exchange_order_id_queries(), vec!["x1"]);
    assert!(replacement.exchange_order_id_queries().is_empty());
    assert!(replacement.exchange_order_query_ids().is_empty());
}

#[test]
fn select_mock_adapter_resets_allowed_exchanges() {
    let service = TradingService::new_mock();
    let _ = must_ok(
        service.select_binance_testnet_adapter("k".into(), "s".into()),
        "binance testnet adapter should construct with arbitrary credentials",
    );
    assert_eq!(service.adapter_name(), "binance_testnet");
    let risk = service.select_mock_adapter();
    assert_eq!(service.adapter_name(), "mock");
    assert!(risk.allowed_exchanges.is_empty());
    assert!(!risk.live_trading_enabled);
}

#[test]
fn select_okx_testnet_adapter_sets_allowed_exchange() {
    let service = TradingService::new_mock();
    let risk = must_ok(
        service.select_okx_testnet_adapter("k".into(), "s".into(), "p".into()),
        "okx testnet adapter constructs offline",
    );
    assert_eq!(service.adapter_name(), "okx_testnet");
    assert!(risk.allowed_exchanges.contains("okx"));
    assert!(!risk.live_trading_enabled);
}

#[test]
fn select_binance_testnet_adapter_sets_allowed_exchange() {
    let service = TradingService::new_mock();
    let risk = must_ok(
        service.select_binance_testnet_adapter("k".into(), "s".into()),
        "binance testnet adapter constructs offline",
    );
    assert_eq!(service.adapter_name(), "binance_testnet");
    assert!(risk.allowed_exchanges.contains("binance"));
    assert!(!risk.live_trading_enabled);
}

#[test]
fn set_kill_switch_toggles_state() {
    let service = TradingService::new_mock();
    let on = service.set_kill_switch(true);
    assert!(on.kill_switch_active);
    let off = service.set_kill_switch(false);
    assert!(!off.kill_switch_active);
}

#[test]
fn try_select_adapter_enables_live_with_configured_credentials() {
    let service = TradingService::new_mock();
    let risk = must_ok(
        service.try_select_adapter(LIVE_ROUTER_ADAPTER_ID, credentials()),
        "live mode should only require configured credentials",
    );
    assert_eq!(service.adapter_name(), LIVE_ROUTER_ADAPTER_ID);
    assert!(risk.live_trading_enabled);
    assert!(!risk.allowed_exchanges.is_empty());
}

#[test]
fn try_select_adapter_rejects_missing_credentials() {
    let service = TradingService::new_mock();
    let err = must_err(
        service.try_select_adapter(LIVE_ROUTER_ADAPTER_ID, AdapterCredentials::default()),
        "missing credentials must fail",
    );
    assert!(matches!(err, SelectAdapterError::MissingCredentials));
    assert_eq!(service.adapter_name(), "mock");
}

#[test]
fn try_select_adapter_rejects_unsupported_adapter_id() {
    let service = TradingService::new_mock();
    let err = must_err(
        service.try_select_adapter("does_not_exist", AdapterCredentials::default()),
        "unsupported adapter must fail",
    );
    assert!(matches!(err, SelectAdapterError::Unsupported(_)));
}

#[tokio::test]
async fn try_select_adapter_blocks_when_open_orders_exist() {
    let service = TradingService::new_mock();
    must_ok(
        service.submit(limit_intent("o1")).await,
        "mock submit accepts limit order",
    );
    assert!(service.open_order_count() >= 1);
    let err = must_err(
        service.try_select_adapter(LIVE_ROUTER_ADAPTER_ID, credentials()),
        "open orders must block adapter switch",
    );
    assert!(matches!(err, SelectAdapterError::OpenOrders));
    assert_eq!(service.adapter_name(), "mock");
}

#[tokio::test]
async fn try_select_adapter_allows_kill_switched_recovery_with_open_orders() {
    let service = TradingService::new_mock();
    must_ok(
        service.submit(limit_intent("o1")).await,
        "mock submit accepts limit order",
    );
    let risk = service.set_kill_switch(true);
    assert!(risk.kill_switch_active);

    let selected = must_ok(
        service.try_select_adapter(LIVE_ROUTER_ADAPTER_ID, credentials()),
        "kill-switched recovery may load the live query adapter",
    );

    assert_eq!(service.adapter_name(), LIVE_ROUTER_ADAPTER_ID);
    assert!(selected.live_trading_enabled);
    assert!(selected.kill_switch_active);
}

#[test]
fn try_select_adapter_can_switch_back_to_mock() {
    let service = TradingService::new_mock();
    let _ = must_ok(
        service.select_binance_testnet_adapter("k".into(), "s".into()),
        "seed binance_testnet adapter",
    );
    assert_eq!(service.adapter_name(), "binance_testnet");
    let risk = must_ok(
        service.try_select_adapter("mock", AdapterCredentials::default()),
        "switching back to mock requires no confirmation",
    );
    assert_eq!(service.adapter_name(), "mock");
    assert!(risk.allowed_exchanges.is_empty());
}
