use super::*;

#[path = "tests_runtime/health.rs"]
mod health;
#[test]
fn runtime_rows_include_family_children_for_parent_venue() {
    let rows = vec![
        operation_row(
            "hyperliquid:xyz",
            "private_ws_account_stream",
            VenueOperationStatus::Ok,
        ),
        operation_row(
            "hyperliquid:km",
            "credential_probe:balance_read",
            VenueOperationStatus::Blocked,
        ),
        operation_row(
            "okx",
            "credential_probe:balance_read",
            VenueOperationStatus::Blocked,
        ),
    ];

    let selection =
        selected_runtime_health_rows(VenueOperationHealthSnapshot::new(rows, 10), "hyperliquid");
    assert_eq!(selection.total, 2);
    assert_eq!(selection.attention, 1);
    assert_eq!(selection.rows[0].venue, "hyperliquid:km");
    assert_eq!(selection.rows[0].status, VenueOperationStatus::Blocked);
    assert_eq!(selection.rows[1].venue, "hyperliquid:xyz");
}

#[test]
fn runtime_rows_keep_exact_builder_selection_scoped() {
    let rows = vec![
        operation_row(
            "hyperliquid",
            "credential_probe:balance_read",
            VenueOperationStatus::Ok,
        ),
        operation_row(
            "hyperliquid:xyz",
            "private_ws_account_stream",
            VenueOperationStatus::Ok,
        ),
        operation_row(
            "hyperliquid:km",
            "credential_probe:balance_read",
            VenueOperationStatus::Blocked,
        ),
    ];

    let selection = selected_runtime_health_rows(
        VenueOperationHealthSnapshot::new(rows, 10),
        "hyperliquid:xyz",
    );
    assert_eq!(selection.total, 1);
    assert_eq!(selection.attention, 0);
    assert_eq!(selection.rows[0].venue, "hyperliquid:xyz");
}

#[test]
fn runtime_rows_keep_all_matches_for_table_pagination() {
    let rows = (0..(RUNTIME_HEALTH_PAGE_SIZE + 5))
        .map(|index| {
            operation_row(
                "okx",
                &format!("credential_probe:balance_read:{index}"),
                VenueOperationStatus::Warn,
            )
        })
        .collect::<Vec<_>>();

    let selection =
        selected_runtime_health_rows(VenueOperationHealthSnapshot::new(rows, 10), "okx");

    assert_eq!(selection.total, RUNTIME_HEALTH_PAGE_SIZE + 5);
    assert_eq!(selection.rows.len(), RUNTIME_HEALTH_PAGE_SIZE + 5);
}

#[test]
fn runtime_selection_tracks_order_finality_for_selected_venue() {
    let mut finality = operation_row("okx", "order_finality", VenueOperationStatus::Warn);
    finality.source = "run_finality.refresh_pending_runs".to_owned();
    finality.problem = Some(
        ApiProblem::new("ORDER_FINALITY_REMOTE_MISSING", "远端订单暂未返回终态")
            .with_request_id(Some("req-finality".to_owned()))
            .with_retry_after_ms(Some(5_000))
            .with_source("run_finality"),
    );
    finality.evidence = Some(operation_evidence("req-finality"));
    let rows = vec![
        operation_row(
            "okx",
            "credential_probe:order_permission",
            VenueOperationStatus::Blocked,
        ),
        finality,
        operation_row("bybit", "order_finality", VenueOperationStatus::Blocked),
    ];

    let selection =
        selected_runtime_health_rows(VenueOperationHealthSnapshot::new(rows, 10), "okx");
    assert!(selection.trading_evidence.order_finality.is_some());
    if let Some(finality) = selection.trading_evidence.order_finality.as_ref() {
        assert_eq!(finality.venue, "okx");
        assert_eq!(finality.operation, "order_finality");
        assert!(runtime_health_message(finality).contains("request_id req-finality"));
        assert!(runtime_evidence_summary(finality).contains("req-finality"));
    }
}

#[test]
fn runtime_selection_leaves_order_finality_missing_when_not_recorded() {
    let rows = vec![operation_row(
        "okx",
        "credential_probe:balance_read",
        VenueOperationStatus::Ok,
    )];

    let selection =
        selected_runtime_health_rows(VenueOperationHealthSnapshot::new(rows, 10), "okx");

    assert!(selection.trading_evidence.order_finality.is_none());
}

#[test]
fn runtime_selection_pins_trading_runtime_evidence_for_selected_venue() {
    let mut order_permission = operation_row(
        "okx",
        "credential_probe:order_permission",
        VenueOperationStatus::Unknown,
    );
    order_permission.evidence = Some(operation_evidence("req-permission"));
    let rows = vec![
        order_permission,
        operation_row("okx", "order_write", VenueOperationStatus::Ok),
        operation_row("okx", "private_ws_order_stream", VenueOperationStatus::Warn),
        operation_row("okx", "order_finality", VenueOperationStatus::Ok),
        operation_row("bybit", "order_write", VenueOperationStatus::Blocked),
    ];

    let selection =
        selected_runtime_health_rows(VenueOperationHealthSnapshot::new(rows, 10), "okx");

    assert_eq!(trading_runtime_rows(&selection.trading_evidence).len(), 4);
    assert_eq!(
        selection
            .trading_evidence
            .order_permission
            .as_ref()
            .map(|row| row.operation.as_str()),
        Some("credential_probe:order_permission")
    );
    assert_eq!(
        selection
            .trading_evidence
            .order_write
            .as_ref()
            .map(|row| row.operation.as_str()),
        Some("order_write")
    );
    assert_eq!(
        selection
            .trading_evidence
            .private_order_stream
            .as_ref()
            .map(|row| row.operation.as_str()),
        Some("private_ws_order_stream")
    );
    assert_eq!(trading_runtime_ready_count(&selection.trading_evidence), 2);
    assert_eq!(
        trading_runtime_attention_count(&selection.trading_evidence),
        2
    );
    assert_eq!(
        trading_runtime_status_label(&selection.trading_evidence),
        "当前状态降级"
    );
    assert!(trading_runtime_summary("okx", &selection.trading_evidence).contains("2/4 正常"));
    let order_permission = selection.trading_evidence.order_permission.as_ref();
    assert!(order_permission.is_some());
    assert_eq!(
        order_permission.map(runtime_request_id_label),
        Some("request_id req-permission".to_owned())
    );
}

#[test]
fn runtime_operation_label_uses_shared_kind_label() {
    assert_eq!(runtime_operation_label("order_finality"), "订单终态回查");
    assert_eq!(runtime_operation_label("custom_probe"), "custom_probe");
}

#[test]
fn runtime_dataset_key_ignores_freshness_but_tracks_status() {
    let mut selection = selected_runtime_health_rows(
        VenueOperationHealthSnapshot::new(
            vec![operation_row(
                "okx",
                "http_rest:GET /api/v5/account/balance",
                VenueOperationStatus::Warn,
            )],
            10,
        ),
        "okx",
    );
    let baseline = runtime_selection_dataset_key("okx", &selection);
    selection.rows[0].freshness_ms = Some(9_999);
    selection.rows[0].latency_ms = Some(88);
    selection.rows[0].latency_p95_ms = Some(120);
    assert_eq!(
        runtime_sample(&selection.rows[0]),
        "1/1 · HTTP RTT 88ms · p95 120ms"
    );
    assert_eq!(baseline, runtime_selection_dataset_key("okx", &selection));
    selection.rows[0].status = VenueOperationStatus::Blocked;
    assert_ne!(baseline, runtime_selection_dataset_key("okx", &selection));
}

fn operation_row(
    venue: &str,
    operation: &str,
    status: VenueOperationStatus,
) -> VenueOperationHealth {
    VenueOperationHealth {
        venue: venue.to_owned(),
        operation: operation.to_owned(),
        status,
        source: "credential_probe".to_owned(),
        message: "checked".to_owned(),
        supported: Some(true),
        configured: Some(true),
        requested: Some(1),
        rows: Some(1),
        freshness_ms: Some(100),
        retry_after_ms: None,
        latency_ms: Some(12),
        latency_p95_ms: None,
        error: None,
        evidence: None,
        problem: None,
        observed_at_ms: 10,
    }
}

fn operation_evidence(request_id: &str) -> VenueOperationEvidence {
    VenueOperationEvidence {
        method: "GET".to_owned(),
        path: "/api/v5/account/balance".to_owned(),
        checked_at: "2026-06-05".to_owned(),
        doc_version: "v5".to_owned(),
        schema_hash: "schema".to_owned(),
        fixture_id: "fixture".to_owned(),
        parser_test: "parser".to_owned(),
        request_builder_test: "builder".to_owned(),
        auth_kind: "hmac".to_owned(),
        request_id: Some(request_id.to_owned()),
        request_context: vec!["balance".to_owned()],
        doc_urls: vec!["https://www.okx.com/docs".to_owned()],
        use_cases: vec!["balance".to_owned()],
        data_kinds: vec!["account".to_owned()],
        rate_scopes: vec!["private".to_owned()],
        weight: 1,
    }
}
