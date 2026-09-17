use super::*;
use chrono::{TimeZone, Utc};
use shared_types::{OrderSide, OrderType};

#[test]
fn ws_delta_requires_complete_seed() {
    let cache = VenueOpenOrderCache::new(30_000, 60_000);

    assert!(!cache.apply_order("binance", 1, order("1", OrderStatus::Open)));
    assert!(cache.venues(1).is_empty());
    assert_eq!(cache.latest_change_ms(), 0);
}

#[test]
fn identical_snapshot_refreshes_freshness_without_reporting_a_change() {
    let cache = VenueOpenOrderCache::new(30_000, 60_000);
    cache.replace_at("okx", 1, Vec::new(), 10_000);
    let first_change_ms = cache.latest_change_ms();

    cache.replace_at("okx", 1, Vec::new(), 20_000);

    assert_eq!(cache.latest_change_ms(), first_change_ms);
    assert!(cache.fresh("okx", 1, 20_001).is_some());
}

#[test]
fn repeated_order_delta_only_reports_a_real_projection_change() {
    let cache = VenueOpenOrderCache::new(30_000, 60_000);
    cache.replace_at("binance", 1, Vec::new(), 10_000);
    let open = order("1", OrderStatus::Open);

    assert!(cache.apply_order("binance", 1, open.clone()));
    let first_change_ms = cache.latest_change_ms();
    assert!(!cache.apply_order("binance", 1, open));
    assert_eq!(cache.latest_change_ms(), first_change_ms);
}

#[test]
fn ws_open_and_terminal_deltas_patch_seeded_snapshot() {
    let cache = VenueOpenOrderCache::new(30_000, 60_000);
    cache.replace_at("binance", 1, vec![order("1", OrderStatus::Open)], 10_000);

    assert!(cache.apply_order("binance", 1, order("2", OrderStatus::Open)));
    assert!(cache.apply_order("binance", 1, order("1", OrderStatus::Filled)));

    let rows = cache.fresh_all(&["binance".to_owned()], 1, 10_001);
    assert_eq!(rows.as_ref().map(Vec::len), Some(1));
    assert_eq!(
        rows.as_ref().map(|rows| rows[0].order_id.as_str()),
        Some("2")
    );
}

#[test]
fn disconnect_invalidates_completeness_but_preserves_bounded_stale_rows() {
    let cache = VenueOpenOrderCache::new(30_000, 60_000);
    cache.replace_at("bitget", 1, vec![order("7", OrderStatus::Open)], 10_000);

    cache.invalidate("bitget");

    assert!(cache.fresh_all(&["bitget".to_owned()], 1, 10_001).is_none());
    assert_eq!(
        cache
            .stale_all(&["bitget".to_owned()], 1, 10_001)
            .as_ref()
            .map(Vec::len),
        Some(1)
    );
    assert!(!cache.apply_order("bitget", 1, order("8", OrderStatus::Open)));
}

#[test]
fn empty_snapshot_is_complete_for_its_venue() {
    let cache = VenueOpenOrderCache::new(30_000, 60_000);
    cache.replace_at("okx", 2, Vec::new(), 20_000);

    assert_eq!(
        cache
            .fresh_all(&["okx".to_owned()], 2, 20_001)
            .as_ref()
            .map(Vec::len),
        Some(0)
    );
}

#[test]
fn active_private_ws_session_extends_seeded_snapshot_freshness() {
    let cache = VenueOpenOrderCache::new(30_000, 60_000);
    cache.replace_at("okx", 2, Vec::new(), common::time::now_ms() - 40_000);

    cache.touch("okx", 2);

    assert_eq!(
        cache
            .fresh_all(&["okx".to_owned()], 2, common::time::now_ms())
            .as_ref()
            .map(Vec::len),
        Some(0)
    );
}

fn order(id: &str, status: OrderStatus) -> OrderInfo {
    OrderInfo {
        order_id: id.to_owned(),
        symbol: "SOLUSDT".to_owned(),
        exchange: "binance".to_owned(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        status,
        quantity: 1.0,
        price: 100.0,
        filled_quantity: 0.0,
        filled_price: 0.0,
        fees: 0.0,
        created_at: Utc.timestamp_millis_opt(1).single().unwrap_or_default(),
        execution_style: None,
        venue_time_in_force: None,
        client_order_id: Some(format!("client-{id}")),
        reduce_only: None,
    }
}
