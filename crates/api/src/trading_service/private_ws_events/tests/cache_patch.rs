use super::*;

#[tokio::test]
async fn account_dirty_marks_only_scoped_venue_stale() {
    let service = TradingService::new_mock();
    service
        .apply_private_ws_event(PrivateWsEvent::Positions(PrivatePositionsSnapshot {
            venue: "okx".into(),
            rows: vec![position_info("okx", "BTC", "long", 1.0)],
        }))
        .await;
    service
        .apply_private_ws_event(PrivateWsEvent::Positions(PrivatePositionsSnapshot {
            venue: "bybit".into(),
            rows: vec![position_info("bybit", "ETH", "short", 2.0)],
        }))
        .await;

    let outcome = service
        .apply_private_ws_event(PrivateWsEvent::AccountDirty(PrivateAccountDirty::new(
            "okx",
            PrivateAccountScope::Positions,
            "incremental_position_delta",
        )))
        .await;

    let dirty = outcome.account_cache_dirty.as_ref();
    assert_eq!(dirty.map(|dirty| dirty.venue.as_str()), Some("okx"));
    assert_eq!(
        dirty.map(|dirty| dirty.scope),
        Some(PrivateAccountScope::Positions)
    );
    assert_eq!(
        dirty.map(|dirty| dirty.reason.as_str()),
        Some("incremental_position_delta")
    );
    let snapshots = service.position_cache_health();
    assert!(snapshots.iter().any(|row| {
        row.venue == "okx"
            && row.quality == crate::trading_service::AccountCacheQuality::Stale
            && row.rows == 1
    }));
    assert!(snapshots.iter().any(|row| {
        row.venue == "bybit"
            && row.quality == crate::trading_service::AccountCacheQuality::Fresh
            && row.rows == 1
    }));
}

#[tokio::test]
async fn position_patch_updates_one_venue_without_replacing_other_venues() -> anyhow::Result<()> {
    let service = TradingService::new_mock();
    service.update_risk_config(|config| {
        config.allowed_exchanges.insert("okx".to_owned());
        config.allowed_exchanges.insert("bybit".to_owned());
    });
    service
        .apply_private_ws_event(PrivateWsEvent::Positions(PrivatePositionsSnapshot {
            venue: "okx".into(),
            rows: vec![position_info("okx", "BTC", "long", 1.0)],
        }))
        .await;
    service
        .apply_private_ws_event(PrivateWsEvent::Positions(PrivatePositionsSnapshot {
            venue: "bybit".into(),
            rows: vec![position_info("bybit", "ETH", "short", 2.0)],
        }))
        .await;
    service
        .apply_private_ws_event(PrivateWsEvent::PositionPatch(PrivatePositionsPatch {
            venue: "okx".into(),
            rows: vec![position_info("okx", "BTC", "long", 3.0)],
        }))
        .await;

    let rows = service.list_positions().await?;
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .any(|row| row.exchange == "okx" && row.quantity == 3.0));
    assert!(rows
        .iter()
        .any(|row| row.exchange == "bybit" && row.quantity == 2.0));
    Ok(())
}

#[tokio::test]
async fn position_patch_without_seed_stays_dirty_and_does_not_invent_snapshot() {
    let service = TradingService::new_mock();

    let outcome = service
        .apply_private_ws_event(PrivateWsEvent::PositionPatch(PrivatePositionsPatch {
            venue: "binance".into(),
            rows: vec![position_info("binance", "SOL", "long", 1.0)],
        }))
        .await;

    assert!(!outcome.account_cache_updated);
    assert_eq!(
        outcome
            .account_cache_dirty
            .as_ref()
            .map(|dirty| dirty.reason.as_str()),
        Some("position_patch_requires_rest_seed")
    );
    assert!(service.position_cache_health().is_empty());
}

#[tokio::test]
async fn balance_patch_updates_one_currency_without_dropping_other_currencies() -> anyhow::Result<()>
{
    let service = TradingService::new_mock();
    let creds = AdapterCredentials {
        gate_live: Some(("k".into(), "s".into())),
        ..AdapterCredentials::default()
    };
    service
        .apply_private_ws_event(PrivateWsEvent::Balances(Box::new(
            PrivateBalancesSnapshot {
                venue: "gate".into(),
                rows: vec![
                    balance_info("gate", "USDT", 100.0),
                    balance_info("gate", "BTC", 1.0),
                ],
            },
        )))
        .await;
    service
        .apply_private_ws_event(PrivateWsEvent::BalancePatch(Box::new(
            PrivateBalancesPatch {
                venue: "gate".into(),
                rows: vec![balance_info("gate", "USDT", 200.0)],
            },
        )))
        .await;

    let rows = service.list_configured_balances(creds).await?;
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .any(|row| row.currency == "USDT" && row.total == 200.0));
    assert!(rows
        .iter()
        .any(|row| row.currency == "BTC" && row.total == 1.0));
    Ok(())
}

#[tokio::test]
async fn private_order_delta_updates_seeded_open_order_projection() {
    let service = TradingService::new_mock();
    let epoch = service.account_cache_epoch();
    service
        .open_order_cache
        .replace("mock", epoch, vec![order_info(OrderStatus::Open)]);

    let outcome = service
        .apply_private_ws_event(PrivateWsEvent::Order(PrivateOrderDelta {
            client_order_id: String::new(),
            order: order_info(OrderStatus::Filled),
            received_at_ms: 10,
        }))
        .await;

    assert!(outcome.open_order_cache_updated);
    assert!(!outcome.account_cache_updated);
    let rows =
        service
            .open_order_cache
            .fresh_all(&["mock".to_owned()], epoch, common::time::now_ms());
    assert!(rows.as_ref().is_some_and(Vec::is_empty));
}

#[tokio::test]
async fn terminal_order_without_positive_fill_does_not_invalidate_account_cache() {
    let service = TradingService::new_mock();
    let mut order = order_info(OrderStatus::Filled);
    order.filled_quantity = 0.0;
    order.filled_price = 0.0;
    order.fees = 0.0;

    let outcome = service
        .apply_private_ws_event(PrivateWsEvent::Order(PrivateOrderDelta {
            client_order_id: String::new(),
            order,
            received_at_ms: 10,
        }))
        .await;

    assert!(outcome.account_cache_dirty.is_none());
}

#[tokio::test]
async fn private_order_delta_without_seed_requests_refresh_without_inventing_snapshot() {
    let service = TradingService::new_mock();

    let outcome = service
        .apply_private_ws_event(PrivateWsEvent::Order(PrivateOrderDelta {
            client_order_id: String::new(),
            order: order_info(OrderStatus::Open),
            received_at_ms: 10,
        }))
        .await;

    assert!(!outcome.account_cache_updated);
    assert!(!outcome.open_order_cache_updated);
    assert!(service.open_order_cache_latest_change_ms() > 0);
    assert!(service
        .open_order_cache
        .venues(service.account_cache_epoch())
        .is_empty());
}
