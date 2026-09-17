use super::*;

#[test]
fn operation_sample_includes_http_latency() {
    let mut row = operation_row(
        "binance",
        "http_rest:GET /fapi/v1/depth",
        VenueOperationStatus::Ok,
    );
    row.requested = Some(12);
    row.rows = Some(3);
    row.latency_ms = Some(8);
    row.latency_p95_ms = Some(25);

    assert_eq!(operation_sample(&row), "3/12 · HTTP RTT 8ms · p95≤25ms");
    assert!(operation_health_matches(&row, "latency_p95_ms=25ms"));
    assert!(operation_health_matches(&row, "http_rtt_ms=8ms"));
    assert!(operation_health_matches(&row, "http_rtt_p95_ms=25ms"));
}

#[test]
fn operation_health_message_exposes_row_retry_after() {
    let mut row = operation_row(
        "gate",
        "host_gate:futures.gateio.ws",
        VenueOperationStatus::Warn,
    );
    row.retry_after_ms = Some(60_000);

    let message = operation_health_message(&row);
    let title = operation_health_title(&row);

    assert!(message.contains("runtime retry 60000ms"));
    assert!(title.contains("runtime retry 60000ms"));
    assert!(operation_health_matches(&row, "retry_after_ms=60000ms"));
}

#[test]
fn operation_health_message_exposes_typed_problem_context() {
    let mut row = operation_row("okx", "private_read", VenueOperationStatus::Blocked);
    row.problem = Some(
        shared_types::ApiProblem::new("CREDENTIAL_PERMISSION_DENIED", "missing permission")
            .with_status(403)
            .with_request_id(Some("req-problem-1".into()))
            .with_retry_after_ms(Some(2_000))
            .with_source("credential_validation"),
    );

    let message = operation_health_message(&row);
    let title = operation_health_title(&row);

    assert!(message.contains("CREDENTIAL_PERMISSION_DENIED"));
    assert!(message.contains("HTTP 403"));
    assert!(message.contains("request_id req-problem-1"));
    assert!(message.contains("source credential_validation"));
    assert!(message.contains("retry 2000ms"));
    assert!(title.contains("CREDENTIAL_PERMISSION_DENIED"));
    assert!(operation_health_matches(
        &row,
        "problem_request_id=req-problem-1"
    ));
    assert!(operation_health_matches(
        &row,
        "problem_retry_after_ms=2000ms"
    ));
    assert!(operation_health_matches(
        &row,
        "problem_source=credential_validation"
    ));
}

#[test]
fn diagnostics_stale_problem_message_exposes_resource_problem_context() {
    let problem = shared_types::ApiProblem::new("VENUE_HEALTH_RATE_LIMITED", "rate limited")
        .with_status(429)
        .with_request_id(Some("req-snapshot-1".into()))
        .with_retry_after_ms(Some(5_000))
        .with_source("operation-health");

    let message = diagnostics_stale_problem_message("行情诊断刷新失败", &problem);

    assert!(message.contains("行情诊断刷新失败"));
    assert!(message.contains("保留上次数据"));
    assert!(message.contains("VENUE_HEALTH_RATE_LIMITED"));
    assert!(message.contains("HTTP 429"));
    assert!(message.contains("request_id req-snapshot-1"));
    assert!(message.contains("source operation-health"));
    assert!(message.contains("retry 5000ms"));
}

#[test]
fn funding_runtime_summary_keeps_cold_start_health_without_rows() {
    let envelope = shared_types::FundingRatesEnvelope {
        data: Vec::new(),
        health: shared_types::MarketDataHealth {
            quality: shared_types::MarketDataQuality::Missing,
            source: shared_types::MarketDataSourceKind::RestColdStart,
            freshness_ms: None,
            retry_after_ms: None,
            last_error: Some("funding snapshot warming".into()),
            observed_at_ms: 1_000,
            coverage: None,
            problem: None,
        },
        retry_after_ms: None,
        row_cap: None,
        row_evidence: Vec::new(),
        fanout: Vec::new(),
    };

    let summary = funding_runtime_summary(&envelope);

    assert!(summary.contains("0 条 funding 行"));
    assert!(summary.contains("缺数据"));
    assert!(summary.contains("REST 冷启动"));
    assert!(summary.contains("funding snapshot warming"));
}

#[test]
fn funding_runtime_summary_exposes_typed_degraded_context() {
    let problem = shared_types::ApiProblem::new("FUNDING_REFRESH_RATE_LIMITED", "rate limited")
        .with_status(429)
        .with_request_id(Some("req-funding-1".into()))
        .with_retry_after_ms(Some(5_000))
        .with_source("funding_refresh");
    let envelope = shared_types::FundingRatesEnvelope {
        data: Vec::new(),
        health: shared_types::MarketDataHealth {
            quality: shared_types::MarketDataQuality::RateLimited,
            source: shared_types::MarketDataSourceKind::RestFallback,
            freshness_ms: Some(12_000),
            retry_after_ms: Some(5_000),
            last_error: None,
            observed_at_ms: 1_000,
            coverage: None,
            problem: Some(problem),
        },
        retry_after_ms: Some(8_000),
        row_cap: None,
        row_evidence: Vec::new(),
        fanout: Vec::new(),
    };

    let summary = funding_runtime_summary(&envelope);

    assert!(summary.contains("限频"));
    assert!(summary.contains("REST 兜底"));
    assert!(summary.contains("FUNDING_REFRESH_RATE_LIMITED"));
    assert!(summary.contains("HTTP 429"));
    assert!(summary.contains("request_id req-funding-1"));
    assert!(summary.contains("source funding_refresh"));
    assert!(summary.contains("5000ms 后重试"));
    assert!(summary.contains("envelope retry 8000ms"));
}

#[test]
fn operation_sample_prefers_probe_counts_over_static_credentials() {
    let mut row = operation_row(
        "binance",
        "credential_probe:order_permission",
        VenueOperationStatus::Unknown,
    );
    row.source = "credential_validation".to_owned();
    row.requested = Some(1);
    row.rows = Some(0);

    assert_eq!(operation_sample(&row), "0/1");
}

#[test]
fn operation_health_summary_reports_filter_count() {
    assert_eq!(
        operation_health_summary(3, 1, 8, "okx", HealthStatusFilter::Blocked),
        "最差优先 · 阻断 · 匹配 1 / 8 条 · 需关注 3"
    );
}

#[test]
fn status_pill_class_keeps_blockers_visually_distinct() {
    assert_eq!(
        status_pill_class(VenueOperationStatus::Blocked),
        "status-pill blocked"
    );
    assert_eq!(
        status_pill_class(VenueOperationStatus::Unknown),
        "status-pill pending"
    );
    assert_eq!(
        status_pill_class(VenueOperationStatus::Ok),
        "status-pill ready"
    );
}

#[test]
fn diagnostics_storage_keys_are_namespaced() {
    for key in [
        DIAGNOSTICS_ENV_PAGE_KEY,
        DIAGNOSTICS_HEALTH_PAGE_KEY,
        DIAGNOSTICS_ACCESS_PAGE_KEY,
        DIAGNOSTICS_STATUS_PAGE_KEY,
        DIAGNOSTICS_ROW_EVIDENCE_PAGE_KEY,
        DIAGNOSTICS_HEALTH_QUERY_KEY,
        DIAGNOSTICS_HEALTH_STATUS_KEY,
    ] {
        assert!(key.starts_with("crossline.settings.diagnostics."));
    }
}

#[test]
fn diagnostics_health_status_filter_round_trips_storage_key() {
    for filter in [
        HealthStatusFilter::All,
        HealthStatusFilter::Attention,
        HealthStatusFilter::Blocked,
        HealthStatusFilter::Warn,
        HealthStatusFilter::Unknown,
        HealthStatusFilter::Ok,
        HealthStatusFilter::Unsupported,
    ] {
        assert_eq!(stored_health_status_filter(filter.as_key()), Some(filter));
    }
    assert_eq!(
        stored_health_status_filter("not-real"),
        Some(HealthStatusFilter::All)
    );
}

#[test]
fn diagnostics_health_query_storage_is_trimmed_and_bounded() {
    assert_eq!(stored_health_query(" okx ").as_deref(), Some("okx"));
    let long = "x".repeat(160);

    assert_eq!(
        stored_health_query(&long).map(|value| value.len()),
        Some(120)
    );
}

#[test]
fn api_base_apply_confirmation_is_explicit_without_reload_copy() {
    assert!(api_base_apply_confirmed("apply"));
    assert!(api_base_apply_confirmed(" apply "));
    assert!(!api_base_apply_confirmed("reload"));
    assert!(!api_base_apply_confirmed(""));
}

#[test]
fn auth_status_copy_never_echoes_token() {
    assert_eq!(auth_status_label(true), "Token 已填写");
    assert_eq!(auth_status_label(false), "Token 未填写");
    assert!(!auth_apply_message(true).contains("secret"));
    assert!(auth_apply_message(true).contains("ticket"));
    assert!(auth_apply_message(false).contains("不会发起未鉴权订阅"));
    assert!(!auth_apply_message(true).contains("等待后端握手契约"));
}
