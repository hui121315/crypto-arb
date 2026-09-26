//! 运行态健康与证据文本的测试。

use super::*;

#[test]
fn runtime_message_exposes_problem_request_and_retry() {
    let mut row = operation_row(
        "okx",
        "credential_probe:order_permission",
        VenueOperationStatus::Blocked,
    );
    row.retry_after_ms = Some(2_000);
    row.problem = Some(
        ApiProblem::new("ORDER_PERMISSION_UNKNOWN", "权限探针失败")
            .with_request_id(Some("req-7".to_owned()))
            .with_retry_after_ms(Some(1_000))
            .with_source("credential_probe"),
    );

    let message = runtime_health_message(&row);

    assert!(message.contains("ORDER_PERMISSION_UNKNOWN"));
    assert!(message.contains("request_id req-7"));
    assert!(message.contains("source credential_probe"));
    assert!(message.contains("retry 1000ms"));
    assert!(message.contains("runtime retry 2000ms"));
}

#[test]
fn runtime_stale_message_keeps_request_and_retry_context() {
    let problem = ApiProblem::new("RATE_LIMITED", "运行状态验证限流")
        .with_request_id(Some("req-runtime".to_owned()))
        .with_retry_after_ms(Some(4_000));

    let message = runtime_stale_message(Some(&problem)).unwrap_or_default();

    assert!(!message.is_empty());
    assert!(message.contains("运行状态验证刷新失败，显示上次结果"));
    assert!(message.contains("request_id req-runtime"));
    assert!(message.contains("retry 4000ms"));
}

#[test]
fn runtime_stale_message_is_absent_without_problem() {
    assert!(runtime_stale_message(None).is_none());
}

#[test]
fn runtime_evidence_summary_uses_official_endpoint_identity() {
    let mut row = operation_row(
        "okx",
        "credential_probe:balance_read",
        VenueOperationStatus::Ok,
    );
    row.evidence = Some(operation_evidence("req-9"));

    assert_eq!(
        runtime_evidence_summary(&row),
        "GET /api/v5/account/balance · request_id req-9"
    );
    let detail = runtime_evidence_detail(&row);
    assert!(detail.contains("request_id req-9"));
    assert!(detail.contains("doc v5"));
    assert!(detail.contains("parser parser"));
    assert!(detail.contains("builder builder"));
    assert!(detail.contains("docs https://www.okx.com/docs"));
}

#[test]
fn runtime_evidence_detail_keeps_order_finality_sample_context_visible() {
    let mut row = operation_row("okx", "order_finality", VenueOperationStatus::Blocked);
    let mut evidence = operation_evidence("req-finality");
    evidence.request_context = vec![
        "operation=order_finality".to_owned(),
        "sample_raw_order_id=exchange-1".to_owned(),
        "sample_internal_order_id=internal-1".to_owned(),
        "sample_error=upstream timeout".to_owned(),
    ];
    row.evidence = Some(evidence);

    let detail = runtime_evidence_detail(&row);

    assert!(detail.contains("sample_raw_order_id=exchange-1"));
    assert!(detail.contains("sample_internal_order_id=internal-1"));
    assert!(detail.contains("sample_error=upstream timeout"));
}

#[test]
fn runtime_evidence_summary_surfaces_live_write_probe_context() {
    let mut row = operation_row(
        "okx",
        "credential_probe:order_permission",
        VenueOperationStatus::Unknown,
    );
    let mut evidence = operation_evidence("req-live");
    evidence.request_context = vec![
        "probe_scope=okx.order-precheck".to_owned(),
        "live_place_remote_proof=missing".to_owned(),
        "live_cancel_remote_proof=missing".to_owned(),
        "does_not_grant_live_write=true".to_owned(),
    ];
    row.evidence = Some(evidence);

    let summary = runtime_evidence_summary(&row);
    let detail = runtime_evidence_detail(&row);

    assert!(summary.contains("live proof"));
    assert!(summary.contains("live_place_remote_proof=missing"));
    assert!(summary.contains("live_cancel_remote_proof=missing"));
    assert!(summary.contains("does_not_grant_live_write=true"));
    assert!(detail.contains("probe_scope=okx.order-precheck"));
}

#[test]
fn current_availability_requires_every_runtime_link_to_be_ok() {
    let operations = [
        "credential_probe:order_permission",
        "order_write",
        "private_ws_order_stream",
        "order_finality",
    ];
    let ready = selected_runtime_health_rows(
        VenueOperationHealthSnapshot::new(
            operations
                .iter()
                .map(|operation| operation_row("okx", operation, VenueOperationStatus::Ok))
                .collect(),
            10,
        ),
        "okx",
    );
    assert_eq!(
        trading_runtime_status_label(&ready.trading_evidence),
        "当前可用"
    );

    for (status, expected) in [
        (VenueOperationStatus::Unknown, "当前状态未验证"),
        (VenueOperationStatus::Warn, "当前状态降级"),
        (VenueOperationStatus::Blocked, "当前不可用"),
    ] {
        let mut rows = ready.rows.clone();
        rows[0].status = status;
        let selection =
            selected_runtime_health_rows(VenueOperationHealthSnapshot::new(rows, 10), "okx");
        assert_eq!(
            trading_runtime_status_label(&selection.trading_evidence),
            expected
        );
        assert_ne!(
            trading_runtime_status_label(&selection.trading_evidence),
            "当前可用"
        );
    }
}

#[test]
fn ok_but_unconfigured_runtime_link_is_not_currently_usable() {
    let mut rows = trading_runtime_rows_with_ok_status();
    rows[0].configured = Some(false);

    let selection = assert_selected_runtime_is_unavailable(rows);
    let row = &selection.rows[0];
    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.configured, Some(false));
    assert_eq!(row.supported, Some(true));
    assert!(!row.is_currently_usable());
}

#[test]
fn ok_but_unsupported_runtime_link_is_not_currently_usable() {
    let mut rows = trading_runtime_rows_with_ok_status();
    rows[0].supported = Some(false);

    let selection = assert_selected_runtime_is_unavailable(rows);
    let row = &selection.rows[0];
    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.configured, Some(true));
    assert_eq!(row.supported, Some(false));
    assert!(!row.is_currently_usable());
}

fn trading_runtime_rows_with_ok_status() -> Vec<VenueOperationHealth> {
    [
        "credential_probe:order_permission",
        "order_write",
        "private_ws_order_stream",
        "order_finality",
    ]
    .into_iter()
    .map(|operation| operation_row("okx", operation, VenueOperationStatus::Ok))
    .collect()
}

fn assert_selected_runtime_is_unavailable(
    rows: Vec<VenueOperationHealth>,
) -> RuntimeHealthSelection {
    let selection =
        selected_runtime_health_rows(VenueOperationHealthSnapshot::new(rows, 10), "okx");

    assert_eq!(selection.attention, 1);
    assert_eq!(trading_runtime_ready_count(&selection.trading_evidence), 3);
    assert_eq!(
        trading_runtime_status_label(&selection.trading_evidence),
        "当前不可用"
    );
    assert!(trading_runtime_summary("okx", &selection.trading_evidence).contains("3/4 正常"));
    selection
}
