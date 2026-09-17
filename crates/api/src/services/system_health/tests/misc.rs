use super::*;

#[test]
fn api_failed_operation_row_surfaces_runtime_problem() {
    let mut row = operation_health(
        "okx",
        "credential_probe:order_permission",
        VenueOperationStatus::Unknown,
    );
    row.retry_after_ms = Some(5_000);

    let problems =
        operation_health_problems(&VenueOperationHealthSnapshot::new(vec![row], 10), true);

    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].scope, "trading_api");
    assert_eq!(problems[0].operation, "credential_probe:order_permission");
    assert_eq!(problems[0].code, "VENUE_OPERATION_UNKNOWN");
    assert_eq!(problems[0].venue.as_deref(), Some("okx"));
    assert_eq!(problems[0].retry_after_ms, Some(5_000));
}

#[test]
fn api_transport_operation_rows_surface_runtime_problems_without_trading_scope() {
    let mut row = operation_health(
        "gate",
        "http_rest:GET /api/v4/orders",
        VenueOperationStatus::Warn,
    );
    row.retry_after_ms = Some(4_000);
    row.problem = Some(
        shared_types::ApiProblem::new("EXCHANGE_HTTP_DEGRADED", "Gate order query degraded")
            .with_retry_after_ms(Some(3_000)),
    );

    let problems =
        operation_health_problems(&VenueOperationHealthSnapshot::new(vec![row], 10), true);

    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].scope, "api_transport");
    assert_eq!(problems[0].operation, "http_rest:GET /api/v4/orders");
    assert_eq!(problems[0].code, "EXCHANGE_HTTP_DEGRADED");
    assert_eq!(problems[0].message, "Gate order query degraded");
    assert_eq!(problems[0].venue.as_deref(), Some("gate"));
    assert_eq!(problems[0].retry_after_ms, Some(3_000));
}

#[test]
fn ws_failed_operation_row_surfaces_typed_runtime_problem() {
    let mut row = operation_health(
        "binance",
        "private_ws_order_stream",
        VenueOperationStatus::Blocked,
    );
    row.problem = Some(shared_types::ApiProblem::new(
        "PRIVATE_WS_AUTH_FAILED",
        "order stream auth failed",
    ));

    let problems =
        operation_health_problems(&VenueOperationHealthSnapshot::new(vec![row], 10), true);

    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].scope, "private_ws");
    assert_eq!(problems[0].operation, "private_ws_order_stream");
    assert_eq!(problems[0].code, "PRIVATE_WS_AUTH_FAILED");
    assert_eq!(problems[0].message, "order stream auth failed");
    assert_eq!(problems[0].venue.as_deref(), Some("binance"));
}

#[test]
fn api_and_ws_health_ignore_background_task_operation_rows() {
    let snapshot = VenueOperationHealthSnapshot::new(
        vec![
            operation_health("system", "background_tasks", VenueOperationStatus::Ok),
            operation_health(
                "system",
                "background_task:funding",
                VenueOperationStatus::Blocked,
            ),
            operation_health("system", "storage:history", VenueOperationStatus::Warn),
            operation_health(
                "system",
                "storage:portfolio_nav",
                VenueOperationStatus::Blocked,
            ),
        ],
        1_000,
    );

    assert!(api_health_from_operations(&snapshot, true).is_none());
    assert!(ws_health_from_operations(&snapshot, true).is_none());
}

#[test]
fn down_task_surfaces_as_runtime_problem() {
    let registry = crate::task_registry::TaskRegistry::default();
    registry.register("snapshot", 5_000);
    registry.mark_exited("snapshot", "panicked: boom");

    let problems = task_issue_problems(&registry, common::time::now_ms());

    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].scope, "background_task");
    assert_eq!(problems[0].operation, "snapshot");
    assert_eq!(problems[0].code, "TASK_DOWN");
    assert!(problems[0].message.contains("panicked: boom"));
    assert!(problems[0].venue.is_none());
}

#[test]
fn running_tasks_yield_no_problems() {
    let registry = crate::task_registry::TaskRegistry::default();
    registry.register("snapshot", 5_000);

    assert!(task_issue_problems(&registry, common::time::now_ms()).is_empty());
}

#[test]
fn gate_parse_failure_surfaces_as_runtime_problem() {
    let error = common::AppError::upstream(
        shared_types::problem::codes::UPSTREAM_PARSE,
        "gate json: error decoding response body",
    );
    let problem =
        crate::services::runtime_problem::from_app_error("portfolio", "positions", &error, &[]);
    assert_eq!(problem.venue.as_deref(), Some("gate"));
    assert_eq!(problem.code, "UPSTREAM_PARSE");
}

#[test]
fn order_elapsed_ignores_non_terminal_orders() {
    let created = order(LiveOrderState::Created, 1_000, 3_000);
    let accepted = order(LiveOrderState::Accepted, 1_000, 1_120);
    let partial = order(LiveOrderState::PartiallyFilled, 1_000, 1_240);

    assert_eq!(order_elapsed_ms(&[created, accepted, partial]), None);
}

#[test]
fn order_elapsed_averages_terminal_samples() {
    let accepted = order(LiveOrderState::Accepted, 1_000, 1_120);
    let filled = order(LiveOrderState::Filled, 1_000, 1_120);
    let rejected = order(LiveOrderState::Rejected, 1_000, 1_200);

    assert_eq!(order_elapsed_ms(&[accepted, filled, rejected]), Some(160));
}

#[test]
fn next_funding_ignores_expired_timestamp() {
    let mut rows = vec![position(1_000)];
    rows.push(position(120_000));

    let next = next_funding_slot(&rows, 60_000);

    assert_eq!(next.map(|slot| slot.minutes_to_settle), Some(1));
}

#[test]
fn next_funding_uses_row_funding_rate() {
    let mut rows = vec![position(120_000)];
    rows[0].funding_rate_8h = 0.001;

    let outflow = next_funding_slot(&rows, 60_000).map(|slot| slot.estimated_outflow_usd);

    assert_eq!(outflow, Some(0.1));
}

#[test]
fn next_funding_ignores_unverified_funding_rate() {
    let mut rows = vec![position(120_000)];
    rows[0].funding_rate_8h = 0.001;
    rows[0].funding_rate_verified = false;

    let next = next_funding_slot(&rows, 60_000);

    assert!(next.is_none());
}

#[test]
fn risk_status_is_driven_by_portfolio_exposure_not_operational_health() {
    let summary = shared_types::PortfolioSummary {
        total_nav_usd: 0.0,
        nav_evidence: Default::default(),
        nav_change_24h_pct: Some(0.0),
        net_delta_usd: 0.0,
        net_delta_pct_of_nav: 0.0,
        naked_exposure_usd: 0.0,
        naked_position_count: 0,
        realized_pnl_today_usd: 0.0,
        pnl_breakdown: Default::default(),
        updated_at_ms: 1,
    };
    let risk = shared_types::RiskSnapshot {
        var_99_1d_usd: 0.0,
        var_pct_of_nav: 0.0,
        var_sample_size: 0,
        funding_clustering: Vec::new(),
        delta_concentration: Vec::new(),
        margin_utilization: Vec::new(),
        hard_limits: Default::default(),
        updated_at_ms: 1,
    };

    assert_eq!(risk_status(&summary, &risk), RiskStatusSlot::Ok);
}

#[test]
fn paper_mode_omits_unconfigured_private_operation_problems() {
    let mut row = operation_health("okx", "private_read", VenueOperationStatus::Blocked);
    row.configured = Some(false);

    let problems =
        operation_health_problems(&VenueOperationHealthSnapshot::new(vec![row], 10), false);

    assert!(problems.is_empty());
}
