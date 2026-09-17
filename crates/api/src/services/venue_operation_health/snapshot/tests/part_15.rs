#[test]
fn nav_storage_row_maps_disabled_path_to_warn_problem() {
    let health = nav_health(false);

    let row = nav_storage_row(&health, 10);

    assert_eq!(row.venue, SYSTEM_VENUE);
    assert_eq!(row.operation, OP_NAV_STORAGE);
    assert_eq!(row.source, SOURCE_NAV_STORAGE);
    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::NAV_STORAGE_UNAVAILABLE)
    );
    assert!(row
        .problem
        .as_ref()
        .and_then(|problem| problem.details.as_ref())
        .and_then(|details| details.pointer("/storageContract/backendKind"))
        .is_some_and(|kind| kind == "memory"));
    assert!(row
        .problem
        .as_ref()
        .and_then(|problem| problem.details.as_ref())
        .and_then(|details| details.pointer("/migrationAuthority/applied"))
        .is_some_and(|applied| applied == false));
}

#[test]
fn market_ws_warmup_is_unknown_without_operator_error() {
    let row = market_row(
        MarketRuntimeHealth {
            venue: "kucoin".to_owned(),
            operation: crate::services::market_data::cache::MARKET_OP_WS_TICKER_SNAPSHOT,
            quality: MarketQuality::Warming,
            source: MarketSource::WsPush,
            requested: 8,
            rows: 7,
            retry_after_ms: Some(7_000),
            last_error: Some("awaiting first websocket event".to_owned()),
            problem: None,
            observed_at_ms: 1_000,
        },
        1_500,
    );

    assert_eq!(row.status, VenueOperationStatus::Unknown);
    assert_eq!(row.rows, Some(7));
    assert_eq!(row.retry_after_ms, Some(7_000));
    assert!(row.error.is_none());
}
