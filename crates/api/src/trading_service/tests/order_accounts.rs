use super::*;
use shared_types::FeeProduct;

fn accounts(key: &str) -> HashMap<(String, FeeProduct), String> {
    let credentials = AdapterCredentials {
        binance_live: Some((key.into(), "isolated-secret".into())),
        ..Default::default()
    };
    super::super::live_adapters::account_scopes::from_credentials(&credentials)
}

fn intent(id: &str) -> OrderIntent {
    let mut value = limit_intent(id);
    value.exchange = "binance".into();
    value
}

#[tokio::test]
async fn persisted_order_recovers_only_original_account_after_restart() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("orders.jsonl");
    let before = TradingService::new_mock_with_journal(Arc::new(
        OrderJournal::new_with_storage_paths(None, Some(path.clone())),
    ));
    before
        .engine
        .set_adapter_with_accounts(Arc::new(super::adapters::BalanceProbeAdapter::new()), accounts("account-a"));
    let original = before.submit(intent("account-restart")).await?;
    assert_eq!(original.state, LiveOrderState::Accepted);
    let scope = original
        .identity
        .account_scope
        .clone()
        .expect("account scope persisted");
    assert!(!scope.contains("account-a"));
    assert!(!scope.contains("isolated-secret"));
    drop(before);
    let service = TradingService::new_mock_with_journal(Arc::new(
        OrderJournal::new_with_storage_paths(None, Some(path)),
    ));
    let mut remote =
        super::support::order_info("probe", OrderStatus::Canceled, 0.01);
    remote.exchange = "binance".into();
    let wrong = Arc::new(super::adapters::ReconcileTestAdapter::new(
        vec![],
        Some(remote.clone()),
    ));
    service
        .engine
        .set_adapter_with_accounts(wrong.clone(), accounts("account-b"));
    assert!(matches!(
        service.cancel("account-restart").await,
        Err(TradingError::OrderAccountMismatch { .. })
    ));
    assert!(matches!(
        service.refresh_order_state("account-restart").await,
        Err(TradingError::OrderAccountMismatch { .. })
    ));
    let outcome = service.reconcile_and_refresh_missing_orders().await?;
    assert!(outcome.refreshed.is_empty());
    assert_eq!(outcome.refresh_failures.len(), 1);
    assert!(wrong.exchange_order_id_queries().is_empty());
    assert!(wrong.exchange_order_query_ids().is_empty());
    assert!(wrong.cancel_requests().is_empty());
    let unchanged = service
        .get_order("account-restart")
        .expect("original retained");
    assert_eq!(unchanged.state, LiveOrderState::Accepted);
    assert_eq!(unchanged.identity.account_scope.as_ref(), Some(&scope));
    let original_connection = Arc::new(super::adapters::ReconcileTestAdapter::new(
        vec![],
        Some(remote),
    ));
    service
        .engine
        .set_adapter_with_accounts(original_connection.clone(), accounts("account-a"));
    let cancelled = service.cancel("account-restart").await?;
    assert_eq!(cancelled.state, LiveOrderState::Cancelled);
    assert_eq!(original_connection.cancel_requests(), vec!["account-restart"]);
    assert_eq!(
        original_connection.exchange_order_id_queries(),
        vec!["probe"]
    );
    // Old records stay readable, but absence of account identity is never auto-adopted.
    service
        .journal
        .insert_created(intent("legacy-account"), common::time::now_ms());
    assert!(matches!(
        service.refresh_order_state("legacy-account").await,
        Err(TradingError::OrderAccountMismatch { .. })
    ));
    assert_eq!(original_connection.exchange_order_id_queries().len(), 1);
    Ok(())
}

#[tokio::test]
async fn account_switch_does_not_consume_foreign_private_order_or_revive_mock_session(
) -> anyhow::Result<()> {
    use crate::trading_service::private_ws_events::{PrivateOrderDelta, PrivateWsEvent};
    let service = TradingService::new_mock();
    let original = service.submit(limit_intent("account-session")).await?;
    let pinned = service.capture_submission_engine();
    service.select_mock_adapter();
    assert!(service.engine.ensure_order_account(&original).is_err());
    assert!(pinned.ensure_order_account(&original).is_ok());
    let mut order = super::support::order_info("mock-account-session", OrderStatus::Canceled, 0.01);
    order.exchange = original.intent.exchange.clone();
    order.symbol = original.intent.symbol.clone();
    let outcome = service
        .apply_private_ws_event(PrivateWsEvent::Order(PrivateOrderDelta {
            client_order_id: original.intent.client_order_id.clone(),
            order,
            received_at_ms: common::time::now_ms(),
        }))
        .await;
    assert!(outcome.order.is_none());
    assert_eq!(
        service
            .get_order(&original.intent.id)
            .expect("old order")
            .state,
        LiveOrderState::Accepted
    );
    assert!(matches!(
        service.cancel(&original.intent.id).await,
        Err(TradingError::OrderAccountMismatch { .. })
    ));
    assert_eq!(
        service
            .cancel_on_engine(&original.intent.id, &pinned)
            .await?
            .state,
        LiveOrderState::CancelRequested
    );
    Ok(())
}

#[test]
fn account_scopes_are_per_venue_product_and_never_contain_credentials() {
    let mut credentials = credentials();
    credentials.kraken_live = Some(KrakenAdapterCredentials {
        spot: Some(("spot-account".into(), "spot-secret".into())),
        futures: Some(("futures-account".into(), "futures-secret".into())),
    });
    let first = super::super::live_adapters::account_scopes::from_credentials(&credentials);
    assert_eq!(
        first,
        super::super::live_adapters::account_scopes::from_credentials(&credentials)
    );
    credentials
        .kraken_live
        .as_mut()
        .unwrap()
        .spot
        .as_mut()
        .unwrap()
        .0 = "new-spot".into();
    let second = super::super::live_adapters::account_scopes::from_credentials(&credentials);
    assert_ne!(
        first[&("kraken".into(), FeeProduct::Spot)],
        second[&("kraken".into(), FeeProduct::Spot)]
    );
    assert_eq!(
        first[&("kraken".into(), FeeProduct::Perp)],
        second[&("kraken".into(), FeeProduct::Perp)]
    );
    assert_eq!(
        first[&("binance".into(), FeeProduct::Perp)],
        second[&("binance".into(), FeeProduct::Perp)]
    );
    assert!(!first.contains_key(&("kraken".into(), FeeProduct::Unknown)));
    assert!(first
        .values()
        .all(|scope| scope.starts_with("mainnet:hmac-sha256:")
            && !scope.contains("secret")
            && !scope.contains("account")));
}
