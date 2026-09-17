fn assert_http_row_core(row: &VenueOperationHealth) {
    assert_eq!(row.venue, "binance");
    assert_eq!(row.source, SOURCE_HTTP_OUTCOME_METRICS);
    assert_eq!(row.status, VenueOperationStatus::Blocked);
    assert_eq!(row.rows, Some(3));
    assert_eq!(row.freshness_ms, Some(1_000));
    assert_eq!(row.retry_after_ms, Some(4_000));
    assert_eq!(row.latency_ms, Some(8));
    assert_eq!(row.latency_p95_ms, Some(25));
    assert!(row.error.is_some());
}

fn assert_http_row_evidence(row: &VenueOperationHealth) {
    let evidence = row.evidence.as_ref().expect("endpoint evidence");
    assert_ne!(evidence.checked_at, UNRECORDED_EVIDENCE_MARKER);
    assert_ne!(evidence.doc_version, UNRECORDED_EVIDENCE_MARKER);
    assert_ne!(evidence.schema_hash, UNRECORDED_EVIDENCE_MARKER);
    assert_ne!(evidence.fixture_id, UNRECORDED_EVIDENCE_MARKER);
    assert_ne!(evidence.parser_test, UNRECORDED_EVIDENCE_MARKER);
    assert_ne!(evidence.request_builder_test, UNRECORDED_EVIDENCE_MARKER);
    assert_ne!(evidence.auth_kind, UNRECORDED_EVIDENCE_MARKER);
    assert_eq!(evidence.request_id.as_deref(), Some("req-rate_limited"));
    assert_eq!(
        evidence.request_context,
        vec!["symbol=BTCUSDT", "contract_code=BTC-USDT"]
    );
}

fn assert_private_ws_evidence(row: &VenueOperationHealth, doc_version_prefix: &str) {
    let evidence = row.evidence.as_ref().expect("private ws evidence");
    assert_private_ws_registry(evidence, doc_version_prefix);
    assert_private_ws_context(evidence);
}

fn assert_private_ws_registry(
    evidence: &shared_types::VenueOperationEvidence,
    doc_version_prefix: &str,
) {
    assert_eq!(evidence.method, "WS");
    assert!(evidence.path.contains("wss://"));
    assert!(evidence.path.contains('#'));
    assert_ne!(evidence.checked_at, UNRECORDED_EVIDENCE_MARKER);
    assert!(evidence.doc_version.starts_with(doc_version_prefix));
    assert!(evidence.schema_hash.starts_with("sha256:"));
    assert!(evidence.fixture_id.starts_with("crates/exchange/fixtures/"));
    assert_ne!(evidence.parser_test, UNRECORDED_EVIDENCE_MARKER);
    assert_ne!(evidence.request_builder_test, UNRECORDED_EVIDENCE_MARKER);
    assert_ne!(evidence.auth_kind, UNRECORDED_EVIDENCE_MARKER);
    assert!(evidence
        .doc_urls
        .iter()
        .any(|url| url.starts_with("https://")));
}

fn assert_private_ws_context(evidence: &shared_types::VenueOperationEvidence) {
    assert!(evidence
        .request_context
        .iter()
        .any(|item| item.starts_with("runtime_operation=private_ws_")));
    assert!(evidence
        .request_context
        .iter()
        .any(|item| item == "release_status=production_ready"));
    assert!(evidence
        .request_context
        .iter()
        .any(|item| item.starts_with("schema_hash=sha256:")));
    assert!(evidence
        .request_context
        .iter()
        .any(|item| item.starts_with("fixture_id=crates/exchange/fixtures/")));
}

#[test]
fn kraken_and_gate_crossex_private_ws_health_projects_registry_fixture() {
    let ws_venues = exchange::trading_ws_venues().venues;
    for venue in ["kraken", "gate_crossex"] {
        let evidence = private_ws_operation_evidence(
            venue,
            OP_PRIVATE_WS_ORDER_STREAM,
            Some("runtime-request"),
            &ws_venues,
        )
        .expect("private WS registry evidence");
        assert!(evidence.schema_hash.starts_with("sha256:"));
        assert!(evidence.fixture_id.starts_with("crates/exchange/fixtures/"));
        assert_eq!(evidence.request_id.as_deref(), Some("runtime-request"));
    }
}

fn assert_http_row_problem(row: &VenueOperationHealth) {
    let problem = row.problem.as_ref().expect("http outcome problem");
    let details = problem.details.as_ref().expect("http problem details");
    assert_eq!(problem.request_id.as_deref(), Some("req-rate_limited"));
    assert_eq!(
        details.get("operation").and_then(|value| value.as_str()),
        Some("http_rest:GET /fapi/v1/depth")
    );
    assert_eq!(
        details.get("path").and_then(|value| value.as_str()),
        Some("/fapi/v1/depth")
    );
    assert_eq!(
        details.get("symbol").and_then(|value| value.as_str()),
        Some("BTCUSDT")
    );
    assert_eq!(
        details
            .get("lastLatencyMs")
            .and_then(|value| value.as_u64()),
        Some(8)
    );
    assert_eq!(
        details.get("requestId").and_then(|value| value.as_str()),
        Some("req-rate_limited")
    );
    assert!(details
        .get("requestContext")
        .and_then(|context| context.as_array())
        .is_some_and(|context| context.len() == 2));
}

#[test]
fn task_registry_rows_report_healthy_aggregate() {
    let registry = crate::task_registry::TaskRegistry::default();
    registry.register("funding", 5_000);
    registry.register("portfolio", 5_000);

    let rows = task_registry_rows(&registry, common::time::now_ms());

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].venue, SYSTEM_VENUE);
    assert_eq!(rows[0].operation, OP_BACKGROUND_TASKS);
    assert_eq!(rows[0].source, SOURCE_TASK_REGISTRY);
    assert_eq!(rows[0].status, VenueOperationStatus::Ok);
    assert_eq!(rows[0].requested, Some(2));
    assert_eq!(rows[0].rows, Some(2));
    assert!(rows[0].problem.is_none());

    let funding = rows
        .iter()
        .find(|row| row.operation == "background_task:funding")
        .expect("funding task row");
    assert_eq!(funding.status, VenueOperationStatus::Ok);
    assert_eq!(funding.requested, Some(1));
    assert_eq!(funding.rows, Some(1));
    assert!(funding.error.is_none());
    assert!(funding.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|item| item == "task=funding")
    }));
}

#[test]
fn task_registry_rows_report_restart_count() {
    let registry = crate::task_registry::TaskRegistry::default();
    registry.register("funding", 5_000);
    registry.mark_exited("funding", "panicked: transient");
    registry.record_restart("funding");

    let rows = task_registry_rows(&registry, common::time::now_ms());
    let funding = rows
        .iter()
        .find(|row| row.operation == "background_task:funding")
        .expect("funding task row");

    assert_eq!(funding.status, VenueOperationStatus::Ok);
    assert!(funding.message.contains("restart 1"));
    assert!(funding.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|item| item == "restart_count=1")
    }));
}

#[test]
fn task_registry_rows_warn_on_slow_latest_duration() {
    let registry = crate::task_registry::TaskRegistry::default();
    registry.register("market_prewarm", 5_000);
    registry.record_result_timed("market_prewarm", common::time::now_ms() - 10_000, Ok(()));

    let rows = task_registry_rows(&registry, common::time::now_ms());

    let summary = rows
        .iter()
        .find(|row| row.operation == OP_BACKGROUND_TASKS)
        .expect("background summary row");
    assert_eq!(summary.status, VenueOperationStatus::Warn);
    assert_eq!(summary.rows, Some(0));
    assert!(summary.message.contains("慢迭代 1"));

    let task = rows
        .iter()
        .find(|row| row.operation == "background_task:market_prewarm")
        .expect("slow task row");
    assert_eq!(task.status, VenueOperationStatus::Warn);
    assert_eq!(task.rows, Some(0));
    assert!(task
        .latency_ms
        .is_some_and(|duration_ms| duration_ms >= 10_000));
    assert!(task.problem.is_none());
    assert!(task.error.as_deref().is_some_and(|error| {
        error.contains("slow_threshold 7500ms") && error.contains("slow_tick 1")
    }));
    let evidence = task.evidence.as_ref().expect("task evidence");
    assert!(evidence
        .request_context
        .iter()
        .any(|item| item == "slow_threshold_ms=7500"));
    assert!(evidence
        .request_context
        .iter()
        .any(|item| item == "slow_tick_count=1"));
    assert!(evidence
        .request_context
        .iter()
        .any(|item| item.starts_with("last_duration_ms=")));
}

#[test]
fn task_registry_rows_project_unhealthy_tasks() {
    let registry = crate::task_registry::TaskRegistry::default();
    registry.register("funding", 5_000);
    registry.register("snapshot", 5_000);
    registry.mark_exited("snapshot", "panicked: boom");
    for _ in 0..3 {
        registry.record_failure("funding", "no rates");
    }

    let rows = task_registry_rows(&registry, common::time::now_ms());

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].operation, OP_BACKGROUND_TASKS);
    assert_eq!(rows[0].status, VenueOperationStatus::Blocked);
    assert_eq!(rows[0].requested, Some(2));
    assert_eq!(rows[0].rows, Some(0));

    let funding = rows
        .iter()
        .find(|row| row.operation == "background_task:funding")
        .expect("funding task row");
    assert_eq!(funding.status, VenueOperationStatus::Blocked);
    assert!(funding
        .error
        .as_deref()
        .is_some_and(|error| error.contains("no rates")));
    assert_eq!(funding.rows, Some(0));
    let problem = funding.problem.as_ref().expect("task problem");
    assert_eq!(problem.code, "TASK_FAILING");
    assert_eq!(problem.source.as_deref(), Some(SOURCE_TASK_REGISTRY));

    let snapshot = rows
        .iter()
        .find(|row| row.operation == "background_task:snapshot")
        .expect("snapshot task row");
    assert_eq!(snapshot.status, VenueOperationStatus::Blocked);
    assert!(snapshot
        .problem
        .as_ref()
        .is_some_and(|problem| problem.code == "TASK_DOWN"));
}
