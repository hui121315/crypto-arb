use super::*;

#[tokio::test]
async fn empty_open_orders_snapshot_seeds_a_fresh_order_cache() {
    let service = TradingService::new_mock();
    let outcome = service
        .apply_private_ws_event(PrivateWsEvent::OpenOrders(PrivateOpenOrdersSnapshot {
            venue: "hyperliquid:xyz".to_owned(),
            rows: Vec::new(),
        }))
        .await;

    assert!(outcome.open_order_cache_updated);
    assert!(!outcome.account_cache_updated);
    let cached = service
        .open_order_cache
        .fresh(
            "hyperliquid:xyz",
            service.account_cache_epoch(),
            common::time::now_ms(),
        )
        .expect("authoritative empty snapshot seeds cache");
    assert!(cached.is_empty());
}

#[tokio::test]
async fn position_and_balance_snapshots_seed_account_cache() -> anyhow::Result<()> {
    let service = TradingService::new_mock();
    let positions = vec![PositionInfo {
        symbol: "BTC".into(),
        exchange: "mock".into(),
        side: "long".into(),
        quantity: 1.0,
        entry_price: 50_000.0,
        mark_price: 50_100.0,
        unrealized_pnl: 100.0,
        leverage: 1.0,
        liquidation_price: None,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin: 1_000.0,
        maintenance_margin_ratio: 0.0,
        position_mode: None,
        margin_mode: None,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }];
    let balances = vec![VenueBalanceInfo {
        venue: "okx".into(),
        currency: "USDT".into(),
        total: 10.0,
        available: 8.0,
        frozen: 2.0,
        unrealized_pnl: 0.0,
    }];
    // PR-DP-08 D-8：list_configured_balances 现在按 dispatcher_venues_for 决定
    // 哪些 venue 走 cache，AdapterCredentials::default() 不含任何 venue → 退到
    // mock fetch 路径。用 fake okx credentials 触发 "okx" venue dispatcher。
    let credentials = AdapterCredentials {
        okx_live: Some(("k".into(), "s".into(), "p".into())),
        ..AdapterCredentials::default()
    };

    let positions_outcome = service
        .apply_private_ws_event(PrivateWsEvent::Positions(PrivatePositionsSnapshot {
            venue: "mock".into(),
            rows: positions.clone(),
        }))
        .await;
    let balances_outcome = service
        .apply_private_ws_event(PrivateWsEvent::Balances(Box::new(
            PrivateBalancesSnapshot {
                venue: "okx".into(),
                rows: balances.clone(),
            },
        )))
        .await;

    assert!(positions_outcome.account_cache_updated);
    assert!(balances_outcome.account_cache_updated);
    service.update_risk_config(|config| {
        config.allowed_exchanges.insert("mock".to_owned());
    });
    let cached_positions = service.list_positions().await?;
    assert_eq!(cached_positions.len(), 1);
    assert_eq!(cached_positions[0].symbol, positions[0].symbol);
    assert_eq!(cached_positions[0].quantity, positions[0].quantity);
    assert_eq!(
        service.list_configured_balances(credentials).await?,
        balances
    );
    Ok(())
}

/// PR-DP-08 D-8 验证：per-venue cache 改造后，同一个 venue 的 WS 推送、
/// 任意 credentials 集合的读取都能命中 cache rows（修复了原 single-entry
/// `AccountCache` + credentials-hash cache key 造成的不命中 bug）。
#[tokio::test]
async fn per_venue_ws_balances_visible_under_any_credentials_read() {
    let service = TradingService::new_mock();
    let single_venue = AdapterCredentials {
        okx_live: Some(("k".into(), "s".into(), "p".into())),
        ..AdapterCredentials::default()
    };
    let full_venues = AdapterCredentials {
        okx_live: Some(("k".into(), "s".into(), "p".into())),
        bybit_live: Some(("bk".into(), "bs".into())),
        ..AdapterCredentials::default()
    };
    let okx_balance = VenueBalanceInfo {
        venue: "okx".into(),
        currency: "USDT".into(),
        total: 100.0,
        available: 80.0,
        frozen: 20.0,
        unrealized_pnl: 0.0,
    };

    // dispatcher 推 OKX venue 的 Balances snapshot（D-8 后 mapper 使用 venue 字段）
    service
        .apply_private_ws_event(PrivateWsEvent::Balances(Box::new(
            PrivateBalancesSnapshot {
                venue: "okx".into(),
                rows: vec![okx_balance.clone()],
            },
        )))
        .await;

    // 用 OKX-only credentials 读取 → 命中 cache
    let cached_with_single = service
        .list_configured_balances(single_venue)
        .await
        .expect("single-venue read");
    assert!(
        cached_with_single.iter().any(|row| row == &okx_balance),
        "OKX row should be visible under single-venue read; got: {cached_with_single:?}"
    );

    // 用 full multi-venue credentials 读取 → OKX 行仍可见（会 fetch Bybit 部分但不会
    // 覆盖 OKX cache entry；本 test 不检查 fetch 起作用的 row、仅验证 OKX visible）。
    let cached_with_full = service
        .list_configured_balances(full_venues)
        .await
        .expect("full-credentials read");
    assert!(
        cached_with_full.iter().any(|row| row == &okx_balance),
        "OKX row should be visible under full-credentials read after D-8 fix; \
         cache returned: {cached_with_full:?}"
    );
}

/// PR-DP-08 D-8 验证：两个不同 venue 的 WS 推送各自独立 → 读取后两个都能看到。
#[tokio::test]
async fn two_venue_ws_balances_coexist_in_cache() {
    let service = TradingService::new_mock();
    let creds = AdapterCredentials {
        okx_live: Some(("k".into(), "s".into(), "p".into())),
        bybit_live: Some(("bk".into(), "bs".into())),
        ..AdapterCredentials::default()
    };
    let okx_row = VenueBalanceInfo {
        venue: "okx".into(),
        currency: "USDT".into(),
        total: 100.0,
        available: 80.0,
        frozen: 20.0,
        unrealized_pnl: 0.0,
    };
    let bybit_row = VenueBalanceInfo {
        venue: "bybit".into(),
        currency: "USDT".into(),
        total: 200.0,
        available: 150.0,
        frozen: 50.0,
        unrealized_pnl: 0.0,
    };

    service
        .apply_private_ws_event(PrivateWsEvent::Balances(Box::new(
            PrivateBalancesSnapshot {
                venue: "okx".into(),
                rows: vec![okx_row.clone()],
            },
        )))
        .await;
    service
        .apply_private_ws_event(PrivateWsEvent::Balances(Box::new(
            PrivateBalancesSnapshot {
                venue: "bybit".into(),
                rows: vec![bybit_row.clone()],
            },
        )))
        .await;

    let merged = service
        .list_configured_balances(creds)
        .await
        .expect("merged read");
    assert!(
        merged.iter().any(|row| row == &okx_row),
        "OKX row should be in merged cache; got: {merged:?}"
    );
    assert!(
        merged.iter().any(|row| row == &bybit_row),
        "Bybit row should be in merged cache; got: {merged:?}"
    );
}
