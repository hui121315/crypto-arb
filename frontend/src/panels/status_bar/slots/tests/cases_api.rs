use super::super::*;
use super::fixtures::*;
use shared_types::UNRECORDED_EVIDENCE_MARKER;

#[test]
fn trading_api_missing_evidence_is_visible_and_degraded() {
    assert_eq!(api_label(None), "无证据");
    assert_eq!(api_slot_class(None, None), "slot degraded");
    assert!(api_title(None, None).contains("venue-operation-health 快照"));

    let no_api_rows = VenueOperationHealthSnapshot::new(
        vec![operation_row(
            "okx",
            "private_ws_order_stream",
            VenueOperationStatus::Ok,
        )],
        1_000,
    );
    assert_eq!(api_label(Some(&no_api_rows)), "无证据");
    assert!(api_degraded(Some(&no_api_rows), None));
    assert!(api_title(Some(&no_api_rows), None).contains("API 行"));
}

#[test]
fn trading_api_problem_overrides_healthy_operation() {
    let snapshot = VenueOperationHealthSnapshot::new(
        vec![operation_row(
            "okx",
            "credential_probe:balance_read",
            VenueOperationStatus::Ok,
        )],
        1_000,
    );
    let problem = api_problem();

    assert_eq!(api_label(Some(&snapshot)), "1可用/1配置");
    assert_eq!(
        api_label_with_problem(Some(&snapshot), Some(&problem)),
        "异常"
    );
    assert!(api_degraded(Some(&snapshot), Some(&problem)));
    assert!(api_title(Some(&snapshot), Some(&problem)).contains("request_id req-1"));
}

#[test]
fn trading_api_separates_configured_from_currently_usable() {
    let mut unconfigured = operation_row("gate", "private_read", VenueOperationStatus::Ok);
    unconfigured.configured = Some(false);
    let mut inherited_account_cache =
        operation_row("hyperliquid:xyz", "balance", VenueOperationStatus::Ok);
    inherited_account_cache.configured = None;
    let snapshot = VenueOperationHealthSnapshot::new(
        vec![
            operation_row("okx", "order_write", VenueOperationStatus::Blocked),
            operation_row("binance", "balance", VenueOperationStatus::Ok),
            unconfigured,
            inherited_account_cache,
        ],
        1_000,
    );

    assert_eq!(api_label(Some(&snapshot)), "1可用/2配置");
    assert_eq!(api_configured_count(&snapshot), 2);
    assert_eq!(api_usable_count(&snapshot), 1);
}

#[test]
fn unconfigured_optional_venue_does_not_degrade_healthy_configured_api_rows() {
    let configured = operation_row("bitget", "private_read", VenueOperationStatus::Ok);
    let mut missing_optional = operation_row("gate", "private_read", VenueOperationStatus::Blocked);
    missing_optional.configured = Some(false);
    missing_optional.message =
        "需配置 Gate：API Key（GATE_API_KEY）、API Secret（GATE_API_SECRET）".into();
    let mut inherited_account_cache =
        operation_row("hyperliquid:xyz", "balance", VenueOperationStatus::Ok);
    inherited_account_cache.configured = None;
    let snapshot = VenueOperationHealthSnapshot::new(
        vec![configured, missing_optional, inherited_account_cache],
        1_000,
    );

    assert_eq!(api_label(Some(&snapshot)), "1可用/1配置");
    assert!(!api_degraded(Some(&snapshot), None));
    let title = api_title(Some(&snapshot), None);
    assert!(title.contains("bitget"));
    assert!(title.contains("GATE_API_KEY"));
}

#[test]
fn live_mode_without_any_configured_api_row_remains_degraded() {
    let mut unconfigured = operation_row("gate", "private_read", VenueOperationStatus::Blocked);
    unconfigured.configured = Some(false);
    let snapshot = VenueOperationHealthSnapshot::new(vec![unconfigured], 1_000);

    assert_eq!(api_label(Some(&snapshot)), "需配置凭证");
    assert!(api_degraded(Some(&snapshot), None));
}

#[test]
fn trading_api_title_keeps_typed_operation_evidence() {
    let mut row = operation_row(
        "binance",
        "credential_probe:order_permission",
        VenueOperationStatus::Warn,
    );
    row.evidence = Some(VenueOperationEvidence {
        method: "GET".into(),
        path: "/fapi/v1/depth".into(),
        checked_at: UNRECORDED_EVIDENCE_MARKER.into(),
        doc_version: UNRECORDED_EVIDENCE_MARKER.into(),
        schema_hash: UNRECORDED_EVIDENCE_MARKER.into(),
        fixture_id: UNRECORDED_EVIDENCE_MARKER.into(),
        parser_test: UNRECORDED_EVIDENCE_MARKER.into(),
        request_builder_test: UNRECORDED_EVIDENCE_MARKER.into(),
        auth_kind: UNRECORDED_EVIDENCE_MARKER.into(),
        request_id: Some("rid-status-1".into()),
        request_context: vec!["symbol=BTCUSDT".into()],
        doc_urls: vec!["https://developers.binance.com/docs/derivatives".into()],
        use_cases: vec!["hot_path_fallback".into()],
        data_kinds: vec!["order_book".into()],
        rate_scopes: vec!["ip".into()],
        weight: 1,
    });
    row.latency_ms = Some(18);
    row.retry_after_ms = Some(60_000);
    let snapshot = VenueOperationHealthSnapshot::new(vec![row], 1_000);
    let title = api_title(Some(&snapshot), None);

    assert!(title.contains("HTTP延迟 18ms"));
    assert!(title.contains("retry 60000ms"));
    assert!(title.contains("request_id rid-status-1"));
    assert!(title.contains("context symbol=BTCUSDT"));
    assert!(title.contains("官方证据 order_book"));
    assert!(!title.contains("App API往返"));
}

#[test]
fn paper_mode_marks_missing_credentials_optional_and_lists_exact_fields() {
    let mut row = operation_row("okx", "private_read", VenueOperationStatus::Blocked);
    row.configured = Some(false);
    row.message = "需配置 OKX：API Key（OKX_API_KEY）、API Secret（OKX_API_SECRET）".into();
    let snapshot = VenueOperationHealthSnapshot::new(vec![row], 1_000);

    assert_eq!(
        api_label_for_environment(Some(&snapshot), Some(ExecutionEnvironment::Paper)),
        "模拟无需"
    );
    assert!(!api_degraded_for_environment(
        Some(&snapshot),
        None,
        Some(ExecutionEnvironment::Paper)
    ));
    let title = api_title_for_environment(Some(&snapshot), None, Some(ExecutionEnvironment::Paper));
    assert!(title.contains("模拟模式无需私有凭证"));
    assert!(title.contains("OKX_API_KEY"));
    assert!(title.contains("OKX_API_SECRET"));
}
