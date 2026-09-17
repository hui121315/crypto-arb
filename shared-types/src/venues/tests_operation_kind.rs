use super::*;

#[test]
fn operation_kind_parses_status_bar_rows() {
    assert_eq!(
        VenueOperationKind::parse("http_rest:GET /api/v5/account/balance"),
        VenueOperationKind::HttpRest
    );
    assert_eq!(
        VenueOperationKind::parse("host_gate:api.binance.com"),
        VenueOperationKind::HostGate
    );
    assert_eq!(
        VenueOperationKind::parse("rate_limiter:binance"),
        VenueOperationKind::RateLimiter
    );
    assert!(VenueOperationKind::parse("order_write").is_api_status_row());
    assert!(VenueOperationKind::parse("order_finality").is_api_status_row());
    assert!(VenueOperationKind::parse("private_ws_order_stream").is_private_ws_status_row());
    assert_eq!(
        VenueOperationKind::parse("app_ws_broadcast:orders"),
        VenueOperationKind::AppWsBroadcast
    );
    assert_eq!(
        VenueOperationKind::AppWsBroadcast.class(),
        VenueOperationClass::AppWs
    );
    assert_eq!(
        VenueOperationKind::parse("rest_funding_rates").class(),
        VenueOperationClass::MarketData
    );
    assert_eq!(
        VenueOperationKind::parse("rest_index_compositions").class(),
        VenueOperationClass::MarketData
    );
    assert_eq!(
        VenueOperationKind::parse("rest_metadata").class(),
        VenueOperationClass::MarketData
    );
    assert_eq!(
        VenueOperationKind::parse("rest_perp_tickers").class(),
        VenueOperationClass::MarketData
    );
    assert_eq!(
        VenueOperationKind::parse("ws_funding").class(),
        VenueOperationClass::MarketData
    );
    assert_eq!(
        VenueOperationKind::parse("ws_ticker").class(),
        VenueOperationClass::MarketData
    );
}

#[test]
fn operation_kind_parses_spot_ws_snapshot_as_market_data() {
    assert_eq!(
        VenueOperationKind::parse(OP_WS_SPOT_SNAPSHOT),
        VenueOperationKind::WsSpotSnapshot
    );
    assert_eq!(
        VenueOperationKind::WsSpotSnapshot.class(),
        VenueOperationClass::MarketData
    );
}

#[test]
fn operation_kind_separates_trading_api_from_transport_rows() {
    assert!(VenueOperationKind::parse("order_write").is_trading_api_status_row());
    assert!(VenueOperationKind::parse("order_finality").is_trading_api_status_row());
    assert!(!VenueOperationKind::parse("order_write").is_api_transport_status_row());
    assert!(
        !VenueOperationKind::parse("http_rest:GET /api/v5/account/balance")
            .is_trading_api_status_row()
    );
    assert!(
        VenueOperationKind::parse("http_rest:GET /api/v5/account/balance")
            .is_api_transport_status_row()
    );
    assert!(!VenueOperationKind::parse("host_gate:api.binance.com").is_trading_api_status_row());
    assert!(VenueOperationKind::parse("host_gate:api.binance.com").is_api_transport_status_row());
    assert!(!VenueOperationKind::parse("rate_limiter:binance").is_trading_api_status_row());
    assert!(VenueOperationKind::parse("rate_limiter:binance").is_api_transport_status_row());
    assert!(!VenueOperationKind::parse("private_ws_order_stream").is_api_transport_status_row());
}

#[test]
fn operation_kind_parses_storage_rows() {
    assert_eq!(
        VenueOperationKind::parse("storage:order_snapshot").class(),
        VenueOperationClass::Storage
    );
    assert_eq!(
        VenueOperationKind::parse("storage:audit_log").class(),
        VenueOperationClass::Storage
    );
    assert_eq!(
        VenueOperationKind::parse("storage:trading_sql_migrations").class(),
        VenueOperationClass::Storage
    );
    assert_eq!(
        VenueOperationKind::parse("storage:trading_sql_ledger").class(),
        VenueOperationClass::Storage
    );
    assert_eq!(
        VenueOperationKind::parse("storage:watchlist_alerts").class(),
        VenueOperationClass::Storage
    );
}

#[test]
fn operation_kind_exposes_static_operation_strings() {
    assert_eq!(
        VenueOperationKind::OrderFinality.as_str(),
        Some(OP_ORDER_FINALITY)
    );
    assert_eq!(
        VenueOperationKind::PrivateWsOrderStream.as_str(),
        Some(OP_PRIVATE_WS_ORDER_STREAM)
    );
    assert_eq!(
        VenueOperationKind::RestFundingRates.as_str(),
        Some(OP_REST_FUNDING_RATES)
    );
    assert_eq!(
        VenueOperationKind::RestMetadata.as_str(),
        Some(OP_REST_METADATA)
    );
    assert_eq!(VenueOperationKind::WsTicker.as_str(), Some(OP_WS_TICKER));
    assert_eq!(
        VenueOperationKind::WsSpotSnapshot.as_str(),
        Some(OP_WS_SPOT_SNAPSHOT)
    );
    assert_eq!(
        VenueOperationKind::StorageOrderSnapshot.as_str(),
        Some(OP_STORAGE_ORDER_SNAPSHOT)
    );
    assert_eq!(
        VenueOperationKind::StorageAuditLog.as_str(),
        Some(OP_STORAGE_AUDIT_LOG)
    );
    assert_eq!(
        VenueOperationKind::StorageTradingSqlMigrations.as_str(),
        Some(OP_STORAGE_TRADING_SQL_MIGRATIONS)
    );
    assert_eq!(
        VenueOperationKind::StorageTradingSqlLedger.as_str(),
        Some(OP_STORAGE_TRADING_SQL_LEDGER)
    );
    assert_eq!(
        VenueOperationKind::StorageWatchlistAlerts.as_str(),
        Some(OP_STORAGE_WATCHLIST_ALERTS)
    );
    assert_eq!(VenueOperationKind::HttpRest.as_str(), None);
    assert_eq!(
        credential_probe_operation("order_permission"),
        "credential_probe:order_permission"
    );
}

#[test]
fn operation_kind_exposes_product_labels() {
    assert_eq!(VenueOperationClass::Api.label_zh(), "API");
    assert!(VenueOperationClass::PrivateWs
        .product_explanation_zh()
        .contains("WebSocket"));
    assert_eq!(VenueOperationKind::OrderFinality.label_zh(), "订单终态回查");
    assert!(VenueOperationKind::OrderFinality
        .product_explanation_zh()
        .contains("ExecutionRun"));
    assert_eq!(VenueOperationKind::OrderWrite.label_zh(), "写单运行态");
    assert!(VenueOperationKind::OrderWrite
        .product_explanation_zh()
        .contains("live place/cancel/finality"));
    assert!(!VenueOperationKind::OrderWrite
        .product_explanation_zh()
        .contains("具备实盘写单能力"));
    assert_eq!(
        VenueOperationKind::CredentialProbeOrderPermission.label_zh(),
        "交易权限验证"
    );
    assert!(VenueOperationKind::Unknown
        .product_explanation_zh()
        .contains("排除"));
}

#[test]
fn operation_kind_keeps_unknown_rows_fail_safe() {
    assert_eq!(
        VenueOperationKind::parse("credential_probe:made_up"),
        VenueOperationKind::Unknown
    );
    assert!(!VenueOperationKind::parse("credential_probe:made_up").is_api_status_row());
    assert!(!VenueOperationKind::parse("unknown_operation").is_private_ws_status_row());
    assert_eq!(
        VenueOperationKind::parse("unknown_operation").class(),
        VenueOperationClass::Unknown
    );
}

#[test]
fn operation_kind_separates_market_hot_path_from_recovery_rows() {
    assert!(VenueOperationKind::WsSpotSnapshot.is_market_data_execution_core_status_row());
    assert!(VenueOperationKind::WsFundingSnapshot.is_market_data_execution_core_status_row());
    assert!(VenueOperationKind::RestInstrumentSpecs.is_market_data_execution_core_status_row());
    assert!(!VenueOperationKind::RestPerpTickers.is_market_data_execution_core_status_row());
    assert!(VenueOperationKind::RestPerpTickers.is_market_data_recovery_status_row());
    assert!(VenueOperationKind::RestTickerFallback.is_market_data_recovery_status_row());
    assert!(!VenueOperationKind::WsTickerSnapshot.is_market_data_recovery_status_row());
}

#[test]
fn operation_kind_includes_verified_private_read_probes_in_api_status() {
    assert!(VenueOperationKind::parse("credential_probe:order_permission").is_api_status_row());
    assert!(VenueOperationKind::parse("credential_probe:balance_read").is_api_status_row());
    assert!(VenueOperationKind::parse("credential_probe:positions_read").is_api_status_row());
    assert!(VenueOperationKind::parse("credential_probe:open_orders_read").is_api_status_row());
    assert!(VenueOperationKind::parse("credential_probe:account_mode_read").is_api_status_row());
    assert!(
        VenueOperationKind::parse("credential_probe:order_permission").is_trading_api_status_row()
    );
    assert!(VenueOperationKind::parse("credential_probe:balance_read").is_trading_api_status_row());
    assert!(
        VenueOperationKind::parse("credential_probe:positions_read").is_trading_api_status_row()
    );
    assert!(
        VenueOperationKind::parse("credential_probe:open_orders_read").is_trading_api_status_row()
    );
    assert!(
        VenueOperationKind::parse("credential_probe:account_mode_read").is_trading_api_status_row()
    );
    assert!(
        VenueOperationKind::from_credential_probe_kind("balance_read")
            .credential_probe_requires_private_read()
    );
    assert!(
        VenueOperationKind::from_credential_probe_kind("order_permission")
            .credential_probe_requires_order_write()
    );
}
