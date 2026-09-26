use super::*;

#[test]
fn problem_title_contains_operation_and_message() {
    let title = problem_title(&[problem("gate", "positions")], &[], None);

    assert!(title.contains("gate · portfolio/positions"));
    assert!(title.contains("parse failed"));
}

#[test]
fn more_label_only_counts_hidden_rows() {
    assert_eq!(more_label(3, 3), "");
    assert_eq!(more_label(5, 3), "还有 2 条");
    assert_eq!(more_label(4, 4), "");
}

#[test]
fn title_includes_api_problem_request_id() {
    let title = problem_title(&[], &[], Some(&api_problem()));

    assert!(title.contains("request_id req-1"));
    assert!(title.contains("retry 2000ms"));
}

#[test]
fn runtime_problem_title_preserves_typed_request_context() {
    let mut runtime = problem("gate", "positions");
    let mut typed = api_problem().with_source("gate.GET /api/v4/futures/usdt/positions");
    typed.details = Some(serde_json::json!({
        "method": "GET",
        "path": "/api/v4/futures/usdt/positions",
    }));
    runtime.problem = Some(typed);

    let title = problem_title(&[runtime], &[], None);

    assert!(title.contains("request_id req-1"));
    assert!(title.contains("source gate.GET /api/v4/futures/usdt/positions"));
    assert!(title.contains("\"path\":\"/api/v4/futures/usdt/positions\""));
}

#[test]
fn title_includes_nav_storage_health() {
    let title = problem_title(&[], &[health_row()], None);

    assert!(title.contains("system · storage:portfolio_nav · WARN"));
    assert!(title.contains("账户净值 存储未配置"));
}

#[test]
fn ok_operation_health_is_not_attention() {
    let mut row = health_row();
    row.status = VenueOperationStatus::Ok;

    assert!(attention_health(vec![row]).is_empty());
}

#[test]
fn unknown_operation_health_keeps_typed_problem_drilldown() {
    let mut row = health_row();
    row.status = VenueOperationStatus::Unknown;
    row.problem = Some(
        ApiProblem::new("ACCOUNT_SCOPE_UNVERIFIED", "scope is unknown")
            .with_status(424)
            .with_request_id(Some("req-scope".into()))
            .with_source("account_binding"),
    );

    let attention = attention_health(vec![row]);
    let title = problem_title(&[], &attention, None);

    assert_eq!(attention.len(), 1);
    assert!(title.contains("UNKNOWN"));
    assert!(title.contains("ACCOUNT_SCOPE_UNVERIFIED"));
    assert!(title.contains("request_id req-scope"));
    assert!(title.contains("source account_binding"));
}

#[test]
fn idle_bitget_private_order_stream_is_not_presented_as_data_degradation() {
    let mut row = health_row();
    row.venue = "bitget".into();
    row.operation = OP_PRIVATE_WS_ORDER_STREAM.into();
    row.source = "private_ws_runtime".into();
    row.status = VenueOperationStatus::Unknown;
    row.configured = Some(true);
    row.rows = Some(0);
    row.message = "私有 WS 订阅已发送，等待订单流事件样本".into();
    row.error = None;

    assert!(attention_health(vec![row]).is_empty());
}

#[test]
fn idle_binance_private_streams_are_not_presented_as_data_degradation() {
    let mut account = health_row();
    account.venue = "binance".into();
    account.operation = OP_PRIVATE_WS_ACCOUNT_STREAM.into();
    account.source = "private_ws_runtime".into();
    account.status = VenueOperationStatus::Unknown;
    account.configured = Some(true);
    account.rows = Some(0);
    account.message = "私有 WS 订阅已发送，等待账户流事件样本".into();
    account.error = None;

    let mut order = account.clone();
    order.operation = OP_PRIVATE_WS_ORDER_STREAM.into();
    order.message = "私有 WS 订阅已发送，等待订单流事件样本".into();

    assert!(attention_health(vec![account, order]).is_empty());
}

#[test]
fn failed_private_order_stream_remains_visible() {
    let mut row = health_row();
    row.venue = "bitget".into();
    row.operation = OP_PRIVATE_WS_ORDER_STREAM.into();
    row.source = "private_ws_runtime".into();
    row.status = VenueOperationStatus::Unknown;
    row.configured = Some(true);
    row.rows = Some(0);
    row.message = "私有 WS 订阅已发送，等待订单流事件样本".into();
    row.error = Some("subscription state lost".into());

    assert_eq!(attention_health(vec![row]).len(), 1);
}

#[test]
fn failed_private_account_stream_remains_visible() {
    let mut row = health_row();
    row.venue = "binance".into();
    row.operation = OP_PRIVATE_WS_ACCOUNT_STREAM.into();
    row.source = "private_ws_runtime".into();
    row.status = VenueOperationStatus::Unknown;
    row.configured = Some(true);
    row.rows = Some(0);
    row.message = "私有 WS 订阅已发送，等待账户流事件样本".into();
    row.problem = Some(ApiProblem::new(
        "PRIVATE_WS_AUTH_FAILED",
        "listen key rejected",
    ));

    assert_eq!(attention_health(vec![row]).len(), 1);
}

#[test]
fn idle_confirmed_private_streams_are_not_presented_as_data_degradation() {
    for venue in ["gate", "kucoin", "okx"] {
        let mut row = health_row();
        row.venue = venue.into();
        row.operation = OP_PRIVATE_WS_ORDER_STREAM.into();
        row.source = "private_ws_runtime".into();
        row.status = VenueOperationStatus::Unknown;
        row.configured = Some(true);
        row.rows = Some(0);
        row.message = "私有 WS 订阅已发送，等待订单流事件样本".into();
        row.error = None;

        assert!(attention_health(vec![row]).is_empty(), "{venue}");
    }
}

fn problem(venue: &str, operation: &str) -> RuntimeProblem {
    RuntimeProblem {
        scope: "portfolio".into(),
        operation: operation.into(),
        code: "UPSTREAM".into(),
        message: "parse failed".into(),
        venue: Some(venue.into()),
        retry_after_ms: None,
        problem: None,
        observed_at_ms: 1,
    }
}

fn api_problem() -> ApiProblem {
    ApiProblem::new("UPSTREAM", "gateway failed")
        .with_status(502)
        .with_request_id(Some("req-1".into()))
        .with_retry_after_ms(Some(2_000))
}

fn health_row() -> VenueOperationHealth {
    VenueOperationHealth {
        venue: "system".into(),
        operation: "storage:portfolio_nav".into(),
        status: VenueOperationStatus::Warn,
        source: "portfolio_nav_store".into(),
        message: "账户净值 存储未配置".into(),
        supported: Some(true),
        configured: Some(false),
        requested: Some(0),
        rows: Some(0),
        freshness_ms: None,
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: Some("账户净值 存储未配置".into()),
        evidence: None,
        problem: None,
        observed_at_ms: 1,
    }
}
