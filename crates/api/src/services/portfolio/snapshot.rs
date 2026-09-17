use super::*;

pub(crate) async fn snapshot(state: &AppState) -> Result<PortfolioSnapshot, common::AppError> {
    let mut account_state = account_state::snapshot(state).await;
    let now_ms = common::time::now_ms();
    let positions = positions_outcome_from_account_state(state, &account_state, now_ms);
    merge_portfolio_field_quality(&mut account_state, &positions.field_quality);
    let balances = account_state.balances.rows.clone();
    let pnl = state
        .portfolio_pnl_snapshot()
        .value_now()
        .unwrap_or_default();
    let historical_pnl = pnl_history_values(&pnl.history);
    let nav = account_nav(&account_state, now_ms);
    let nav_yesterday = update_nav_history(state, nav.value, now_ms, nav_sample_plan(&nav)).await;
    let summary = summary_from_rows_with_nav(
        &positions.rows,
        &account_state,
        now_ms,
        nav,
        nav_yesterday,
        pnl.today,
    );
    let risk = risk_from_rows(
        state,
        &positions.rows,
        &account_state.balances.account_summaries,
        summary.total_nav_usd,
        now_ms,
        &historical_pnl,
    );
    let mut operation_health = account_state.operation_health.clone();
    operation_health.push(venue_operation_health::portfolio_nav_storage_health_row(
        state, now_ms,
    ));
    let degraded = account_state_degrades_portfolio_snapshot(&account_state)
        || summary.pnl_breakdown.evidence.quality == shared_types::ExecutionLedgerQuality::Missing
        || operation_health_degraded(&operation_health)
        || portfolio_field_quality_degraded(&account_state.field_quality);
    let rows = positions.rows;
    let snapshot_version = positions_version(&rows);
    let mut problems = positions.problems;
    if let Some(problem) = summary.nav_evidence.problem.as_ref() {
        push_runtime_problem(
            &mut problems,
            account_api_problem(problem, summary.nav_evidence.observed_at_ms),
        );
    }
    if let Some(problem) = summary.pnl_breakdown.evidence.problem.as_ref() {
        push_runtime_problem(
            &mut problems,
            account_api_problem(problem, summary.pnl_breakdown.evidence.observed_at_ms),
        );
    }
    let recent_close_runs = recent_close_runs(state);
    Ok(PortfolioSnapshot {
        summary,
        positions: rows,
        balances,
        risk,
        server_now_ms: now_ms,
        snapshot_version,
        degraded,
        problems,
        operation_health,
        account_state,
        recent_close_runs,
    })
}

pub(super) fn account_state_degrades_portfolio_snapshot(
    account_state: &AccountStateSnapshot,
) -> bool {
    account_state.balances.status == ListStatus::Degraded
        || account_state.positions.status == ListStatus::Degraded
        || account_state.open_orders.status == ListStatus::Degraded
        || account_state
            .problems
            .iter()
            .any(account_problem_degrades_portfolio_snapshot)
}

fn account_problem_degrades_portfolio_snapshot(problem: &ApiProblem) -> bool {
    !is_account_equity_advisory_problem(problem)
}

fn is_account_equity_advisory_problem(problem: &ApiProblem) -> bool {
    problem.code == shared_types::problem::codes::ACCOUNT_FIELD_UNKNOWN
        && problem
            .details
            .as_ref()
            .and_then(|details| details.get("field"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|field| field == "equity")
}

pub(super) fn portfolio_field_quality_degraded(rows: &[AccountFieldQuality]) -> bool {
    rows.iter().any(|row| {
        !is_account_equity_advisory_quality(row)
            && field_quality_degraded(std::slice::from_ref(row))
    })
}

fn is_account_equity_advisory_quality(row: &AccountFieldQuality) -> bool {
    row.field == "equity"
        && matches!(
            row.status,
            AccountFieldQualityStatus::Estimated
                | AccountFieldQualityStatus::Unknown
                | AccountFieldQualityStatus::Missing
        )
}

fn push_runtime_problem(rows: &mut Vec<RuntimeProblem>, problem: RuntimeProblem) {
    if !rows.contains(&problem) {
        rows.push(problem);
    }
}

pub(super) fn merge_portfolio_field_quality(
    account_state: &mut AccountStateSnapshot,
    rows: &[AccountFieldQuality],
) {
    if rows.is_empty() {
        return;
    }
    account_state.field_quality.extend_from_slice(rows);
    if field_quality_degraded(rows) {
        account_state.status = ListStatus::Degraded;
    }
}

pub(super) fn positions_outcome_from_account_state(
    state: &AppState,
    account_state: &AccountStateSnapshot,
    now_ms: i64,
) -> PositionsOutcome {
    let orders = state.trading_service().list_orders();
    let include_dry_run = state.trading_service().adapter_name() == "mock";
    let rates = funding_rows(state);
    let pair_evidence = execution_pair_evidence(state);
    let cfg = state.trading_service().risk_config();
    let dry_run_marks = dry_run_mark_prices(state, &orders, now_ms);
    let rows = rows_from_sources_with_dry_run_marks(
        account_state.positions.rows.clone(),
        &orders,
        &rates,
        &pair_evidence,
        DryRunRows {
            enabled: include_dry_run,
            marks: &dry_run_marks,
        },
        RiskAnnotation {
            now_ms,
            warn_pct: cfg.liquidation_warn_pct,
            danger_pct: cfg.liquidation_danger_pct,
        },
    );
    PositionsOutcome {
        field_quality: funding_field_quality(&rows, now_ms),
        rows,
        problems: account_state_runtime_problems(account_state, now_ms),
    }
}

pub(super) fn account_state_runtime_problems(
    account_state: &AccountStateSnapshot,
    observed_at_ms: i64,
) -> Vec<RuntimeProblem> {
    account_state
        .problems
        .iter()
        .map(|problem| account_api_problem(problem, observed_at_ms))
        .collect()
}

pub(crate) fn account_api_problem(problem: &ApiProblem, observed_at_ms: i64) -> RuntimeProblem {
    RuntimeProblem {
        scope: "portfolio".to_owned(),
        operation: problem_operation(problem),
        code: problem.code.clone(),
        message: problem.message.clone(),
        venue: problem_venue(problem),
        retry_after_ms: problem.retry_after_ms,
        problem: Some(problem.clone()),
        observed_at_ms,
    }
}

pub(super) fn problem_operation(problem: &ApiProblem) -> String {
    problem
        .details
        .as_ref()
        .and_then(|details| details.get("operation"))
        .and_then(|value| value.as_str())
        .unwrap_or("account_state")
        .to_owned()
}

pub(super) fn problem_venue(problem: &ApiProblem) -> Option<String> {
    problem
        .details
        .as_ref()
        .and_then(|details| details.get("venue"))
        .and_then(|value| value.as_str())
        .filter(|venue| !venue.is_empty())
        .map(ToOwned::to_owned)
}
