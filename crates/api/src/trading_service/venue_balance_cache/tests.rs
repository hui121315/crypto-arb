#![allow(clippy::expect_used)]

use super::*;

fn row(venue: &str, currency: &str, total: f64) -> VenueBalanceInfo {
    VenueBalanceInfo {
        venue: venue.to_owned(),
        currency: currency.to_owned(),
        total,
        available: total,
        frozen: 0.0,
        unrealized_pnl: 0.0,
    }
}

#[test]
fn replace_then_fresh_returns_rows() {
    let cache = VenueBalanceCache::new(10_000, 60_000);
    cache.replace(" OKX ", 1, vec![row(" OKX ", "USDT", 100.0)]);
    let now_ms = common::time::now_ms();
    let fresh = cache.fresh("okx", 1, now_ms).expect("fresh okx");
    assert_eq!(fresh.len(), 1);
    assert_eq!(fresh[0].total, 100.0);
}

#[test]
fn replace_at_preserves_observed_age() {
    let cache = VenueBalanceCache::new(10, 100);
    let now_ms = common::time::now_ms();
    cache.replace_at("okx", 1, vec![row("okx", "USDT", 100.0)], now_ms - 50);

    assert!(cache.fresh("okx", 1, now_ms).is_none());
    let stale = cache.stale("okx", 1, now_ms).expect("bounded stale");
    assert_eq!(stale[0].total, 100.0);
    let snapshot = cache.snapshots(1, now_ms);
    assert_eq!(snapshot[0].quality, AccountCacheQuality::Stale);
    assert_eq!(snapshot[0].freshness_ms, 50);
}

#[test]
fn fresh_with_wrong_epoch_returns_none() {
    let cache = VenueBalanceCache::new(10_000, 60_000);
    cache.replace("okx", 1, vec![row("okx", "USDT", 100.0)]);
    let now_ms = common::time::now_ms();
    assert!(cache.fresh("okx", 2, now_ms).is_none());
}

#[test]
fn fresh_after_ttl_expires_falls_back_to_stale() {
    let cache = VenueBalanceCache::new(0, 60_000);
    cache.replace("okx", 1, vec![row("okx", "USDT", 100.0)]);
    let later_ms = common::time::now_ms() + 1; // 让 age > 0 (ttl=0)
    assert!(cache.fresh("okx", 1, later_ms).is_none());
    assert!(cache.stale("okx", 1, later_ms).is_some());
}

#[test]
fn stale_after_max_stale_returns_none() {
    let cache = VenueBalanceCache::new(0, 0);
    cache.replace("okx", 1, vec![row("okx", "USDT", 100.0)]);
    let later_ms = common::time::now_ms() + 1;
    assert!(cache.fresh("okx", 1, later_ms).is_none());
    assert!(cache.stale("okx", 1, later_ms).is_none());
}

#[test]
fn replace_does_not_disturb_other_venue_entries() {
    let cache = VenueBalanceCache::new(10_000, 60_000);
    cache.replace("okx", 1, vec![row("okx", "USDT", 100.0)]);
    cache.replace("bybit", 1, vec![row("bybit", "USDT", 200.0)]);
    let now_ms = common::time::now_ms();
    assert_eq!(
        cache.fresh("okx", 1, now_ms).map(|rows| rows.len()),
        Some(1)
    );
    assert_eq!(
        cache.fresh("bybit", 1, now_ms).map(|rows| rows.len()),
        Some(1)
    );
    // 替换 bybit → okx 不动
    cache.replace("bybit", 1, vec![row("bybit", "USDC", 300.0)]);
    let okx = cache.fresh("okx", 1, now_ms).expect("okx still fresh");
    assert_eq!(okx[0].currency, "USDT");
}

#[test]
fn upsert_replaces_one_currency_without_dropping_others() {
    let cache = VenueBalanceCache::new(10_000, 60_000);
    cache.replace(
        " OKX ",
        1,
        vec![row("okx", "USDT", 100.0), row("okx", "BTC", 1.0)],
    );
    cache.upsert("okx", 1, vec![row("okx", "USDT", 200.0)]);
    let now_ms = common::time::now_ms();
    let rows = cache.fresh("okx", 1, now_ms).expect("okx fresh");
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .any(|row| row.currency == "USDT" && row.total == 200.0));
    assert!(rows
        .iter()
        .any(|row| row.currency == "BTC" && row.total == 1.0));
}

#[test]
fn clear_removes_all_entries() {
    let cache = VenueBalanceCache::new(10_000, 60_000);
    cache.replace("okx", 1, vec![row("okx", "USDT", 100.0)]);
    cache.replace("bybit", 1, vec![row("bybit", "USDT", 200.0)]);
    cache.clear();
    let now_ms = common::time::now_ms();
    assert!(cache.fresh("okx", 1, now_ms).is_none());
    assert!(cache.fresh("bybit", 1, now_ms).is_none());
}

#[test]
fn remove_many_preserves_unrelated_entries() {
    let cache = VenueBalanceCache::new(10_000, 60_000);
    cache.replace("okx", 1, vec![row("okx", "USDT", 100.0)]);
    cache.replace("bybit", 1, vec![row("bybit", "USDT", 200.0)]);
    cache.remove_many(&["okx".to_owned()]);

    let now_ms = common::time::now_ms();
    assert!(cache.fresh("okx", 1, now_ms).is_none());
    assert!(cache.fresh("bybit", 1, now_ms).is_some());
}

#[test]
fn snapshots_report_fresh_rows() {
    let cache = VenueBalanceCache::new(10_000, 60_000);
    cache.replace("okx", 1, vec![row("okx", "USDT", 100.0)]);
    let now_ms = common::time::now_ms();

    let rows = cache.snapshots(1, now_ms);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].venue, "okx");
    assert_eq!(rows[0].rows, 1);
    assert_eq!(rows[0].quality, AccountCacheQuality::Fresh);
}

#[test]
fn invalidate_marks_only_target_venue_stale_and_retains_bounded_rows() {
    let cache = VenueBalanceCache::new(10_000, 60_000);
    cache.replace("okx", 1, vec![row("okx", "USDT", 100.0)]);
    cache.replace("bybit", 1, vec![row("bybit", "USDT", 200.0)]);

    cache.invalidate("okx");

    let now_ms = common::time::now_ms();
    assert!(cache.fresh("okx", 1, now_ms).is_none());
    assert_eq!(
        cache.stale("okx", 1, now_ms).map(|rows| rows.len()),
        Some(1)
    );
    assert!(cache.fresh("bybit", 1, now_ms).is_some());
    let rows = cache.snapshots(1, now_ms);
    assert!(rows.iter().any(|row| {
        row.venue == "okx" && row.quality == AccountCacheQuality::Stale && row.rows == 1
    }));
    assert!(rows.iter().any(|row| {
        row.venue == "bybit" && row.quality == AccountCacheQuality::Fresh && row.rows == 1
    }));
}
