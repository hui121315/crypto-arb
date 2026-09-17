use super::super::*;
use super::*;

#[tokio::test]
async fn nav_history_response_uses_memory_rows_and_storage_envelope() -> Result<(), String> {
    let state = portfolio_test_state().await?;
    state
        .portfolio_nav_history()
        .write()
        .await
        .extend_from_slice(&[(1_000, 100.0), (2_000, 120.0)]);

    let response = nav_history(&state, Some(1)).await;

    assert_eq!(response.count, 1);
    assert_eq!(response.rows[0].occurred_at_ms, 2_000);
    assert_eq!(response.rows[0].nav_usd, 120.0);
    assert_eq!(response.latest_at_ms, Some(2_000));
    assert_eq!(response.source, NAV_HISTORY_SOURCE);
    assert!(response.page.is_some());
    let page = response
        .page
        .as_ref()
        .ok_or_else(|| "NAV history response missing page".to_owned())?;
    assert_eq!(page.limit, 1);
    assert!(page.has_more);
    assert_eq!(response.backend_status.backend, "memory");
    assert_eq!(
        response.backend_status.storage_contract.backend_kind,
        shared_types::StorageBackendKind::Memory
    );
    assert!(response
        .backend_status
        .storage_contract
        .degraded_reasons
        .contains(&shared_types::StorageDegradedReason::Disabled));
    assert!(response
        .backend_status
        .storage_contract
        .migration_authority
        .as_ref()
        .is_some_and(|authority| {
            authority.migration_id == nav_persist::NAV_SCHEMA_MIGRATION_ID && !authority.applied
        }));
    assert!(response.backend_status.fallback);
    assert!(response.backend_status.ephemeral);
    assert_eq!(
        response.backend_status.last_error_code.as_deref(),
        Some(shared_types::problem::codes::NAV_STORAGE_UNAVAILABLE)
    );
    assert!(response.storage_health.is_some());
    let storage_health = response
        .storage_health
        .as_ref()
        .ok_or_else(|| "NAV history response missing storage health".to_owned())?;
    assert_eq!(
        storage_health.operation,
        shared_types::OP_STORAGE_PORTFOLIO_NAV
    );
    assert!(response.problem.is_some());
    assert_eq!(response.problems.len(), 1);
    Ok(())
}

#[test]
fn only_configured_current_account_data_health_marks_portfolio_degraded() {
    assert!(!operation_health_degraded(&[operation_health(
        VenueOperationStatus::Ok,
    )]));
    assert!(!operation_health_degraded(&[operation_health(
        VenueOperationStatus::Warn,
    )]));

    let mut positions = operation_health(VenueOperationStatus::Warn);
    positions.venue = "binance".to_owned();
    positions.operation = "positions".to_owned();
    assert!(operation_health_degraded(&[positions]));

    let mut unconfigured = operation_health(VenueOperationStatus::Blocked);
    unconfigured.venue = "gate".to_owned();
    unconfigured.operation = "positions".to_owned();
    unconfigured.configured = Some(false);
    assert!(!operation_health_degraded(&[unconfigured]));
}

#[test]
fn attaches_cached_funding_rate_to_position_rows() {
    let rows = rows_from_positions(vec![position("long")], &[funding()], 0, 15.0, 8.0);

    assert_eq!(rows[0].funding_rate_8h, 0.0002);
    assert!(rows[0].funding_rate_verified);
}

#[test]
fn missing_cached_funding_rate_marks_field_quality() {
    let mut position = position("long");
    position.next_funding_ms = Some(120_000);
    let rows = rows_from_positions(vec![position], &[], 0, 15.0, 8.0);

    let quality = funding_field_quality(&rows, 42);

    assert!(!rows[0].funding_rate_verified);
    assert_eq!(quality.len(), 1);
    assert_eq!(quality[0].field, "fundingRate8h");
    assert_eq!(quality[0].status, AccountFieldQualityStatus::Missing);
    assert!(quality[0].problem.as_ref().is_some_and(
        |problem| problem.code == shared_types::problem::codes::POSITION_FIELD_UNAVAILABLE
    ));
}

#[test]
fn missing_funding_time_marks_field_quality_even_with_verified_rate() {
    let rows = rows_from_positions(vec![position("long")], &[funding()], 0, 15.0, 8.0);

    let quality = funding_field_quality(&rows, 42);

    assert!(rows[0].funding_rate_verified);
    assert_eq!(quality.len(), 1);
    assert_eq!(quality[0].field, "nextFundingMs");
    assert_eq!(quality[0].status, AccountFieldQualityStatus::Missing);
}

#[test]
fn filled_dry_run_orders_create_simulated_position_rows() {
    let orders = vec![
        dry_order("h1-long", "hyperliquid", "MU", OrderSide::Buy, false),
        dry_order("h1-short", "kucoin", "MU", OrderSide::Sell, false),
    ];
    let rows = rows_from_sources(
        Vec::new(),
        &orders,
        &[funding()],
        true,
        &[],
        risk_annotation(),
    );

    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .all(|row| row.origin == PositionOrigin::ExecutionLedger));
    assert_eq!(
        rows.iter()
            .find(|row| row.venue == "hyperliquid")
            .map(|row| row.side),
        Some(PositionSide::Long)
    );
    assert!(rows.iter().all(|row| row.paired_with.is_none()));
    assert!(rows.iter().all(|row| row.pair_evidence.is_none()));
    assert_eq!(summary_from_rows(&rows, 0).naked_position_count, 2);
}

#[test]
fn fresh_venue_marks_reprice_simulated_pair_pnl() {
    let orders = vec![
        dry_order("btc-long", "bybit", "BTCUSDT", OrderSide::Buy, false),
        dry_order("btc-short", "okx", "BTC-USDT-SWAP", OrderSide::Sell, false),
    ];
    let marks = HashMap::from([
        (("bybit".to_owned(), "BTC".to_owned()), 110.0),
        (("okx".to_owned(), "BTC".to_owned()), 90.0),
    ]);

    let rows = rows_from_sources_with_dry_run_marks(
        Vec::new(),
        &orders,
        &[],
        &[],
        DryRunRows {
            enabled: true,
            marks: &marks,
        },
        risk_annotation(),
    );

    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.unrealized_pnl_usd == 20.0));
    assert_eq!(
        rows.iter()
            .find(|row| row.venue == "bybit")
            .map(|row| row.mark_price),
        Some(110.0)
    );
    assert_eq!(
        rows.iter()
            .find(|row| row.venue == "okx")
            .map(|row| row.mark_price),
        Some(90.0)
    );
}

#[test]
fn dry_run_native_symbols_join_execution_pair_evidence() -> Result<(), &'static str> {
    let mut long = dry_order("ewy-long", "bybit", "EWYUSDT", OrderSide::Buy, false);
    let mut short = dry_order("ewy-short", "kucoin", "EWYUSDTM", OrderSide::Sell, false);
    long.filled_quantity = Some(1.0);
    short.filled_quantity = Some(1.0);
    let mut run = execution_run();
    run.long_leg.exchange = "bybit".into();
    run.long_leg.symbol = "EWYUSDT".into();
    run.short_leg.exchange = "kucoin".into();
    run.short_leg.symbol = "EWYUSDTM".into();
    let evidence = execution_run_pair_evidence(&run).ok_or("missing pair evidence")?;

    let rows = rows_from_sources(
        Vec::new(),
        &[long, short],
        &[],
        true,
        &evidence,
        risk_annotation(),
    );

    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.symbol == "EWY"));
    assert!(evidence.iter().all(|row| row.symbol == "EWY"));
    assert!(evidence.iter().all(|row| row.partner_symbol == "EWY"));
    assert!(rows.iter().all(|row| row.pair_evidence.is_some()));
    assert_eq!(summary_from_rows(&rows, 0).naked_position_count, 0);
    Ok(())
}

#[test]
fn reduce_only_dry_run_orders_offset_simulated_rows() {
    let orders = vec![
        dry_order("open", "binance", "BTCUSDT", OrderSide::Buy, false),
        dry_order("close", "binance", "BTC", OrderSide::Sell, true),
    ];
    let rows = rows_from_sources(Vec::new(), &orders, &[], true, &[], risk_annotation());

    assert!(rows.is_empty());
}

#[test]
fn annotates_server_side_funding_countdown_and_severity() {
    let mut near = position("long");
    near.liquidation_price = Some(93.0);
    near.next_funding_ms = Some(125_000);

    let rows = rows_from_positions(vec![near], &[], 5_000, 15.0, 8.0);

    assert_eq!(rows[0].severity, PositionSeverity::Danger);
    assert_eq!(rows[0].seconds_until_funding, Some(120));
}

#[test]
fn missing_or_invalid_liquidation_distance_is_never_marked_ok() {
    let mut missing = position("long");
    missing.liquidation_price = None;
    missing.liquidation_distance_pct = None;
    let rows = rows_from_positions(vec![missing], &[], 0, 15.0, 8.0);

    assert_eq!(rows[0].severity, PositionSeverity::Unknown);
    assert_eq!(
        position_severity(Some(f64::NAN), 15.0, 8.0),
        PositionSeverity::Unknown
    );
    // 距离已有符号化：负值 = mark 已越过强平价，是最强危险信号而非垃圾数据。
    assert_eq!(
        position_severity(Some(-1.0), 15.0, 8.0),
        PositionSeverity::Danger
    );
}

#[test]
fn positions_version_is_order_stable() {
    let first = rows_from_positions(vec![position("long"), position("short")], &[], 0, 15.0, 8.0);
    let mut second = first.clone();
    second.reverse();

    assert_eq!(positions_version(&first), positions_version(&second));
}

#[test]
fn positions_version_changes_when_position_changes() {
    let mut rows = rows_from_positions(vec![position("long")], &[], 0, 15.0, 8.0);
    let before = positions_version(&rows);
    rows[0].quantity += 1.0;

    assert_ne!(before, positions_version(&rows));
}

#[test]
fn positions_version_ignores_mark_price_ticks() {
    let mut rows = rows_from_positions(vec![position("long")], &[], 0, 15.0, 8.0);
    let before = positions_version(&rows);
    rows[0].mark_price += 3.0;

    assert_eq!(before, positions_version(&rows));
}
