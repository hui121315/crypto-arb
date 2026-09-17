use super::super::*;

#[test]
fn unknown_account_equity_is_advisory_without_masking_account_failures() {
    let mut account_state = AccountStateSnapshot {
        status: ListStatus::Degraded,
        ..AccountStateSnapshot::default()
    };
    account_state.balances.status = ListStatus::Fresh;
    account_state.positions.status = ListStatus::Fresh;
    account_state.open_orders.status = ListStatus::Fresh;
    let mut equity_problem = ApiProblem::new(
        shared_types::problem::codes::ACCOUNT_FIELD_UNKNOWN,
        "account equity is unknown",
    );
    equity_problem.details = Some(serde_json::json!({"field": "equity"}));
    account_state.problems.push(equity_problem);
    account_state.field_quality.push(AccountFieldQuality::new(
        AccountFieldSubject::account("hyperliquid:xyz"),
        "equity",
        AccountFieldQualityStatus::Unknown,
        "account_state_runtime",
        Some(1),
    ));

    assert!(!account_state_degrades_portfolio_snapshot(&account_state));
    assert!(!portfolio_field_quality_degraded(
        &account_state.field_quality
    ));

    account_state.problems.push(ApiProblem::new(
        shared_types::problem::codes::UPSTREAM_PARSE,
        "positions response could not be parsed",
    ));
    assert!(account_state_degrades_portfolio_snapshot(&account_state));

    account_state.problems.clear();
    account_state.field_quality[0].status = AccountFieldQualityStatus::Invalid;
    assert!(portfolio_field_quality_degraded(
        &account_state.field_quality
    ));
}
