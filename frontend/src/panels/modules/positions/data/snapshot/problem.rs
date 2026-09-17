use shared_types::{ApiProblem, PortfolioSnapshot, RuntimeProblem};

pub(super) fn raw_snapshot_degraded_problem(snapshot: &PortfolioSnapshot) -> ApiProblem {
    snapshot
        .summary
        .nav_evidence
        .problem
        .clone()
        .or_else(|| snapshot.account_state.problems.first().cloned())
        .or_else(|| snapshot.problems.first().map(runtime_problem_to_api))
        .unwrap_or_else(|| {
            ApiProblem::new(
                shared_types::problem::codes::PORTFOLIO_SNAPSHOT_DEGRADED,
                "portfolio snapshot contains partial or unverified account data",
            )
            .with_source("portfolio_ws_snapshot")
        })
}

fn runtime_problem_to_api(problem: &RuntimeProblem) -> ApiProblem {
    problem.to_api_problem()
}
