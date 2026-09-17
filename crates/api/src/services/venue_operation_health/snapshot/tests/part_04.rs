#[test]
fn order_write_runtime_overlay_replaces_credential_config_row() {
    let credential = credential(true, true);
    let mut rows = credential_rows(std::slice::from_ref(&credential), 10);
    let snapshot = live_order_proof_health("binance", VenueOperationStatus::Ok);

    overlay_order_write_runtime_rows(
        &mut rows,
        order_write_runtime_rows(std::slice::from_ref(&credential), vec![snapshot], 20),
    );
    let order_rows = rows
        .iter()
        .filter(|row| row.operation == OP_ORDER_WRITE)
        .collect::<Vec<_>>();

    assert_eq!(order_rows.len(), 1);
    assert_eq!(order_rows[0].source, SOURCE_LIVE_ORDER_PROOF_RUNTIME);
    assert_eq!(order_rows[0].status, VenueOperationStatus::Ok);
    assert_eq!(order_rows[0].supported, Some(true));
    assert_eq!(order_rows[0].configured, Some(true));
    assert!(order_rows[0].evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|item| item == "live_place_remote_proof=ok")
    }));
}

#[test]
fn order_write_credentials_do_not_become_live_proof() {
    let credential = credential(true, true);
    let rows = order_write_runtime_rows(std::slice::from_ref(&credential), Vec::new(), 10);
    let row = &rows[0];
    let evidence = row.evidence.as_ref().expect("missing proof evidence");

    assert_eq!(row.operation, OP_ORDER_WRITE);
    assert_eq!(row.status, VenueOperationStatus::Unknown);
    assert_eq!(row.source, SOURCE_LIVE_ORDER_PROOF_RUNTIME);
    assert_eq!(row.rows, Some(0));
    assert!(row.problem.is_none());
    assert!(
        evidence
            .request_context
            .iter()
            .any(|item| item == "live_place_remote_proof=missing")
    );
    assert!(
        evidence
            .request_context
            .iter()
            .any(|item| item == "live_cancel_remote_proof=missing")
    );
}

#[test]
fn order_write_runtime_preserves_unsupported_unconfigured_static_gate() {
    let unsupported = credential(false, true);
    let unconfigured = credential(true, false);
    let rows = order_write_runtime_rows(
        &[unsupported, unconfigured],
        vec![live_order_proof_health("binance", VenueOperationStatus::Ok)],
        10,
    );

    assert_eq!(rows[0].status, VenueOperationStatus::Unsupported);
    assert_eq!(rows[0].source, SOURCE_CREDENTIAL_CONFIG);
    assert_eq!(rows[1].status, VenueOperationStatus::Blocked);
    assert_eq!(rows[1].source, SOURCE_CREDENTIAL_CONFIG);
}

#[test]
fn order_write_context_keys_are_searchable() {
    let credential = credential(true, true);
    let mut snapshot = live_order_proof_health("binance", VenueOperationStatus::Warn);
    snapshot.cancel_finality = None;
    snapshot.rows = Some(1);
    snapshot.message = "已有 live 下单 ack 样本，仍缺撤单请求与撤单终态远程证明".to_owned();
    snapshot.error = Some(snapshot.message.clone());

    let rows = order_write_runtime_rows(std::slice::from_ref(&credential), vec![snapshot], 20);
    let row = &rows[0];
    let evidence = row.evidence.as_ref().expect("order write evidence");

    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.requested, Some(1));
    assert_eq!(row.rows, Some(1));
    assert!(row.problem.is_none());
    assert!(row.message.contains("不阻断下单能力"));
    assert!(
        evidence
            .request_context
            .iter()
            .any(|item| item == "probe_source=live_order_proof_runtime")
    );
    assert!(
        evidence
            .request_context
            .iter()
            .any(|item| item == "live_place_remote_proof=ok")
    );
    assert!(
        evidence
            .request_context
            .iter()
            .any(|item| item == "live_cancel_remote_proof=missing")
    );
    assert!(
        evidence
            .request_context
            .iter()
            .any(|item| item == "sample_place_internal_order_id=internal-1")
    );
    assert!(
        evidence
            .request_context
            .iter()
            .any(|item| item == "sample_place_native_transport=hyperliquid_ws_post")
    );
    assert!(
        evidence
            .request_context
            .iter()
            .any(|item| item == "sample_place_native_request_id=257")
    );
}

#[test]
fn newer_submit_failure_still_blocks_order_write() {
    let credential = credential(true, true);
    let mut snapshot = live_order_proof_health("binance", VenueOperationStatus::Blocked);
    snapshot.last_problem = Some(
        crate::services::live_order_proof_health::LiveOrderProofProblem {
            message: "order rejected".to_owned(),
            source: "submit_order".to_owned(),
            request_id: Some("req-rejected".to_owned()),
            retry_after_ms: Some(1_000),
            status: Some(403),
            observed_at_ms: 13,
        },
    );

    let rows = order_write_runtime_rows(std::slice::from_ref(&credential), vec![snapshot], 20);
    let row = &rows[0];
    let problem = row.problem.as_ref().expect("submit failure problem");

    assert_eq!(row.status, VenueOperationStatus::Blocked);
    assert_eq!(row.requested, Some(1));
    assert_eq!(row.rows, Some(1));
    assert_eq!(row.retry_after_ms, Some(1_000));
    assert_eq!(problem.code, codes::HEDGE_PRE_TRADE_REJECTED);
    assert_eq!(problem.status, Some(403));
    assert!(row.message.contains("order rejected"));
}

#[test]
fn run_finality_failure_maps_to_problem_and_evidence() {
    let mut snapshot = run_finality_health("binance", VenueOperationStatus::Blocked);
    snapshot.message =
        "订单终态回查完成：待确认 2，刷新 0，远端缺失 0，已终态 0，刷新失败 1，发布失败 0"
            .to_owned();
    snapshot.error = Some(snapshot.message.clone());
    snapshot.requested = Some(2);
    snapshot.rows = Some(0);
    snapshot.refresh_failure_count = 1;
    snapshot.sample_problem = Some(run_finality_sample_problem(
        "exchange-1",
        "internal-1",
        "订单终态回查失败: upstream timeout",
        Some("upstream timeout"),
    ));

    let rows = run_finality_runtime_rows(&[credential(true, true)], vec![snapshot], 20);
    let row = rows
        .iter()
        .find(|row| row.operation == OP_ORDER_FINALITY)
        .expect("order finality row");
    let problem = row.problem.as_ref().expect("order finality problem");
    let evidence = row.evidence.as_ref().expect("order finality evidence");

    assert_eq!(row.status, VenueOperationStatus::Blocked);
    assert_eq!(row.source, SOURCE_RUN_FINALITY_RUNTIME);
    assert_eq!(row.requested, Some(2));
    assert_eq!(row.rows, Some(0));
    assert_eq!(problem.code, codes::HEDGE_ORDER_FINALITY_FAILED);
    assert_eq!(problem.status, Some(502));
    assert_eq!(problem.source.as_deref(), Some(SOURCE_RUN_FINALITY_RUNTIME));
    assert_eq!(evidence.path, "run_finality.refresh_pending_runs");
    assert_eq!(evidence.auth_kind, "internal_order_query");
    assert!(
        evidence
            .use_cases
            .iter()
            .any(|item| item == "order_finality")
    );
    assert!(
        evidence
            .request_context
            .iter()
            .any(|item| item == "refresh_failure_count=1")
    );
    assert!(
        evidence
            .request_context
            .iter()
            .any(|item| item == "sample_raw_order_id=exchange-1")
    );
    assert!(
        evidence
            .request_context
            .iter()
            .any(|item| item == "sample_error=upstream timeout")
    );
    assert!(
        problem
            .details
            .as_ref()
            .and_then(|details| details.get("sampleProblem"))
            .and_then(|sample| sample.get("internalOrderId"))
            .is_some_and(|order_id| order_id.as_str() == Some("internal-1"))
    );
}

#[test]
fn run_finality_remote_missing_warns_with_conflict_problem() {
    let mut snapshot = run_finality_health("binance", VenueOperationStatus::Warn);
    snapshot.message =
        "订单终态回查完成：待确认 1，刷新 0，远端缺失 1，已终态 0，刷新失败 0，发布失败 0"
            .to_owned();
    snapshot.error = Some(snapshot.message.clone());
    snapshot.requested = Some(1);
    snapshot.rows = Some(0);
    snapshot.remote_missing_count = 1;

    let rows = run_finality_runtime_rows(&[credential(true, true)], vec![snapshot], 20);
    let problem = rows[0].problem.as_ref().expect("remote missing problem");

    assert_eq!(rows[0].status, VenueOperationStatus::Warn);
    assert_eq!(problem.status, Some(409));
    assert!(
        problem
            .details
            .as_ref()
            .and_then(|details| details.get("remoteMissingCount"))
            .is_some_and(|count| count.as_u64() == Some(1))
    );
}

#[test]
fn http_outcome_rows_use_latest_endpoint_outcome() {
    let rows = http_outcome_rows(
        vec![
            http_outcome("success", None, 10_000, 12),
            http_outcome("rate_limited", Some(5_000), 20_000, 3),
        ],
        21_000,
    );

    assert_eq!(rows.len(), 1);
    assert_http_row_core(&rows[0]);
    assert_http_row_evidence(&rows[0]);
    assert_http_row_problem(&rows[0]);
}

#[test]
fn http_outcome_slow_success_warns_and_names_slow_endpoint() {
    let mut snapshot = http_outcome("success", None, 20_000, 5);
    snapshot.latency_p95_ms = Some(5_000);

    let row = http_outcome_row(snapshot, 21_000);

    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert!(row.message.contains("慢 endpoint"));
    assert!(row.message.contains("/fapi/v1/depth"));
    assert!(row.error.is_some());
}

#[test]
fn http_outcome_fast_success_stays_ok() {
    let row = http_outcome_row(http_outcome("success", None, 20_000, 5), 21_000);

    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert!(!row.message.contains("慢 endpoint"));
}

#[test]
fn http_outcome_without_endpoint_evidence_exposes_not_recorded_evidence() {
    let mut snapshot = http_outcome("timeout", None, 20_000, 1);
    snapshot.endpoint_evidence = None;

    let row = http_outcome_row(snapshot, 21_000);
    let evidence = row.evidence.as_ref().expect("fallback evidence");

    assert_eq!(evidence.method, "GET");
    assert_eq!(evidence.path, "/fapi/v1/depth");
    assert_eq!(evidence.checked_at, UNRECORDED_EVIDENCE_MARKER);
    assert_eq!(evidence.schema_hash, UNRECORDED_EVIDENCE_MARKER);
    assert_eq!(evidence.fixture_id, UNRECORDED_EVIDENCE_MARKER);
    assert_eq!(evidence.request_id.as_deref(), Some("req-timeout"));
    assert!(
        evidence
            .request_context
            .iter()
            .any(|item| item == "endpoint_evidence=not_recorded")
    );
}
