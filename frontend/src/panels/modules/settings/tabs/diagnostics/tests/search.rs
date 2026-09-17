use super::*;

#[test]
fn operation_health_rows_are_prioritized_before_paging() {
    let rows = vec![
        operation_row("binance", "balance", VenueOperationStatus::Ok),
        operation_row("okx", "order_write", VenueOperationStatus::Blocked),
        operation_row(
            "gate",
            "private_ws_order_stream",
            VenueOperationStatus::Warn,
        ),
        operation_row("kucoin", "positions", VenueOperationStatus::Unknown),
    ];

    let ordered = prioritized_operation_health_rows(rows);

    assert_eq!(ordered[0].venue, "okx");
    assert_eq!(ordered[1].venue, "gate");
    assert_eq!(ordered[2].venue, "kucoin");
    assert_eq!(ordered[3].venue, "binance");
}

#[test]
fn operation_health_search_filters_after_priority_order() {
    let rows = vec![
        operation_row("binance", "balance", VenueOperationStatus::Ok),
        operation_row("okx", "order_write", VenueOperationStatus::Blocked),
        operation_row("gate", "positions", VenueOperationStatus::Warn),
    ];

    let ordered = filtered_operation_health_rows(rows, "ORDER", HealthStatusFilter::All);

    assert_eq!(ordered.len(), 1);
    assert_eq!(ordered[0].venue, "okx");
}

#[test]
fn operation_health_filter_keeps_attention_rows() {
    let rows = vec![
        operation_row("binance", "balance", VenueOperationStatus::Ok),
        operation_row("okx", "order_write", VenueOperationStatus::Blocked),
        operation_row("gate", "positions", VenueOperationStatus::Warn),
    ];

    let ordered = filtered_operation_health_rows(rows, "", HealthStatusFilter::Attention);

    assert_eq!(ordered.len(), 2);
    assert_eq!(ordered[0].venue, "okx");
    assert_eq!(ordered[1].venue, "gate");
}

#[test]
fn operation_health_search_matches_problem_context() {
    let mut row = operation_row("kucoin", "private_read", VenueOperationStatus::Blocked);
    row.problem = Some(shared_types::ApiProblem::new(
        "CREDENTIAL_PERMISSION_DENIED",
        "missing permission",
    ));

    assert!(operation_health_matches(&row, "permission"));
    assert!(operation_health_matches(
        &row,
        "credential_permission_denied"
    ));
}

#[test]
fn operation_health_separates_configuration_from_current_usability() {
    let mut configured_but_blocked =
        operation_row("okx", "order_write", VenueOperationStatus::Blocked);
    configured_but_blocked.configured = Some(true);
    configured_but_blocked.supported = Some(true);
    assert_eq!(
        operation_configured_label(&configured_but_blocked),
        "配置存在"
    );
    assert_eq!(operation_usable_label(&configured_but_blocked), "不可用");
    assert_eq!(operation_capability_label(&configured_but_blocked), "支持");

    let mut healthy_but_unconfigured =
        operation_row("gate", "private_read", VenueOperationStatus::Ok);
    healthy_but_unconfigured.configured = Some(false);
    assert_eq!(
        operation_configured_label(&healthy_but_unconfigured),
        "配置缺失"
    );
    assert_eq!(operation_usable_label(&healthy_but_unconfigured), "不可用");

    let usable = operation_row("binance", "balance", VenueOperationStatus::Ok);
    assert_eq!(operation_usable_label(&usable), "可用");
}

#[test]
fn operation_health_capability_is_distinct_and_searchable() {
    let mut unsupported = operation_row("kraken", "order_write", VenueOperationStatus::Unsupported);
    unsupported.configured = Some(true);
    unsupported.supported = Some(false);

    assert_eq!(operation_configured_label(&unsupported), "配置存在");
    assert_eq!(operation_capability_label(&unsupported), "不支持");
    assert_eq!(operation_usable_label(&unsupported), "不可用");
    assert!(operation_health_matches(&unsupported, "配置存在"));
    assert!(operation_health_matches(&unsupported, "不支持"));
    assert!(operation_health_matches(&unsupported, "不可用"));

    let mut undeclared = operation_row("local", "rest_metadata", VenueOperationStatus::Ok);
    undeclared.supported = None;
    assert!(undeclared.capability_supported());
    assert_eq!(operation_capability_label(&undeclared), "未声明（按支持）");
    assert!(operation_health_matches(&undeclared, "未声明（按支持）"));
}

#[test]
fn operation_health_search_matches_shared_class_and_kind_labels() {
    let order_permission = operation_row(
        "okx",
        "credential_probe:order_permission",
        VenueOperationStatus::Unknown,
    );
    let storage = operation_row("local", "storage:history", VenueOperationStatus::Warn);
    let private_ws = operation_row(
        "gate",
        "private_ws_order_stream",
        VenueOperationStatus::Blocked,
    );

    assert!(operation_health_matches(&order_permission, "交易权限验证"));
    assert!(operation_health_matches(&order_permission, "api"));
    assert!(operation_health_matches(&storage, "存储"));
    assert!(operation_health_matches(&private_ws, "私有 ws"));
    assert!(operation_health_operation_label(&private_ws).contains("私有订单流"));

    let instruments = operation_row("binance", "rest_instrument_specs", VenueOperationStatus::Ok);
    let ws_snapshot = operation_row("bybit", "ws_ticker_snapshot", VenueOperationStatus::Warn);
    let opportunity = operation_row("system", "opportunity_snapshot", VenueOperationStatus::Ok);
    assert!(operation_health_matches(&instruments, "合约规格"));
    assert!(operation_health_matches(&ws_snapshot, "行情快照"));
    assert!(operation_health_matches(&opportunity, "机会原子快照"));
}

#[test]
fn history_memory_fallback_is_visible_and_searchable() {
    let mut row = operation_row("system", "storage:history", VenueOperationStatus::Warn);
    row.source = "history_store".into();
    row.message = "历史存储 backend=memory 非持久化，仅适合本地/临时历史".into();
    row.problem = Some(shared_types::ApiProblem::new(
        "HISTORY_STORE_FALLBACK",
        "postgres unavailable; using memory fallback",
    ));

    assert!(operation_health_matches(&row, "memory"));
    assert!(operation_health_matches(&row, "非持久化"));
    assert!(operation_health_matches(&row, "fallback"));
    assert!(operation_health_operation_label(&row).contains("历史存储"));
}

#[test]
fn operation_health_operation_label_keeps_unknown_rows_fail_safe() {
    let row = operation_row("okx", "venue_magic", VenueOperationStatus::Unknown);
    let label = operation_health_operation_label(&row);

    assert!(label.contains("未知 / 未知操作"));
    assert!(label.contains("venue_magic"));
    assert!(operation_health_matches(&row, "未知操作"));
}

#[test]
fn operation_health_search_matches_evidence_context() {
    let mut row = operation_row("okx", "private_read", VenueOperationStatus::Unknown);
    row.evidence = Some(VenueOperationEvidence {
        method: "GET".into(),
        path: "/api/v5/account/balance".into(),
        checked_at: UNRECORDED_EVIDENCE_MARKER.into(),
        doc_version: UNRECORDED_EVIDENCE_MARKER.into(),
        schema_hash: UNRECORDED_EVIDENCE_MARKER.into(),
        fixture_id: UNRECORDED_EVIDENCE_MARKER.into(),
        parser_test: UNRECORDED_EVIDENCE_MARKER.into(),
        request_builder_test: UNRECORDED_EVIDENCE_MARKER.into(),
        auth_kind: UNRECORDED_EVIDENCE_MARKER.into(),
        request_id: Some("req-42".into()),
        request_context: vec![
            "unified-account".into(),
            "live_place_remote_proof=missing".into(),
            "live_cancel_remote_proof=missing".into(),
            "does_not_grant_live_write=true".into(),
            "checked_at=not_recorded".into(),
            "doc_version=not_recorded".into(),
            "schema_hash=not_recorded".into(),
            "fixture_id=not_recorded".into(),
            "parser_test=not_recorded".into(),
            "request_builder_test=not_recorded".into(),
            "auth_kind=not_recorded".into(),
        ],
        doc_urls: vec!["https://www.okx.com/docs-v5/en/".into()],
        use_cases: vec!["balance probe".into()],
        data_kinds: vec!["account".into()],
        rate_scopes: vec!["user".into()],
        weight: 1,
    });

    assert!(operation_health_matches(&row, "unified-account"));
    assert!(operation_health_matches(&row, "live_place_remote_proof"));
    assert!(operation_health_matches(&row, "live_cancel_remote_proof"));
    assert!(operation_health_matches(&row, "does_not_grant_live_write"));
    assert!(operation_health_matches(&row, "balance probe"));
    assert!(operation_health_matches(&row, "checked_at"));
    assert!(operation_health_matches(&row, "doc_version=not_recorded"));
    assert!(operation_health_matches(&row, "schema_hash"));
    assert!(operation_health_matches(&row, "fixture_id=not_recorded"));
    assert!(operation_health_matches(&row, "parser_test"));
    assert!(operation_health_matches(
        &row,
        "request_builder_test=not_recorded"
    ));
    assert!(operation_health_matches(&row, "auth_kind"));
    assert!(operation_health_matches(&row, "req-42"));
}
