use super::super::*;
use super::fixtures::*;

#[test]
fn transport_rows_do_not_inflate_trading_api_count() {
    let mut transport = operation_row(
        "gate",
        "http_rest:GET /api/v4/orders",
        VenueOperationStatus::Warn,
    );
    transport.latency_ms = Some(35);
    transport.evidence = Some(transport_evidence("private_read"));
    let snapshot = VenueOperationHealthSnapshot::new(
        vec![
            operation_row("bybit", "positions", VenueOperationStatus::Ok),
            transport,
        ],
        1_000,
    );

    assert_eq!(api_label(Some(&snapshot)), "1可用/1配置");
    assert!(api_degraded(Some(&snapshot), None));
    let title = api_title(Some(&snapshot), None);
    assert!(title.contains("Transport：gate"));
    assert!(title.contains("HTTP RTT 35ms"));
    assert!(!title.contains("App API往返"));
}

#[test]
fn public_market_transport_warning_does_not_degrade_paper_trading_api() {
    let mut private_read = operation_row("binance", "private_read", VenueOperationStatus::Blocked);
    private_read.configured = Some(false);
    let mut public_transport = operation_row(
        "gate",
        "http_rest:GET /api/v4/spot/tickers",
        VenueOperationStatus::Warn,
    );
    public_transport.evidence = Some(transport_evidence("public"));
    let snapshot = VenueOperationHealthSnapshot::new(vec![private_read, public_transport], 1_000);

    assert_eq!(
        api_label_for_environment(Some(&snapshot), Some(ExecutionEnvironment::Paper)),
        "模拟无需"
    );
    assert!(!api_degraded_for_environment(
        Some(&snapshot),
        None,
        Some(ExecutionEnvironment::Paper)
    ));
    assert!(
        !api_title_for_environment(Some(&snapshot), None, Some(ExecutionEnvironment::Paper))
            .contains("Transport：gate")
    );
}

fn transport_evidence(auth_kind: &str) -> VenueOperationEvidence {
    VenueOperationEvidence {
        method: "GET".into(),
        path: "/test".into(),
        checked_at: "2026-07-20".into(),
        doc_version: "test".into(),
        schema_hash: "test".into(),
        fixture_id: "test".into(),
        parser_test: "test".into(),
        request_builder_test: "test".into(),
        auth_kind: auth_kind.into(),
        request_id: None,
        request_context: Vec::new(),
        doc_urls: Vec::new(),
        use_cases: Vec::new(),
        data_kinds: Vec::new(),
        rate_scopes: Vec::new(),
        weight: 1,
    }
}

#[test]
fn unknown_status_degrades_but_unknown_operation_is_not_counted() {
    let status_unknown = VenueOperationHealthSnapshot::new(
        vec![operation_row(
            "kucoin",
            "private_read",
            VenueOperationStatus::Unknown,
        )],
        1_000,
    );
    assert_eq!(api_label(Some(&status_unknown)), "0可用/1配置");
    assert!(api_degraded(Some(&status_unknown), None));

    let operation_unknown = VenueOperationHealthSnapshot::new(
        vec![operation_row(
            "kucoin",
            "made_up_operation",
            VenueOperationStatus::Ok,
        )],
        1_000,
    );
    assert_eq!(api_label(Some(&operation_unknown)), "无证据");
    assert!(api_degraded(Some(&operation_unknown), None));
}

#[test]
fn order_permission_probe_is_part_of_trading_api() {
    let snapshot = VenueOperationHealthSnapshot::new(
        vec![
            operation_row(
                "okx",
                "credential_probe:balance_read",
                VenueOperationStatus::Ok,
            ),
            operation_row(
                "okx",
                "credential_probe:order_permission",
                VenueOperationStatus::Unknown,
            ),
        ],
        1_000,
    );

    assert_eq!(api_label(Some(&snapshot)), "1可用/2配置");
    assert!(api_title(Some(&snapshot), None).contains("credential_probe:order_permission"));
}
