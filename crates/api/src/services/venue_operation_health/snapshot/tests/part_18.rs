#[test]
fn bounded_stale_account_cache_stays_ok_while_background_refresh_runs() {
    let snapshot = AccountCacheSnapshot {
        venue: "binance".to_owned(),
        rows: 3,
        freshness_ms: 6_000,
        observed_at_ms: 10,
        quality: AccountCacheQuality::Stale,
    };

    let rows = account_cache_rows(&[credential(true, true)], vec![snapshot], OP_POSITIONS, 20);

    assert_eq!(rows[0].status, VenueOperationStatus::Ok);
    assert_eq!(rows[0].message, "账户快照可用，后台刷新中");
}

#[test]
fn position_cache_stays_ok_inside_its_bounded_stale_window() {
    let snapshot = AccountCacheSnapshot {
        venue: "binance".to_owned(),
        rows: 3,
        freshness_ms: 14_000,
        observed_at_ms: 10,
        quality: AccountCacheQuality::Stale,
    };

    let rows = account_cache_rows(&[credential(true, true)], vec![snapshot], OP_POSITIONS, 20);

    assert_eq!(rows[0].status, VenueOperationStatus::Ok);
    assert_eq!(rows[0].message, "账户快照可用，后台刷新中");
}

#[test]
fn position_cache_warns_beyond_its_bounded_stale_window() {
    let snapshot = AccountCacheSnapshot {
        venue: "binance".to_owned(),
        rows: 3,
        freshness_ms: 61_000,
        observed_at_ms: 10,
        quality: AccountCacheQuality::Stale,
    };

    let rows = account_cache_rows(&[credential(true, true)], vec![snapshot], OP_POSITIONS, 20);

    assert_eq!(rows[0].status, VenueOperationStatus::Warn);
    assert_eq!(rows[0].message, "账户缓存样本可用但已变旧");
}

#[test]
fn balance_cache_stays_ok_during_bounded_background_refresh() {
    let snapshot = AccountCacheSnapshot {
        venue: "binance".to_owned(),
        rows: 3,
        freshness_ms: 20_000,
        observed_at_ms: 10,
        quality: AccountCacheQuality::Stale,
    };

    let rows = account_cache_rows(&[credential(true, true)], vec![snapshot], OP_BALANCE, 20);

    assert_eq!(rows[0].status, VenueOperationStatus::Ok);
    assert_eq!(rows[0].message, "账户快照可用，后台刷新中");
}

#[test]
fn balance_cache_warns_after_background_refresh_grace() {
    let snapshot = AccountCacheSnapshot {
        venue: "binance".to_owned(),
        rows: 3,
        freshness_ms: 24_000,
        observed_at_ms: 10,
        quality: AccountCacheQuality::Stale,
    };

    let rows = account_cache_rows(&[credential(true, true)], vec![snapshot], OP_BALANCE, 20);

    assert_eq!(rows[0].status, VenueOperationStatus::Warn);
}
