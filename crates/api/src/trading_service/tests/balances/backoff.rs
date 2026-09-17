use super::*;

#[test]
fn partial_route_failure_seeds_only_failed_balance_backoff() {
    let service = TradingService::new_mock();
    let now_ms = common::time::now_ms();
    service.route_failures.record(
        BALANCE_ROUTE_OPERATION,
        vec![RouteFailure::new(
            "gate".to_owned(),
            BALANCE_ROUTE_OPERATION,
            ExchangeError::Timeout { seconds: 3 },
        )],
    );

    let failed = service.record_balance_route_failure_backoffs(now_ms);

    assert_eq!(failed, HashSet::from(["gate".to_owned()]));
    assert!(matches!(
        service.balance_backoff_error("gate", now_ms),
        Some(ExchangeError::Timeout { seconds: 3 })
    ));
    assert!(matches!(
        service.balance_backoff_error("gate", now_ms + BALANCE_FETCH_ERROR_BACKOFF_MS as i64 - 1,),
        Some(ExchangeError::Timeout { seconds: 3 })
    ));
    assert!(service
        .balance_backoff_error("gate", now_ms + BALANCE_FETCH_ERROR_BACKOFF_MS as i64 + 1,)
        .is_none());
    assert!(service.balance_backoff_error("binance", now_ms).is_none());
    assert_eq!(
        service.take_route_failures(BALANCE_ROUTE_OPERATION).len(),
        1
    );
}

#[test]
fn failed_fresh_venue_is_not_merged_twice_when_another_venue_is_missing() {
    let service = TradingService::new_mock();
    let epoch = service.account_cache_epoch();
    let now_ms = common::time::now_ms();
    service
        .balance_cache
        .replace("binance", epoch, vec![balance_row("binance", 250.0)]);
    let missing = vec!["bitget".to_owned()];
    let failed = HashSet::from(["binance".to_owned()]);
    let mut merged = vec![balance_row("binance", 250.0)];

    service.merge_failed_balance_stale(epoch, now_ms, &missing, &failed, &mut merged);

    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].available, 250.0);
}
