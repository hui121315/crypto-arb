use super::*;
use chrono::{TimeZone, Utc};
use shared_types::{OrderSide, OrderStatus, OrderType};

#[test]
fn partial_refresh_keeps_failed_venue_bounded_stale() {
    let service = TradingService::new_mock();
    let epoch = service.account_cache_epoch();
    service
        .open_order_cache
        .replace("binance", epoch, vec![order("binance", "1")]);
    service
        .open_order_cache
        .replace("bitget", epoch, vec![order("bitget", "2")]);
    service.route_failures.record(
        OPEN_ORDER_OPERATION,
        vec![RouteFailure::new(
            "bitget".to_owned(),
            OPEN_ORDER_OPERATION,
            exchange::ExchangeError::Timeout { seconds: 3 },
        )],
    );

    let rows = service.merge_open_order_refresh(
        epoch,
        &["binance".to_owned(), "bitget".to_owned()],
        common::time::now_ms(),
        vec![order("binance", "3")],
    );

    assert!(rows.iter().any(|row| row.order_id == "3"));
    assert!(rows.iter().any(|row| row.order_id == "2"));
    assert!(service
        .open_order_backoff_error("bitget", common::time::now_ms())
        .is_some());
    assert!(service
        .open_order_backoff_error("binance", common::time::now_ms())
        .is_none());
    assert_eq!(service.take_route_failures(OPEN_ORDER_OPERATION).len(), 1);
}

#[test]
fn failed_venue_without_seed_is_not_claimed_as_complete_empty() {
    let service = TradingService::new_mock();
    let epoch = service.account_cache_epoch();
    service.route_failures.record(
        OPEN_ORDER_OPERATION,
        vec![RouteFailure::new(
            "bitget".to_owned(),
            OPEN_ORDER_OPERATION,
            exchange::ExchangeError::Parse("schema drift".to_owned()),
        )],
    );

    let rows = service.merge_open_order_refresh(
        epoch,
        &["binance".to_owned(), "bitget".to_owned()],
        common::time::now_ms(),
        vec![order("binance", "3")],
    );

    assert_eq!(rows.len(), 1);
    assert!(service
        .open_order_cache
        .fresh_all(
            &["binance".to_owned(), "bitget".to_owned()],
            epoch,
            common::time::now_ms(),
        )
        .is_none());
}

#[test]
fn open_order_backoff_serves_stale_and_blocks_missing_evidence() {
    let service = TradingService::new_mock();
    let epoch = service.account_cache_epoch();
    let now_ms = common::time::now_ms();
    service
        .open_order_cache
        .replace("bitget", epoch, vec![order("bitget", "2")]);
    service
        .open_order_fetch_backoffs
        .insert("bitget".to_owned(), timeout_backoff(now_ms + 10_000));
    service
        .open_order_fetch_backoffs
        .insert("gate".to_owned(), timeout_backoff(now_ms + 10_000));

    let (refresh, deferred, blocked_error) = service.partition_open_order_refresh(
        &["bitget".to_owned(), "gate".to_owned()],
        epoch,
        now_ms,
    );

    assert!(refresh.is_empty());
    assert_eq!(deferred.len(), 1);
    assert_eq!(deferred[0].exchange, "bitget");
    assert!(matches!(
        blocked_error,
        Some(exchange::ExchangeError::Timeout { seconds: 3 })
    ));
}

#[test]
fn open_order_backoff_expires_per_venue() {
    let service = TradingService::new_mock();
    let now_ms = common::time::now_ms();
    service
        .open_order_fetch_backoffs
        .insert("gate".to_owned(), timeout_backoff(now_ms + 10_000));

    assert!(service.open_order_backoff_error("gate", now_ms).is_some());
    assert!(service
        .open_order_backoff_error("binance", now_ms)
        .is_none());
    assert!(service
        .open_order_backoff_error("gate", now_ms + 10_001)
        .is_none());
    assert!(!service.open_order_fetch_backoffs.contains_key("gate"));
}

#[test]
fn open_order_backoff_preserves_exact_route_error() {
    let service = TradingService::new_mock();
    let now_ms = common::time::now_ms();
    let error = exchange::ExchangeError::Parse("schema drift".to_owned());

    service.record_open_order_fetch_error(&["gate".to_owned()], &error, now_ms);

    assert!(matches!(
        service.open_order_backoff_error("gate", now_ms),
        Some(exchange::ExchangeError::Parse(message)) if message == "schema drift"
    ));
}

#[test]
fn hyperliquid_open_order_failure_uses_family_cooldown() {
    let service = TradingService::new_mock();
    let now_ms = common::time::now_ms();

    service.record_open_order_fetch_error(
        &["hyperliquid:xyz".to_owned()],
        &exchange::ExchangeError::Timeout { seconds: 10 },
        now_ms,
    );

    assert!(service
        .open_order_backoff_error(
            "hyperliquid:xyz",
            now_ms + HYPERLIQUID_OPEN_ORDER_FETCH_ERROR_BACKOFF_MS - 1,
        )
        .is_some());
    assert!(service
        .open_order_backoff_error(
            "hyperliquid:xyz",
            now_ms + HYPERLIQUID_OPEN_ORDER_FETCH_ERROR_BACKOFF_MS + 1,
        )
        .is_none());
}

fn order(venue: &str, id: &str) -> OrderInfo {
    OrderInfo {
        order_id: id.to_owned(),
        symbol: "SOLUSDT".to_owned(),
        exchange: venue.to_owned(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        status: OrderStatus::Open,
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

fn timeout_backoff(retry_until_ms: i64) -> BalanceFetchBackoff {
    BalanceFetchBackoff {
        retry_until_ms,
        error: CachedExchangeError::Timeout { seconds: 3 },
    }
}
