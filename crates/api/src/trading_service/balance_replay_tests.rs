use super::*;

#[test]
fn sql_balance_replay_seeds_latest_per_venue_asset_cache() {
    let service = TradingService::new_mock();
    let now_ms = common::time::now_ms();
    let older = balance_replay_event("okx", "USDT", 10.0, now_ms - 2);
    let newer = balance_replay_event(" OKX ", "usdt", 20.0, now_ms - 1);

    let seeded = service.seed_balance_cache_from_sql_replay_for_test(&[older, newer]);
    let rows = service
        .balance_cache
        .fresh("okx", service.account_cache_epoch(), now_ms);
    assert!(rows.is_some(), "fresh replay balance");
    let rows = rows.unwrap_or_default();

    assert_eq!(seeded, 1);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].venue, "okx");
    assert_eq!(rows[0].currency, "USDT");
    assert_eq!(rows[0].available, 20.0);
}

#[test]
fn sql_balance_replay_preserves_stale_age() {
    let service = TradingService::new_mock();
    let now_ms = common::time::now_ms();
    let observed_at_ms = now_ms.saturating_sub(BALANCE_CACHE_TTL_MS + 1);
    let seeded = service.seed_balance_cache_from_sql_replay_for_test(&[balance_replay_event(
        "binance",
        "USDC",
        30.0,
        observed_at_ms,
    )]);

    assert_eq!(seeded, 1);
    assert!(service
        .balance_cache
        .fresh("binance", service.account_cache_epoch(), now_ms)
        .is_none());
    assert!(service
        .balance_cache
        .stale("binance", service.account_cache_epoch(), now_ms)
        .is_some());
}

fn balance_replay_event(
    venue: &str,
    currency: &str,
    available: f64,
    observed_at_ms: i64,
) -> SqlBalanceLedgerReplayEvent {
    SqlBalanceLedgerReplayEvent {
        event_id: format!("{venue}:{currency}:{observed_at_ms}"),
        balance_kind: "snapshot".to_owned(),
        row: VenueBalanceInfo {
            venue: venue.to_owned(),
            currency: currency.to_owned(),
            total: available,
            available,
            frozen: 0.0,
            unrealized_pnl: 0.0,
        },
        observed_at_ms,
        captured_at_ms: observed_at_ms,
    }
}
