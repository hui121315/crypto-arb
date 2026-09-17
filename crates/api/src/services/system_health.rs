use crate::services::{
    market_data::MarketQuality, portfolio, portfolio_snapshot_envelope, runtime_state,
    venue_operation_health,
};
use crate::state::AppState;
use shared_types::{
    problem::codes, ApiHealthSlot, ApiProblem, ListStatus, LiveOrderState, NextFundingSlot,
    OrderRecord, ResourceEnvelope, ResourceStatus, RiskStatusSlot, RuntimeProblem, SystemHealth,
    SystemHealthEnvelope, VenueOperationHealth, VenueOperationHealthSnapshot, VenueOperationKind,
    VenueOperationStatus, WsHealthSlot,
};
use std::collections::BTreeSet;

mod problems;
mod slots;
#[cfg(test)]
mod tests;

use problems::*;
use slots::*;

const SYSTEM_HEALTH_SOURCE: &str = "system-health-snapshot";
const SYSTEM_HEALTH_REFRESH_MS: u64 = 5_000;

struct PortfolioHealthFacts {
    account_degraded: bool,
    risk: RiskStatusSlot,
    net_delta_usd: f64,
    net_delta_pct_of_nav: f64,
    next_funding: Option<NextFundingSlot>,
    problems: Vec<RuntimeProblem>,
}

pub(crate) async fn snapshot(state: &AppState) -> SystemHealth {
    let operation_health = venue_operation_health::snapshot(state);
    let now_ms = common::time::now_ms();
    let portfolio = portfolio_health_facts(state, now_ms);
    let live_operations_required = state.trading_service().risk_config().live_trading_enabled;
    let runtime_state = runtime_state::inventory(state).await;
    let mut problems = portfolio.problems;
    problems.extend(task_issue_problems(state.task_registry(), now_ms));
    problems.extend(market_data_problems(state, now_ms));
    problems.extend(operation_health_problems(
        &operation_health,
        live_operations_required,
    ));
    problems.extend(runtime_state::problems(&runtime_state, now_ms));
    let api = api_health_or_missing(
        &operation_health,
        &mut problems,
        now_ms,
        live_operations_required,
    );
    let ws = ws_health_or_missing(
        &operation_health,
        &mut problems,
        now_ms,
        live_operations_required,
    );
    let degraded = portfolio.account_degraded
        || !problems.is_empty()
        || api.healthy < api.total
        || !ws.disconnected.is_empty();

    SystemHealth {
        api_version: env!("CARGO_PKG_VERSION").to_owned(),
        api,
        ws,
        order_elapsed_ms: order_elapsed_ms(&state.trading_service().list_orders()),
        risk: portfolio.risk,
        net_delta_usd: portfolio.net_delta_usd,
        net_delta_pct_of_nav: portfolio.net_delta_pct_of_nav,
        next_funding: portfolio.next_funding,
        updated_at_ms: now_ms,
        degraded,
        problems,
    }
}

fn portfolio_health_facts(state: &AppState, now_ms: i64) -> PortfolioHealthFacts {
    if let Some(entry) = state.portfolio_snapshot().get_arc_now() {
        return portfolio_health_facts_from_snapshot(&entry.value, now_ms);
    }
    missing_portfolio_health(now_ms)
}

fn missing_portfolio_health(now_ms: i64) -> PortfolioHealthFacts {
    let problem = ApiProblem::new(
        codes::PORTFOLIO_SNAPSHOT_UNAVAILABLE,
        "portfolio lifecycle has not published account and risk evidence",
    )
    .with_source(portfolio_snapshot_envelope::SOURCE_LIFECYCLE)
    .with_retry_after_ms(Some(2_000));
    PortfolioHealthFacts {
        account_degraded: true,
        risk: RiskStatusSlot::Block,
        net_delta_usd: 0.0,
        net_delta_pct_of_nav: 0.0,
        next_funding: None,
        problems: vec![RuntimeProblem {
            scope: "portfolio".to_owned(),
            operation: "snapshot".to_owned(),
            code: codes::PORTFOLIO_SNAPSHOT_UNAVAILABLE.to_owned(),
            message: problem.message.clone(),
            venue: None,
            retry_after_ms: problem.retry_after_ms,
            problem: Some(problem),
            observed_at_ms: now_ms,
        }],
    }
}

fn portfolio_health_facts_from_snapshot(
    snapshot: &shared_types::PortfolioSnapshot,
    now_ms: i64,
) -> PortfolioHealthFacts {
    let mut problems = snapshot
        .account_state
        .problems
        .iter()
        .map(|problem| portfolio::account_api_problem(problem, now_ms))
        .collect();
    append_nav_problem(&mut problems, &snapshot.summary);
    PortfolioHealthFacts {
        account_degraded: snapshot.account_state.status == ListStatus::Degraded,
        risk: risk_status(&snapshot.summary, &snapshot.risk),
        net_delta_usd: snapshot.summary.net_delta_usd,
        net_delta_pct_of_nav: snapshot.summary.net_delta_pct_of_nav,
        next_funding: next_funding_slot(&snapshot.positions, now_ms),
        problems,
    }
}

fn append_nav_problem(
    problems: &mut Vec<RuntimeProblem>,
    summary: &shared_types::PortfolioSummary,
) {
    if let Some(problem) = summary.nav_evidence.problem.as_ref() {
        problems.push(portfolio::account_api_problem(
            problem,
            summary.nav_evidence.observed_at_ms,
        ));
    }
}

pub(crate) fn envelope(health: SystemHealth) -> SystemHealthEnvelope {
    let status = if health.degraded {
        ResourceStatus::Degraded
    } else {
        ResourceStatus::Ready
    };
    let problems = health.problems.iter().map(runtime_problem).collect();
    let observed_at_ms = health.updated_at_ms;
    ResourceEnvelope::with_data(
        health,
        status,
        SYSTEM_HEALTH_SOURCE,
        observed_at_ms,
        problems,
    )
}

pub(crate) fn warming_envelope(now_ms: i64) -> SystemHealthEnvelope {
    let problem = ApiProblem::new(
        codes::SYSTEM_HEALTH_SNAPSHOT_WARMING,
        "system health lifecycle has not published its first snapshot",
    )
    .with_source(SYSTEM_HEALTH_SOURCE)
    .with_retry_after_ms(Some(SYSTEM_HEALTH_REFRESH_MS));
    ResourceEnvelope::unavailable(
        ResourceStatus::Warming,
        SYSTEM_HEALTH_SOURCE,
        now_ms,
        vec![problem],
    )
}

fn runtime_problem(problem: &RuntimeProblem) -> ApiProblem {
    problem.to_api_problem()
}
