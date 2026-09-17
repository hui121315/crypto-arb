use super::*;

#[test]
fn fresh_all_requires_every_venue() {
    let cache = VenuePositionCache::new(10_000, 60_000);
    cache.replace("okx", 1, vec![position("okx", "BTC", "long", 1.0)]);
    let now_ms = common::time::now_ms();
    assert!(cache
        .fresh_all(&["okx".to_owned(), "bybit".to_owned()], 1, now_ms)
        .is_none());
}

#[test]
fn upsert_replaces_same_symbol_side_only() {
    let cache = VenuePositionCache::new(10_000, 60_000);
    cache.replace(
        "okx",
        1,
        vec![
            position("okx", "BTC", "long", 1.0),
            position("okx", "BTC", "short", 2.0),
        ],
    );
    cache.upsert("okx", 1, vec![position("okx", "BTC", "long", 3.0)]);
    let now_ms = common::time::now_ms();
    let rows = cache.fresh_all(&["okx".to_owned()], 1, now_ms);
    assert!(rows.as_ref().is_some_and(|rows| rows.len() == 2));
    assert!(rows.as_ref().is_some_and(|rows| rows
        .iter()
        .any(|row| row.side == "long" && row.quantity == 3.0)));
    assert!(rows.as_ref().is_some_and(|rows| rows
        .iter()
        .any(|row| row.side == "short" && row.quantity == 2.0)));
}

#[test]
fn upsert_zero_quantity_removes_existing_row() {
    let cache = VenuePositionCache::new(10_000, 60_000);
    cache.replace("okx", 1, vec![position("okx", "BTC", "long", 1.0)]);
    cache.upsert("okx", 1, vec![position("okx", "BTC", "long", 0.0)]);
    let now_ms = common::time::now_ms();
    let rows = cache.fresh_all(&["okx".to_owned()], 1, now_ms);
    assert!(rows.as_ref().is_some_and(Vec::is_empty));
}

#[test]
fn upsert_preserves_existing_maintenance_ratio_when_patch_lacks_source() {
    let cache = VenuePositionCache::new(10_000, 60_000);
    let mut seeded = position("gate", "BTC", "long", 1.0);
    seeded.maintenance_margin_ratio = 0.012;
    cache.replace("gate", 1, vec![seeded]);

    cache.upsert("gate", 1, vec![position("gate", "BTC", "long", 2.0)]);

    let now_ms = common::time::now_ms();
    let rows = cache.fresh_all(&["gate".to_owned()], 1, now_ms);
    assert_eq!(
        rows.as_ref().map(|rows| rows[0].maintenance_margin_ratio),
        Some(0.012)
    );
    assert_eq!(rows.as_ref().map(|rows| rows[0].quantity), Some(2.0));
}

#[test]
fn upsert_accepts_new_real_maintenance_ratio() {
    let cache = VenuePositionCache::new(10_000, 60_000);
    let mut seeded = position("gate", "BTC", "long", 1.0);
    seeded.maintenance_margin_ratio = 0.012;
    cache.replace("gate", 1, vec![seeded]);

    let mut update = position("gate", "BTC", "long", 2.0);
    update.maintenance_margin_ratio = 0.009;
    cache.upsert("gate", 1, vec![update]);

    let now_ms = common::time::now_ms();
    let rows = cache.fresh_all(&["gate".to_owned()], 1, now_ms);
    assert_eq!(
        rows.as_ref().map(|rows| rows[0].maintenance_margin_ratio),
        Some(0.009)
    );
}

#[test]
fn incremental_patch_requires_a_full_rest_or_ws_seed() {
    let cache = VenuePositionCache::new(10_000, 60_000);

    assert!(!cache.upsert("binance", 1, vec![position("binance", "SOL", "long", 1.0)]));
    assert!(cache.snapshots(1, common::time::now_ms()).is_empty());
}

#[test]
fn incremental_patch_preserves_rest_only_risk_fields() {
    let cache = VenuePositionCache::new(10_000, 60_000);
    let mut seeded = position("binance", "SOL", "long", 1.0);
    seeded.mark_price = 181.0;
    seeded.leverage = 10.0;
    seeded.liquidation_price = Some(150.0);
    seeded.liquidation_distance_pct = Some(17.0);
    seeded.next_funding_ms = Some(42);
    seeded.paired_with = Some("bitget".to_owned());
    seeded.maintenance_margin_ratio = 0.005;
    seeded.risk_rate = Some(0.12);
    cache.replace("binance", 1, vec![seeded]);

    let mut patch = position("binance", "SOL", "long", 0.5);
    patch.entry_price = 180.0;
    patch.mark_price = 0.0;
    patch.leverage = 0.0;
    patch.unrealized_pnl = 0.5;
    assert!(cache.upsert("binance", 1, vec![patch]));

    let rows = cache.fresh_all(&["binance".to_owned()], 1, common::time::now_ms());
    assert!(rows.is_some(), "seeded cache should remain complete");
    if let Some(rows) = rows {
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.quantity, 0.5);
        assert_eq!(row.entry_price, 180.0);
        assert_eq!(row.mark_price, 181.0);
        assert_eq!(row.leverage, 10.0);
        assert_eq!(row.liquidation_price, Some(150.0));
        assert_eq!(row.liquidation_distance_pct, Some(17.0));
        assert_eq!(row.next_funding_ms, Some(42));
        assert_eq!(row.paired_with.as_deref(), Some("bitget"));
        assert_eq!(row.maintenance_margin_ratio, 0.005);
        assert_eq!(row.risk_rate, Some(0.12));
    }
}

#[test]
fn snapshots_report_fresh_rows() {
    let cache = VenuePositionCache::new(10_000, 60_000);
    cache.replace("okx", 1, vec![position("okx", "BTC", "long", 1.0)]);
    let now_ms = common::time::now_ms();

    let rows = cache.snapshots(1, now_ms);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].venue, "okx");
    assert_eq!(rows[0].rows, 1);
    assert_eq!(rows[0].quality, AccountCacheQuality::Fresh);
}

#[test]
fn invalidate_marks_only_target_venue_stale_and_retains_bounded_rows() {
    let cache = VenuePositionCache::new(10_000, 60_000);
    cache.replace("okx", 1, vec![position("okx", "BTC", "long", 1.0)]);
    cache.replace("bybit", 1, vec![position("bybit", "ETH", "short", 2.0)]);

    cache.invalidate("okx");

    let now_ms = common::time::now_ms();
    assert!(cache.fresh_all(&["okx".to_owned()], 1, now_ms).is_none());
    assert!(cache.stale_all(&["okx".to_owned()], 1, now_ms).is_some());
    assert!(cache.fresh_all(&["bybit".to_owned()], 1, now_ms).is_some());
    let rows = cache.snapshots(1, now_ms);
    assert!(rows.iter().any(|row| {
        row.venue == "okx" && row.quality == AccountCacheQuality::Stale && row.rows == 1
    }));
    assert!(rows.iter().any(|row| {
        row.venue == "bybit" && row.quality == AccountCacheQuality::Fresh && row.rows == 1
    }));
}

#[test]
fn session_activity_refreshes_valid_snapshot_but_not_invalidated_snapshot() {
    let cache = VenuePositionCache::new(10_000, 60_000);
    cache.replace("bitget", 1, Vec::new());
    let seeded_at_ms = cache.snapshots(1, common::time::now_ms())[0].observed_at_ms;
    let heartbeat_at_ms = seeded_at_ms + 20_000;

    cache.touch_at("bitget", 1, heartbeat_at_ms);

    assert_eq!(
        cache.snapshots(1, heartbeat_at_ms + 30_000)[0].quality,
        AccountCacheQuality::Fresh
    );
    assert_eq!(
        cache.fresh_venues(1, heartbeat_at_ms + 30_000),
        vec!["bitget"]
    );

    cache.invalidate("bitget");
    cache.touch_at("bitget", 1, heartbeat_at_ms + 1_000);
    assert!(cache.fresh_venues(1, heartbeat_at_ms + 1_000).is_empty());
}

fn position(venue: &str, symbol: &str, side: &str, quantity: f64) -> PositionInfo {
    PositionInfo {
        exchange: venue.to_owned(),
        symbol: symbol.to_owned(),
        side: side.to_owned(),
        quantity,
        entry_price: 1.0,
        mark_price: 1.0,
        unrealized_pnl: 0.0,
        leverage: 1.0,
        liquidation_price: None,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin: 0.0,
        maintenance_margin_ratio: 0.0,
        position_mode: None,
        margin_mode: None,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }
}
